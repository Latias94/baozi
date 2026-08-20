use crate::config::MAX_DETECTOR_WORK_BETWEEN_POLLS;
use crate::descriptor::{TextDecodingMode, TextProfile};
use crate::error::{ScopeAbort, ScopeAbortCause};
use crate::format::DetectionOutcome;
use crate::ledger::ImportDimension;
use crate::store::{ImportObservedWorkFailure, ImportStoreHost, ImportStoreHostError};
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
    host: ImportStoreHost<'session>,
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    work_outstanding: Cell<bool>,
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
    pub(crate) const fn new(
        host: ImportStoreHost<'session>,
        terminal: &'session RefCell<Option<ScopeAbortCause>>,
    ) -> Self {
        Self {
            host,
            terminal,
            work_outstanding: Cell::new(false),
            not_send_or_sync: PhantomData,
        }
    }

    /// Polls cancellation and deadline state, latching the first terminal cause.
    pub fn poll(&self) -> DetectionResult<()> {
        if self.work_outstanding.get() {
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
        if self.work_outstanding.get() {
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

        let callback = DetectionCallbackGuard::begin(&self.work_outstanding);
        let result =
            self.host
                .with_observed_work(
                    [(ImportDimension::DetectionWork, amount)],
                    |work| match action() {
                        Ok(value) => {
                            work.complete([amount])
                                .map_err(|cause| self.abort_store(cause))?;
                            Ok(value)
                        }
                        Err(cause) => Err(cause),
                    },
                );
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
    /// Formal parsing must still use `ParseScope::with_conformed_text_input`; this method exists so
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
        if self.work_outstanding.replace(false) {
            let _ = self.abort(ScopeAbortCause::DescriptorContract {
                code: "detection-work-callback-unresolved",
            });
        }
        self.terminal.borrow_mut().take().map(ScopeAbort::new)
    }

    fn poll_active(&self) -> DetectionResult<()> {
        self.preflight_active()?;
        self.host.poll().map_err(|cause| self.abort_store(cause))
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
    outstanding: &'scope Cell<bool>,
    finished: bool,
}

impl<'scope> DetectionCallbackGuard<'scope> {
    fn begin(outstanding: &'scope Cell<bool>) -> Self {
        debug_assert!(!outstanding.replace(true));
        Self {
            outstanding,
            finished: false,
        }
    }

    fn finish(mut self) {
        self.outstanding.set(false);
        self.finished = true;
    }
}

impl Drop for DetectionCallbackGuard<'_> {
    fn drop(&mut self) {
        if !self.finished && self.outstanding.replace(false) {
            debug_assert!(
                std::thread::panicking(),
                "the private detector callback guard may only be abandoned by unwinding"
            );
        }
    }
}
