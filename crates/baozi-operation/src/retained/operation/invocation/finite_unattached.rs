//! Consuming finite unattached close under the original operation-start admission.

use super::{FiniteOperationInvocation, InvocationSlot, UnattachedTerminalStamp};
use crate::ledger::LedgerCause;
use crate::ledger::finite::FiniteLedgerSnapshot;
use crate::retained::operation::record_terminal_cause_with_precedence;
use crate::retained::startup;
use crate::retained::{AccountPoisonReason, PreparationFailureCode};
use crate::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
use std::fmt;

#[derive(Debug)]
pub(crate) struct FiniteUnattachedTerminal<P, D> {
    pub(crate) snapshot: FiniteLedgerSnapshot<P, D>,
    pub(crate) cause: Option<PreparationFailureCode>,
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::retained) enum FiniteUnattachedFailureCause<D> {
    OwnershipUnverified,
    Ledger(LedgerCause<D>),
}

#[must_use = "the failure retains the actual finite invocation until containment"]
pub(crate) struct FiniteUnattachedFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    cause: FiniteUnattachedFailureCause<D>,
    residue: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
}

type FiniteUnattachedResult<O, P, PP, U, D> =
    Result<FiniteUnattachedTerminal<P, D>, FiniteUnattachedFailure<O, P, PP, U, D>>;

impl<O, P, PP, U, D> FiniteUnattachedFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(in crate::retained) fn cause(&self) -> &FiniteUnattachedFailureCause<D> {
        &self.cause
    }

    pub(in crate::retained) fn terminal_cause(&self) -> Option<PreparationFailureCode> {
        self.residue
            .as_ref()
            .and_then(|common| common.lifecycle.sticky_terminal_cause.get())
    }

    pub(in crate::retained) fn residue(
        &self,
    ) -> Option<&FiniteOperationInvocation<O, P, PP, U, D>> {
        self.residue.as_ref()
    }
}

impl<O, P, PP, U, D> Drop for FiniteUnattachedFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if let Some(common) = self.residue.take() {
            std::mem::forget(common);
        }
    }
}

/// Owns the actual state before preparation and keeps it armed through fixed capture.
struct FiniteUnattachedGuard<'account, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    common: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    admission: startup::PendingOperationStartAdmission<'account>,
}

impl<O, P, PP, U, D> FiniteOperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    #[expect(
        clippy::result_large_err,
        reason = "structural failure owns the original finite invocation without allocation"
    )]
    pub(in crate::retained) fn prepare_and_finish_unattached(
        self,
        admission: startup::PendingOperationStartAdmission<'_>,
        final_progress: FinalProgress<PP, U>,
    ) -> FiniteUnattachedResult<O, P, PP, U, D> {
        let mut guard = FiniteUnattachedGuard {
            common: Some(self),
            admission,
        };
        match guard.prepare(final_progress) {
            Ok(stamp) => guard.finish(stamp),
            Err(()) => Err(guard.reject(FiniteUnattachedFailureCause::OwnershipUnverified)),
        }
    }
}

impl<O, P, PP, U, D> FiniteUnattachedGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn prepare(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<UnattachedTerminalStamp, ()> {
        let account = self.admission.account();
        InvocationSlot::Finite(&mut self.common)
            .into_view()
            .prepare_unattached_terminal(account, final_progress)
    }

    #[expect(
        clippy::result_large_err,
        reason = "a rejected stamp or dirty capture transfers the actual finite owner"
    )]
    fn finish(mut self, stamp: UnattachedTerminalStamp) -> FiniteUnattachedResult<O, P, PP, U, D> {
        let common = self
            .common
            .as_mut()
            .expect("finite terminal capture retains its original invocation");
        let identity = common.coordinator.retained_ledger().as_read().identity();
        if !common
            .lifecycle
            .try_consume_unattached_stamp(stamp, identity)
        {
            return Err(self.reject(FiniteUnattachedFailureCause::OwnershipUnverified));
        }
        let (snapshot, ledger_cause) = common
            .coordinator
            .retained_ledger_mut()
            .take_terminal_snapshot(0);
        if let Some(cause) = ledger_cause {
            return Err(self.reject(FiniteUnattachedFailureCause::Ledger(cause)));
        }
        let cause = common.lifecycle.sticky_terminal_cause.get();
        common.lifecycle.terminal_outstanding = false;
        let common = self
            .common
            .take()
            .expect("clean finite close consumes its original invocation once");
        drop(common);
        Ok(FiniteUnattachedTerminal { snapshot, cause })
    }

    fn reject(
        mut self,
        cause: FiniteUnattachedFailureCause<D>,
    ) -> FiniteUnattachedFailure<O, P, PP, U, D> {
        let terminal_cause = match &cause {
            FiniteUnattachedFailureCause::OwnershipUnverified => {
                PreparationFailureCode::product_graph(0x5541_4f57)
            }
            FiniteUnattachedFailureCause::Ledger(_) => PreparationFailureCode::ledger(),
        };
        let common = self
            .common
            .as_ref()
            .expect("finite rejection retains its original invocation");
        record_terminal_cause_with_precedence(
            &common.lifecycle.sticky_terminal_cause,
            terminal_cause,
        );
        let mut failure = FiniteUnattachedFailure {
            cause,
            residue: None,
        };
        std::mem::swap(&mut failure.residue, &mut self.common);
        let _ = self
            .admission
            .account()
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        failure
    }
}

impl<O, P, PP, U, D> Drop for FiniteUnattachedGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if let Some(common) = self.common.take() {
            let _ = self
                .admission
                .account()
                .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            std::mem::forget(common);
        }
    }
}

#[cfg(test)]
mod tests;
