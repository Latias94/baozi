use crate::config::MAX_DETECTOR_WORK_BETWEEN_POLLS;
use crate::descriptor::{TextDecodingMode, TextProfile};
use crate::error::{ScopeAbort, ScopeAbortCause};
use crate::format::DetectionOutcome;
use crate::ledger::ImportDimension;
use crate::store::{
    ImportBoundWorkGroup, ImportObservedWorkFailure, ImportStoreExecution, ImportStoreHostError,
};
use crate::text_input::TextConformanceResult;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

/// Result transport for one cooperative detector invocation.
pub type DetectionResult<T = DetectionOutcome> = Result<T, DetectionAbort>;

/// Opaque detector transport for a session-owned terminal failure.
///
/// ```compile_fail
/// use baozi_import::DetectionAbort;
///
/// fn require_send<T: Send>() {}
/// require_send::<DetectionAbort>();
/// ```
///
/// ```compile_fail
/// use baozi_import::DetectionAbort;
///
/// fn require_sync<T: Sync>() {}
/// require_sync::<DetectionAbort>();
/// ```
#[derive(Debug)]
pub struct DetectionAbort {
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl DetectionAbort {
    const fn new() -> Self {
        Self {
            not_send_or_sync: PhantomData,
        }
    }
}

impl fmt::Display for DetectionAbort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("detection scope aborted")
    }
}

impl std::error::Error for DetectionAbort {}

/// Capability-limited cancellation and work accounting for one detector invocation.
///
/// Detector work is callback-scoped; the former escaping permit API is intentionally absent.
///
/// ```compile_fail
/// use baozi_import::DetectionScope;
///
/// fn escaping_work_is_not_available(scope: &DetectionScope<'_>) {
///     let _permit = scope.begin_work(1);
/// }
/// ```
pub struct DetectionScope<'session> {
    exec: RefCell<ImportStoreExecution<'session>>,
    work_group: ImportBoundWorkGroup<'session, 1>,
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    callback_active: Cell<bool>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl fmt::Debug for DetectionScope<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DetectionScope")
            .finish_non_exhaustive()
    }
}

impl<'session> DetectionScope<'session> {
    pub(crate) fn new(
        exec: ImportStoreExecution<'session>,
        terminal: &'session RefCell<Option<ScopeAbortCause>>,
    ) -> Result<Self, ImportStoreHostError> {
        let work_group = exec.bind_group([ImportDimension::DetectionWork])?;
        Ok(Self {
            exec: RefCell::new(exec),
            work_group,
            terminal,
            callback_active: Cell::new(false),
            not_send_or_sync: PhantomData,
        })
    }

    /// Polls cancellation and deadline state, latching the first terminal cause.
    pub fn poll(&self) -> DetectionResult<()> {
        if self.callback_active.get() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detection-poll-during-work",
            }));
        }
        self.poll_active()
    }

    /// Runs one bounded detector work chunk after polling operation control.
    ///
    /// The work capability is confined to this call: success records the complete chunk and polls
    /// control once more, while an error preserves attempted work without fabricating completion.
    /// A panic clears the callback-active marker before it continues into the registry's existing
    /// containment boundary.
    pub fn with_work<T>(
        &self,
        amount: u64,
        action: impl FnOnce() -> DetectionResult<T>,
    ) -> DetectionResult<T> {
        self.preflight_active()?;
        if self.callback_active.get() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detection-work-already-in-progress",
            }));
        }
        if amount == 0 {
            self.poll_active()?;
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detector-work-zero-amount",
            }));
        }
        if amount > MAX_DETECTOR_WORK_BETWEEN_POLLS {
            self.poll_active()?;
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detector-work-quantum-exceeded",
            }));
        }

        self.poll_active()?;
        let mut work = self
            .work_group
            .admit_observed([amount])
            .map_err(|cause| self.abort_store(cause))?;
        // Only the real permit owns numeric work; this flag enforces callback reentry rules.
        let callback = DetectionCallbackGuard::begin(&self.callback_active);
        let result = match action() {
            Ok(value) => match work.complete([amount]) {
                Ok(()) => work
                    .finish()
                    .map(|()| value)
                    .map_err(ImportObservedWorkFailure::Store),
                Err(cause) => {
                    let _ = work.finish_partial();
                    Err(ImportObservedWorkFailure::Action(self.abort_store(cause)))
                }
            },
            Err(cause) => {
                let _ = work.finish_partial();
                Err(ImportObservedWorkFailure::Action(cause))
            }
        };
        callback.finish();

        match result {
            Ok(value) => {
                self.poll_active()?;
                Ok(value)
            }
            Err(ImportObservedWorkFailure::Action(cause)) => {
                let _ = self.poll_active();
                Err(cause)
            }
            Err(ImportObservedWorkFailure::Store(cause)) => Err(self.abort_store(cause)),
        }
    }

    /// Checks a bounded probe against a text profile without allocating decoded text or line state.
    ///
    /// Formal parsing must still use `ParseExecution::with_conformed_text_input`; this method exists so
    /// detectors can share the same BOM, UTF-8, newline, lexical, and control classification.
    pub fn check_text_input_conformance(
        &self,
        profile: &TextProfile,
        bytes: &[u8],
        decoding_mode: TextDecodingMode,
    ) -> DetectionResult<TextConformanceResult<()>> {
        if bytes.is_empty() {
            return Ok(crate::text_input::check_text_input_conformance(
                profile,
                bytes,
                decoding_mode,
            ));
        }

        let work = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.with_work(work, || {
            Ok(crate::text_input::check_text_input_conformance(
                profile,
                bytes,
                decoding_mode,
            ))
        })
    }

    pub(crate) fn into_terminal_abort(self) -> Option<ScopeAbort> {
        if self.callback_active.replace(false) {
            let _ = self.abort(ScopeAbortCause::DescriptorContract {
                code: "detection-work-callback-unresolved",
            });
        }
        if let Err(cause) = self.exec.borrow().finish_region() {
            let _ = self.abort_store(cause);
        }
        self.terminal.borrow_mut().take().map(ScopeAbort::new)
    }

    fn poll_active(&self) -> DetectionResult<()> {
        self.preflight_active()?;
        self.exec
            .borrow_mut()
            .poll()
            .map_err(|cause| self.abort_store(cause))
    }

    fn abort(&self, cause: ScopeAbortCause) -> DetectionAbort {
        let mut terminal = self.terminal.borrow_mut();
        if terminal.is_none() {
            *terminal = Some(cause);
        }
        DetectionAbort::new()
    }

    fn preflight_active(&self) -> DetectionResult<()> {
        if self.terminal.borrow().is_some() {
            Err(DetectionAbort::new())
        } else {
            Ok(())
        }
    }

    fn abort_store(&self, cause: ImportStoreHostError) -> DetectionAbort {
        self.abort(cause.into_scope_abort_cause())
    }
}

struct DetectionCallbackGuard<'scope> {
    active: &'scope Cell<bool>,
    finished: bool,
}

impl<'scope> DetectionCallbackGuard<'scope> {
    fn begin(active: &'scope Cell<bool>) -> Self {
        let already_active = active.replace(true);
        debug_assert!(!already_active);
        Self {
            active,
            finished: false,
        }
    }

    fn finish(mut self) {
        self.active.set(false);
        self.finished = true;
    }
}

impl Drop for DetectionCallbackGuard<'_> {
    fn drop(&mut self) {
        if !self.finished && self.active.replace(false) {
            debug_assert!(
                std::thread::panicking(),
                "the private detector callback guard may only be abandoned by unwinding"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportLedgerPhase, ImportOperationControl};
    use crate::limits::ResourceLimits;
    use crate::metered_storage::AccountedVec;
    use crate::store::{
        ImportStoreBootstrapRequirements, ImportStoreSessionOwner, try_import_store_bootstrap,
    };
    use baozi_operation::{
        CancellationToken, ControlCause, PhysicalStorageAccountFactory,
        PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
    };
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn store(control: ImportOperationControl) -> ImportStoreSessionOwner {
        let limits = ResourceLimits::default();
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap();
        let account = factory
            .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
            .unwrap();
        let requirements = ImportStoreBootstrapRequirements::for_node_scene(&limits).unwrap();
        let bootstrap = try_import_store_bootstrap(&limits, requirements).unwrap();
        ImportStoreSessionOwner::try_start(control, &account, bootstrap).unwrap()
    }

    #[test]
    fn detector_callback_can_allocate_and_error_preserves_only_completed_chunks() {
        let mut owner = store(ImportOperationControl::default());
        let terminal = RefCell::new(None);
        let scope = DetectionScope::new(owner.execution().unwrap(), &terminal).unwrap();
        let bytes = scope
            .with_work(3, || {
                let mut exec = scope
                    .exec
                    .try_borrow_mut()
                    .expect("the detector action must not retain an execution borrow");
                let state = exec
                    .dimension_state(ImportDimension::DetectionWork)
                    .unwrap();
                assert_eq!(state.attempted(), 3);
                assert_eq!(state.consumed(), 0);
                let mut bytes = AccountedVec::new();
                bytes
                    .try_reserve(8, "detector-callback-bytes", |capacity| {
                        exec.reserve(
                            ImportLedgerPhase::Detection,
                            ImportDimension::ScratchBytes,
                            capacity,
                        )
                        .map_err(ImportStoreHostError::into_scope_abort_cause)
                    })
                    .unwrap();
                bytes.try_extend_from_slice_reserved(b"detected").unwrap();
                Ok(bytes)
            })
            .unwrap();
        assert_eq!(&*bytes, b"detected");
        scope.with_work(2, || Ok(())).unwrap();
        assert!(
            scope
                .with_work(5, || Err::<(), _>(scope.abort(
                    ScopeAbortCause::ResourceLimit {
                        resource: "detector-action",
                        limit: 5
                    },
                )))
                .is_err()
        );
        assert!(matches!(
            scope.into_terminal_abort().unwrap().cause(),
            ScopeAbortCause::ResourceLimit {
                resource: "detector-action",
                limit: 5
            }
        ));
        drop(bytes);
        let exec = owner.execution().unwrap();
        let state = exec
            .dimension_state(ImportDimension::DetectionWork)
            .unwrap();
        assert_eq!(state.attempted(), 10);
        assert_eq!(state.consumed(), 5);
        exec.finish_region().unwrap();
    }

    #[test]
    fn detector_reentry_is_rejected_before_nested_work_or_control() {
        for nested_work in [false, true] {
            let mut owner = store(ImportOperationControl::default());
            let terminal = RefCell::new(None);
            let scope = DetectionScope::new(owner.execution().unwrap(), &terminal).unwrap();
            let nested_called = Cell::new(false);
            assert!(
                scope
                    .with_work(1, || {
                        let result = if nested_work {
                            scope.with_work(1, || {
                                nested_called.set(true);
                                Ok(())
                            })
                        } else {
                            scope.poll()
                        };
                        assert!(result.is_err());
                        Ok(())
                    })
                    .is_err()
            );
            assert!(!nested_called.get());
            let abort = scope.into_terminal_abort().unwrap();
            let expected = if nested_work {
                "detection-work-already-in-progress"
            } else {
                "detection-poll-during-work"
            };
            assert!(
                matches!(abort.cause(), ScopeAbortCause::DescriptorContract { code }
                if *code == expected)
            );
            let exec = owner.execution().unwrap();
            let state = exec
                .dimension_state(ImportDimension::DetectionWork)
                .unwrap();
            assert_eq!(state.attempted(), 1);
            assert_eq!(state.consumed(), 1);
            exec.finish_region().unwrap();
        }
    }

    #[test]
    fn detector_quantum_rejection_precedes_the_callback_and_attempted_work() {
        for (amount, expected) in [
            (0, "detector-work-zero-amount"),
            (
                MAX_DETECTOR_WORK_BETWEEN_POLLS + 1,
                "detector-work-quantum-exceeded",
            ),
        ] {
            let mut owner = store(ImportOperationControl::default());
            let terminal = RefCell::new(None);
            let scope = DetectionScope::new(owner.execution().unwrap(), &terminal).unwrap();
            let called = Cell::new(false);
            assert!(
                scope
                    .with_work(amount, || {
                        called.set(true);
                        Ok(())
                    })
                    .is_err()
            );
            assert!(!called.get());
            let abort = scope.into_terminal_abort().unwrap();
            assert!(
                matches!(abort.cause(), ScopeAbortCause::DescriptorContract { code }
                if *code == expected)
            );
            let exec = owner.execution().unwrap();
            let state = exec
                .dimension_state(ImportDimension::DetectionWork)
                .unwrap();
            assert_eq!(state.attempted(), 0);
            assert_eq!(state.consumed(), 0);
            exec.finish_region().unwrap();
        }
    }

    #[test]
    fn detector_final_poll_observes_cancellation_after_the_completed_action() {
        let token = CancellationToken::new();
        let mut owner = store(ImportOperationControl::default().with_cancellation(token.clone()));
        let terminal = RefCell::new(None);
        let scope = DetectionScope::new(owner.execution().unwrap(), &terminal).unwrap();
        assert!(
            scope
                .with_work(2, || {
                    token.cancel();
                    Ok(())
                })
                .is_err()
        );
        let state = scope
            .exec
            .borrow()
            .dimension_state(ImportDimension::DetectionWork)
            .unwrap();
        assert_eq!(state.attempted(), 2);
        assert_eq!(state.consumed(), 2);
        assert!(matches!(
            scope.into_terminal_abort().unwrap().cause(),
            ScopeAbortCause::Control(ControlCause::Cancelled { .. })
        ));
    }

    #[test]
    fn detector_panic_releases_the_callback_and_actual_work_owner() {
        let mut owner = store(ImportOperationControl::default());
        let terminal = RefCell::new(None);
        let scope = DetectionScope::new(owner.execution().unwrap(), &terminal).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                scope.with_work::<()>(2, || panic!("detector action panic"))
            }))
            .is_err()
        );
        assert!(!scope.callback_active.get());
        assert!(scope.exec.try_borrow_mut().is_ok());
        assert!(scope.poll().is_err());
        let state = scope
            .exec
            .borrow()
            .dimension_state(ImportDimension::DetectionWork)
            .unwrap();
        assert_eq!(state.attempted(), 2);
        assert_eq!(state.consumed(), 0);
        scope.exec.borrow().finish_region().unwrap();
        assert!(scope.into_terminal_abort().is_some());
    }
}
