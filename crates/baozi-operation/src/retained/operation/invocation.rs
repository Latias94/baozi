//! Concrete invocation owners share lifecycle state through short disjoint loans.

use super::*;
use crate::coordinator::CoordinatorView;
use crate::coordinator::finite::FiniteOperationCoordinator;
use crate::ledger::LedgerReadView;
use crate::ledger::finite::FiniteLedgerLimits;

mod attached;
mod finite_attached;
mod finite_unattached;
pub(in crate::retained) use attached::DynamicTerminalRootFields;
pub(crate) use attached::{AttachedTerminalDecision, AttachedTerminalStamp};
pub(in crate::retained::operation) use finite_attached::FiniteTerminalRootFields;
pub(in crate::retained) use finite_unattached::{
    FiniteUnattachedFailure, FiniteUnattachedTerminal,
};

/// A short selection of the actual installed state; consuming ownership stays concrete.
pub(in crate::retained) enum InvocationSlot<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    Dynamic(&'inv mut Option<OperationInvocation<O, P, PP, U, D>>),
    Finite(&'inv mut Option<FiniteOperationInvocation<O, P, PP, U, D>>),
}

impl<'inv, O, P, PP, U, D> InvocationSlot<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn reborrow(&mut self) -> InvocationSlot<'_, O, P, PP, U, D> {
        match self {
            Self::Dynamic(slot) => InvocationSlot::Dynamic(slot),
            Self::Finite(slot) => InvocationSlot::Finite(slot),
        }
    }

    pub(super) fn is_installed(&self) -> bool {
        match self {
            Self::Dynamic(slot) => slot.is_some(),
            Self::Finite(slot) => slot.is_some(),
        }
    }

    pub(super) fn invocation_identity(&self) -> *const () {
        match self {
            Self::Dynamic(slot) => std::ptr::from_ref(&**slot).cast(),
            Self::Finite(slot) => std::ptr::from_ref(&**slot).cast(),
        }
    }

    pub(super) fn into_view(self) -> OperationInvocationView<'inv, O, P, PP, U, D> {
        match self {
            Self::Dynamic(slot) => slot
                .as_mut()
                .expect("the dynamic state stays in its original slot")
                .view(),
            Self::Finite(slot) => slot
                .as_mut()
                .expect("the finite state stays in its original slot")
                .view(),
        }
    }

    pub(super) fn into_terminal(
        mut self,
        contained: &'inv mut bool,
    ) -> TerminalInvocationLoan<'inv, O, P, PP, U, D> {
        self.lifecycle_mut().terminal_outstanding = true;
        TerminalInvocationLoan {
            common: self,
            contained,
            not_send_or_sync: PhantomData,
        }
    }

    pub(super) fn as_read(&self) -> LedgerReadView<'_, O, P, D> {
        match self {
            Self::Dynamic(slot) => slot
                .as_ref()
                .expect("the dynamic state is installed")
                .coordinator
                .retained_ledger()
                .as_read(),
            Self::Finite(slot) => slot
                .as_ref()
                .expect("the finite state is installed")
                .coordinator
                .retained_ledger()
                .as_read(),
        }
    }

    pub(super) fn ledger_view(&mut self) -> crate::ledger::LedgerView<'_, O, P, D> {
        match self {
            Self::Dynamic(slot) => slot
                .as_mut()
                .expect("the dynamic state is installed")
                .coordinator
                .retained_ledger_mut()
                .view(),
            Self::Finite(slot) => slot
                .as_mut()
                .expect("the finite state is installed")
                .coordinator
                .retained_ledger_mut()
                .view(),
        }
    }

    pub(super) fn lifecycle_mut(&mut self) -> &mut InvocationLifecycle<P> {
        match self {
            Self::Dynamic(slot) => {
                &mut slot
                    .as_mut()
                    .expect("the dynamic state is installed")
                    .lifecycle
            }
            Self::Finite(slot) => {
                &mut slot
                    .as_mut()
                    .expect("the finite state is installed")
                    .lifecycle
            }
        }
    }

    pub(super) fn lifecycle(&self) -> &InvocationLifecycle<P> {
        match self {
            Self::Dynamic(slot) => {
                &slot
                    .as_ref()
                    .expect("the dynamic state is installed")
                    .lifecycle
            }
            Self::Finite(slot) => {
                &slot
                    .as_ref()
                    .expect("the finite state is installed")
                    .lifecycle
            }
        }
    }

    pub(super) fn recorded_control_cause(&self) -> Option<ControlCause> {
        match self {
            Self::Dynamic(slot) => slot
                .as_ref()
                .expect("the dynamic state is installed")
                .coordinator
                .recorded_control_cause(),
            Self::Finite(slot) => slot
                .as_ref()
                .expect("the finite state is installed")
                .coordinator
                .recorded_control_cause(),
        }
    }

    pub(super) fn known_administrative_bytes(&self) -> Option<u64> {
        match self {
            Self::Dynamic(slot) => slot
                .as_ref()
                .expect("the dynamic state is installed")
                .coordinator
                .known_administrative_bytes(),
            Self::Finite(slot) => slot
                .as_ref()
                .expect("the finite state is installed")
                .coordinator
                .known_administrative_bytes(),
        }
    }
}

enum InvocationTerminalSeal<P> {
    UnattachedReady,
    AttachedReady(AttachedTerminalDecision<P>),
}

/// A private decision for one actual ledger; it owns neither state nor an account permission.
pub(super) struct UnattachedTerminalStamp {
    identity: crate::ledger::LedgerIdentity,
}

pub(in crate::retained) struct InvocationLifecycle<P> {
    pub(super) sticky_terminal_cause: Cell<Option<PreparationFailureCode>>,
    pub(super) control_action_debt: Cell<u16>,
    pub(super) execution_state: Cell<StoreExecutionState>,
    pub(super) canonical_lineage: CanonicalLineageAuthority,
    pub(super) terminal_outstanding: bool,
    terminal_seal: Option<InvocationTerminalSeal<P>>,
}

#[cfg(test)]
mod finite_attached_tests;
#[cfg(test)]
mod finite_physical_tests;
#[cfg(test)]
mod tests;

impl<P> InvocationLifecycle<P> {
    pub(super) fn new(canonical_lineage: CanonicalLineageAuthority) -> Self {
        Self {
            sticky_terminal_cause: Cell::new(None),
            control_action_debt: Cell::new(0),
            execution_state: Cell::new(StoreExecutionState::Running),
            canonical_lineage,
            terminal_outstanding: false,
            terminal_seal: None,
        }
    }

    fn try_consume_unattached_stamp(
        &mut self,
        stamp: UnattachedTerminalStamp,
        identity: crate::ledger::LedgerIdentity,
    ) -> bool {
        if !self.terminal_outstanding
            || !matches!(
                self.terminal_seal,
                Some(InvocationTerminalSeal::UnattachedReady)
            )
            || stamp.identity != identity
        {
            return false;
        }
        self.terminal_seal = None;
        // Capture still needs the actual owner's fail-closed unwind protection.
        true
    }

    pub(super) fn record_terminal_gate<D>(
        &self,
        result: Result<(), PublicationCause<D>>,
    ) -> Option<ControlCause> {
        match result {
            Ok(()) => None,
            Err(PublicationCause::Control(cause)) => {
                record_terminal_cause_with_precedence(
                    &self.sticky_terminal_cause,
                    PreparationFailureCode::from_control(&cause),
                );
                Some(cause)
            }
            Err(PublicationCause::Ledger(_)) => {
                record_terminal_cause_with_precedence(
                    &self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
                None
            }
        }
    }

    pub(super) fn preflight_exclusive<O, D: Copy + Ord>(
        &self,
        recorded: impl FnOnce() -> Option<ControlCause>,
        ledger: LedgerReadView<'_, O, P, D>,
    ) -> Result<(), ExecutionError<D>>
    where
        P: Copy + Ord,
    {
        check_execution_state(&self.sticky_terminal_cause, &self.execution_state, recorded)?;
        ledger.check_work_quiescent().map_err(|cause| {
            record_terminal_cause_with_precedence(
                &self.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            ExecutionError::Ledger(cause)
        })
    }
}

impl<O, P, PP, U, D> OperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn view(&mut self) -> OperationInvocationView<'_, O, P, PP, U, D> {
        OperationInvocationView::new(self.coordinator.view(), &mut self.lifecycle)
    }

    #[expect(
        clippy::result_large_err,
        reason = "a rejected private stamp returns the same actual invocation without allocation"
    )]
    pub(super) fn finish_unattached(
        self,
        stamp: UnattachedTerminalStamp,
        account: &PhysicalStorageAccount,
    ) -> Result<UnattachedTerminalEvidence<P, D>, Self> {
        let mut owner = scopeguard::guard(self, |owner| {
            let _ =
                account.record_poison(super::super::AccountPoisonReason::OwnershipGraphUnverified);
            std::mem::forget(owner);
        });
        let identity = owner.coordinator.retained_ledger().as_read().identity();
        if !owner
            .lifecycle
            .try_consume_unattached_stamp(stamp, identity)
        {
            return Err(scopeguard::ScopeGuard::into_inner(owner));
        }
        let (snapshot, ledger_cause) = owner.coordinator.take_terminal_snapshot();
        if ledger_cause.is_some() {
            record_terminal_cause_with_precedence(
                &owner.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        }
        let cause = owner.lifecycle.sticky_terminal_cause.get();
        let contained = ledger_cause.is_some();
        if contained {
            drop(owner);
        } else {
            let mut owner = scopeguard::ScopeGuard::into_inner(owner);
            owner.lifecycle.terminal_outstanding = false;
            drop(owner);
        }
        Ok(UnattachedTerminalEvidence {
            snapshot,
            cause,
            contained,
        })
    }
}

pub(crate) struct FiniteOperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) coordinator: FiniteOperationCoordinator<O, P, PP, U, D>,
    pub(super) lifecycle: InvocationLifecycle<P>,
}

impl<O, P, PP, U, D> FiniteOperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn try_new(
        control: OperationControl<PP, U>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        canonical_lineage: CanonicalLineageAuthority,
    ) -> Result<Self, LedgerCause<D>> {
        Self::try_new_with_control(&mut Some(control), limits, phases, canonical_lineage)
    }

    pub(super) fn try_new_with_control(
        control: &mut Option<OperationControl<PP, U>>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        canonical_lineage: CanonicalLineageAuthority,
    ) -> Result<Self, LedgerCause<D>> {
        Ok(Self {
            coordinator: FiniteOperationCoordinator::try_new_with_control(control, limits, phases)?,
            lifecycle: InvocationLifecycle::new(canonical_lineage),
        })
    }

    pub(crate) fn view(&mut self) -> OperationInvocationView<'_, O, P, PP, U, D> {
        OperationInvocationView::new(self.coordinator.view(), &mut self.lifecycle)
    }

    pub(crate) fn known_administrative_bytes(&self) -> Option<u64> {
        self.coordinator.known_administrative_bytes()
    }

    pub(crate) fn terminal_ledger_mut(
        &mut self,
    ) -> &mut crate::ledger::finite::FiniteLedger<O, P, D> {
        self.coordinator.retained_ledger_mut()
    }

    pub(crate) fn preflight_exclusive(&self) -> Result<(), ExecutionError<D>> {
        self.lifecycle.preflight_exclusive(
            || self.coordinator.recorded_control_cause(),
            self.coordinator.retained_ledger().as_read(),
        )
    }
}

pub(crate) struct OperationInvocationView<'loan, O, P, PP, U, D> {
    pub(in crate::retained) coordinator: CoordinatorView<'loan, O, P, PP, U, D>,
    pub(super) lifecycle: &'loan mut InvocationLifecycle<P>,
}

impl<'loan, O, P, PP, U, D> OperationInvocationView<'loan, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn new(
        coordinator: CoordinatorView<'loan, O, P, PP, U, D>,
        lifecycle: &'loan mut InvocationLifecycle<P>,
    ) -> Self {
        Self {
            coordinator,
            lifecycle,
        }
    }

    pub(super) fn prepare_terminal_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Option<ControlCause> {
        let was_outstanding = std::mem::replace(&mut self.lifecycle.terminal_outstanding, true);
        let result = self.coordinator.try_prepare_typed_control(final_progress);
        // A normal return restores the original abandonment duty. An escaping callback
        // leaves the actual root armed until its outer owner contains the graph.
        self.lifecycle.terminal_outstanding = was_outstanding;
        self.lifecycle.record_terminal_gate(result)
    }

    pub(super) fn close_terminal_owners(&mut self) -> Option<ControlCause> {
        let was_outstanding = std::mem::replace(&mut self.lifecycle.terminal_outstanding, true);
        let result = self.coordinator.close_for_terminal_extraction();
        self.lifecycle.terminal_outstanding = was_outstanding;
        self.lifecycle.record_terminal_gate(result)
    }

    pub(super) fn prepare_unattached_terminal(
        mut self,
        account: &PhysicalStorageAccount,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<UnattachedTerminalStamp, ()> {
        if self.lifecycle.terminal_outstanding || self.lifecycle.terminal_seal.is_some() {
            return Err(());
        }
        // Root/finite guard cleanup must cover owner-defined progress code as well as observers.
        self.lifecycle.terminal_outstanding = true;
        let account_ready = match account.try_begin_account_preflight() {
            Ok(permit) => {
                drop(permit);
                true
            }
            Err(_) => {
                record_terminal_cause_with_precedence(
                    &self.lifecycle.sticky_terminal_cause,
                    PreparationFailureCode::product_graph(0x5541_4343),
                );
                false
            }
        };
        if account_ready && self.lifecycle.sticky_terminal_cause.get().is_none() {
            self.prepare_terminal_control(final_progress);
        }
        self.close_terminal_owners();
        // Control-owner disposal may quarantine this account. No client callback follows.
        match account.try_begin_account_preflight() {
            Ok(permit) => drop(permit),
            Err(_) => record_terminal_cause_with_precedence(
                &self.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::product_graph(0x5541_4343),
            ),
        }
        self.lifecycle.terminal_seal = Some(InvocationTerminalSeal::UnattachedReady);
        Ok(UnattachedTerminalStamp {
            identity: self.coordinator.ledger_read().identity(),
        })
    }

    pub(crate) fn into_exclusive_parts(
        self,
        account: &'loan PhysicalStorageAccount,
        share_work: RetainedHandleSharePlan<D>,
    ) -> Result<ExclusiveOperationParts<'loan, O, P, PP, U, D>, ExecutionError<D>> {
        let (poll, progress, ledger) = self.coordinator.into_exclusive_progress_and_ledger();
        let (work, storage) = ledger.into_exclusive().map_err(|cause| {
            record_terminal_cause_with_precedence(
                &self.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            ExecutionError::Ledger(cause)
        })?;
        Ok(ExclusiveOperationParts {
            work,
            progress,
            ordinary: ExecutionOrdinaryStorage {
                storage,
                poll,
                failure: &self.lifecycle.sticky_terminal_cause,
                debt: &self.lifecycle.control_action_debt,
                execution_state: &self.lifecycle.execution_state,
            },
            account,
            share_work,
        })
    }
}
