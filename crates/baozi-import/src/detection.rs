use crate::config::MAX_DETECTOR_WORK_BETWEEN_POLLS;
use crate::error::{ScopeAbort, ScopeAbortCause};
use crate::format::DetectionOutcome;
use crate::ledger::{ImportCoordinator, ImportDimension};
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
    coordinator: &'session RefCell<ImportCoordinator>,
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
        coordinator: &'session RefCell<ImportCoordinator>,
        terminal: &'session RefCell<Option<ScopeAbortCause>>,
    ) -> Self {
        Self {
            coordinator,
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
        let mut work = self.begin_work(amount)?;
        match action() {
            Ok(value) => {
                work.complete()?;
                Ok(value)
            }
            Err(cause) => {
                work.finish_partial();
                let _ = self.poll_active();
                Err(cause)
            }
        }
    }

    fn begin_work(&self, amount: u64) -> DetectionResult<DetectionWorkGuard<'_, 'session>> {
        self.preflight_active()?;
        if self.work_outstanding.get() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detection-work-already-in-progress",
            }));
        }
        self.poll_active()?;
        if amount == 0 {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detector-work-zero-amount",
            }));
        }
        if amount > MAX_DETECTOR_WORK_BETWEEN_POLLS {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "detector-work-quantum-exceeded",
            }));
        }

        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_attempted(ImportDimension::DetectionWork, amount)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;

        self.work_outstanding.set(true);
        Ok(DetectionWorkGuard {
            scope: self,
            amount,
            finished: false,
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
        let result = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            coordinator.poll()
        };
        result.map_err(|cause| self.abort(ScopeAbortCause::Control(cause)))
    }

    fn commit_work(&self, amount: u64) -> DetectionResult<()> {
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_consumed(ImportDimension::DetectionWork, amount)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
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

    fn reentrant_capability_failure(&self) -> DetectionAbort {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "detection-capability-reentered",
        })
    }
}

struct DetectionWorkGuard<'scope, 'session> {
    scope: &'scope DetectionScope<'session>,
    amount: u64,
    finished: bool,
}

impl DetectionWorkGuard<'_, '_> {
    fn complete(&mut self) -> DetectionResult<()> {
        let committed = self.scope.commit_work(self.amount);
        self.scope.work_outstanding.set(false);
        self.finished = true;
        committed?;
        self.scope.poll_active()
    }

    fn finish_partial(&mut self) {
        self.scope.work_outstanding.set(false);
        self.finished = true;
    }
}

impl Drop for DetectionWorkGuard<'_, '_> {
    fn drop(&mut self) {
        if !self.finished && self.scope.work_outstanding.replace(false) {
            debug_assert!(
                std::thread::panicking(),
                "the private detector work guard may only be abandoned by unwinding"
            );
        }
    }
}
