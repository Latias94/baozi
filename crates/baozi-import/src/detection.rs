use crate::config::MAX_DETECTOR_WORK_BETWEEN_POLLS;
use crate::descriptor::{TextDecodingMode, TextProfile};
use crate::error::{ScopeAbort, ScopeAbortCause};
use crate::format::DetectionOutcome;
use crate::ledger::ImportDimension;
use crate::store::{ImportRootExecution, ImportRootWorkLayout, ImportStoreHostError};
use crate::text_input::TextConformanceResult;
use std::cell::RefCell;
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
/// Sequential work and explicit polls borrow the same scope:
///
/// ```
/// use baozi_import::{DetectionResult, DetectionScope};
/// fn sequential(scope: &mut DetectionScope<'_, '_>) -> DetectionResult<()> {
///     scope.with_work(1, || Ok(()))?;
///     scope.poll()?;
///     scope.with_work(1, || Ok(()))
/// }
/// ```
///
/// A callback cannot poll its scope while its work is admitted:
///
/// ```compile_fail
/// use baozi_import::{DetectionResult, DetectionScope};
/// fn reenter_poll(scope: &mut DetectionScope<'_, '_>) -> DetectionResult<()> {
///     scope.with_work(1, || scope.poll())
/// }
/// ```
///
/// Nor can it admit nested work on that scope:
///
/// ```compile_fail
/// use baozi_import::{DetectionResult, DetectionScope};
/// fn reenter_work(scope: &mut DetectionScope<'_, '_>) -> DetectionResult<()> {
///     scope.with_work(1, || scope.with_work(1, || Ok(())))
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::DetectionScope;
///
/// fn escaping_work_is_not_available(scope: &mut DetectionScope<'_, '_>) {
///     let _permit = scope.begin_work(1);
/// }
/// ```
pub struct DetectionScope<'loan, 'op> {
    exec: ImportRootExecution<'loan, 'op>,
    work_group: ImportRootWorkLayout<'op, 1>,
    terminal: &'loan RefCell<Option<ScopeAbortCause>>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl fmt::Debug for DetectionScope<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DetectionScope")
            .finish_non_exhaustive()
    }
}

impl<'loan, 'op> DetectionScope<'loan, 'op> {
    pub(crate) fn new(
        exec: &'loan mut ImportRootExecution<'_, 'op>,
        terminal: &'loan RefCell<Option<ScopeAbortCause>>,
    ) -> Result<Self, ImportStoreHostError> {
        let work_group = exec.bind_group([ImportDimension::DetectionWork])?;
        Ok(Self {
            exec: exec.reborrow(),
            work_group,
            terminal,
            not_send_or_sync: PhantomData,
        })
    }

    /// Polls cancellation and deadline state, latching the first terminal cause.
    pub fn poll(&mut self) -> DetectionResult<()> {
        self.poll_active()
    }

    /// Runs one bounded detector work chunk after polling operation control.
    ///
    /// The work capability is confined to this call: success records the complete chunk and polls
    /// control once more, while an error preserves attempted work without fabricating completion.
    /// The mutable loan prevents a callback from polling or admitting more work on this scope.
    /// A panic resolves the host-owned numeric frame before reaching registry containment.
    pub fn with_work<T>(
        &mut self,
        amount: u64,
        action: impl FnOnce() -> DetectionResult<T>,
    ) -> DetectionResult<T> {
        self.preflight_active()?;
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

        let result = self
            .exec
            .run_fallible_action(&self.work_group, [amount], action)
            .map_err(|cause| self.abort_store(cause))?;

        match result {
            Ok(value) => {
                self.poll_active()?;
                Ok(value)
            }
            Err(cause) => {
                let _ = self.poll_active();
                Err(cause)
            }
        }
    }

    /// Checks a bounded probe against a text profile without allocating decoded text or line state.
    ///
    /// Formal parsing must still use `ParseRoot::with_conformed_text_input`; this method exists so
    /// detectors can share the same BOM, UTF-8, newline, lexical, and control classification.
    pub fn check_text_input_conformance(
        &mut self,
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
        // Every work callback is closed by its real action host; the enclosing root checks
        // quiescence while retaining the sole numeric and cleanup authority.
        self.terminal.borrow_mut().take().map(ScopeAbort::new)
    }

    fn poll_active(&mut self) -> DetectionResult<()> {
        self.preflight_active()?;
        self.exec.poll().map_err(|cause| self.abort_store(cause))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportLedgerPhase, ImportOperationControl, import_ledger_limits};
    use crate::limits::ResourceLimits;
    use crate::metered_storage::AccountedVec;
    use crate::store::{
        ImportStoreBodyParts, ImportStoreBootstrapRequirements, ImportStoreInvocationRoot,
        ImportStoreSessionOwner, try_import_store_bootstrap,
    };
    use baozi_core::__scene_integration::integration::SceneParseOutcome;
    use baozi_operation::{
        CancellationToken, ControlCause, PhysicalStorageAccountFactory,
        PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
    };
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn store<'inv>(
        parts: ImportStoreBodyParts<'inv>,
        control: ImportOperationControl,
    ) -> ImportStoreSessionOwner<'inv> {
        let limits = ResourceLimits::default();
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap();
        let account = factory
            .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
            .unwrap();
        let import_limits = import_ledger_limits(&limits).expect("test import limits are valid");
        let requirements =
            ImportStoreBootstrapRequirements::for_node_scene(&import_limits).unwrap();
        let bootstrap = try_import_store_bootstrap(import_limits, requirements).unwrap();
        ImportStoreSessionOwner::try_start_in_body(parts, control, &account, bootstrap).unwrap()
    }

    #[test]
    fn detector_callback_fills_admitted_storage_and_error_preserves_only_completed_chunks() {
        let mut invocation_root = ImportStoreInvocationRoot::new();
        let finish = invocation_root
            .run_body(|parts, disposition| {
                let mut owner = store(parts, ImportOperationControl::default());
                let terminal = RefCell::new(None);
                owner
                    .with_root_execution(|exec| -> Result<(), ImportStoreHostError> {
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
                        let mut scope = DetectionScope::new(exec, &terminal)?;
                        let bytes = scope
                            .with_work(3, || {
                                bytes.try_extend_from_slice_reserved(b"detected").unwrap();
                                Ok(bytes)
                            })
                            .unwrap();
                        assert_eq!(&*bytes, b"detected");
                        let state = scope.exec.dimension_state(ImportDimension::DetectionWork)?;
                        assert_eq!(state.attempted(), 3);
                        assert_eq!(state.consumed(), 3);
                        scope.with_work(2, || Ok(())).unwrap();
                        assert!(
                            scope
                                .with_work(5, || {
                                    *terminal.borrow_mut() = Some(ScopeAbortCause::ResourceLimit {
                                        resource: "detector-action",
                                        limit: 5,
                                    });
                                    Err::<(), _>(DetectionAbort::new())
                                })
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
                        let state = exec.dimension_state(ImportDimension::DetectionWork)?;
                        assert_eq!(state.attempted(), 10);
                        assert_eq!(state.consumed(), 5);
                        Ok(())
                    })
                    .unwrap();
                owner
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap();
            })
            .unwrap();
        let _finish = finish;
    }

    // Reentry through the same scope is statically rejected by the paired public examples.

    #[test]
    fn detector_quantum_rejection_precedes_the_callback_and_attempted_work() {
        for (amount, expected) in [
            (0, "detector-work-zero-amount"),
            (
                MAX_DETECTOR_WORK_BETWEEN_POLLS + 1,
                "detector-work-quantum-exceeded",
            ),
        ] {
            let mut invocation_root = ImportStoreInvocationRoot::new();
            let finish = invocation_root
                .run_body(|parts, disposition| {
                    let mut owner = store(parts, ImportOperationControl::default());
                    let terminal = RefCell::new(None);
                    owner
                        .with_root_execution(|exec| -> Result<(), ImportStoreHostError> {
                            let mut scope = DetectionScope::new(exec, &terminal)?;
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
                            let state = exec.dimension_state(ImportDimension::DetectionWork)?;
                            assert_eq!(state.attempted(), 0);
                            assert_eq!(state.consumed(), 0);
                            Ok(())
                        })
                        .unwrap();
                    owner
                        .request_finish(disposition, SceneParseOutcome::Rejected)
                        .unwrap();
                })
                .unwrap();
            let _finish = finish;
        }
    }

    #[test]
    fn detector_final_poll_observes_cancellation_after_the_completed_action() {
        let token = CancellationToken::new();
        let mut invocation_root = ImportStoreInvocationRoot::new();
        let finish = invocation_root
            .run_body(|parts, disposition| {
                let mut owner = store(
                    parts,
                    ImportOperationControl::default().with_cancellation(token.clone()),
                );
                let terminal = RefCell::new(None);
                owner
                    .with_root_execution(|exec| -> Result<(), ImportStoreHostError> {
                        let mut scope = DetectionScope::new(exec, &terminal)?;
                        assert!(
                            scope
                                .with_work(2, || {
                                    token.cancel();
                                    Ok(())
                                })
                                .is_err()
                        );
                        let state = scope.exec.dimension_state(ImportDimension::DetectionWork)?;
                        assert_eq!(state.attempted(), 2);
                        assert_eq!(state.consumed(), 2);
                        assert!(matches!(
                            scope.into_terminal_abort().unwrap().cause(),
                            ScopeAbortCause::Control(ControlCause::Cancelled { .. })
                        ));
                        Ok(())
                    })
                    .unwrap();
                owner
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap();
            })
            .unwrap();
        let _finish = finish;
    }

    #[test]
    fn detector_panic_releases_the_callback_and_actual_work_owner() {
        let mut invocation_root = ImportStoreInvocationRoot::new();
        let finish = invocation_root
            .run_body(|parts, disposition| {
                let mut owner = store(parts, ImportOperationControl::default());
                let terminal = RefCell::new(None);
                owner
                    .with_root_execution(|exec| -> Result<(), ImportStoreHostError> {
                        let mut scope = DetectionScope::new(exec, &terminal)?;
                        assert!(
                            catch_unwind(AssertUnwindSafe(|| {
                                scope.with_work::<()>(2, || panic!("detector action panic"))
                            }))
                            .is_err()
                        );
                        assert!(scope.poll().is_err());
                        let state = scope.exec.dimension_state(ImportDimension::DetectionWork)?;
                        assert_eq!(state.attempted(), 2);
                        assert_eq!(state.consumed(), 0);
                        assert!(scope.into_terminal_abort().is_some());
                        Ok(())
                    })
                    .unwrap();
                owner
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap();
            })
            .unwrap();
        let _finish = finish;
    }
}
