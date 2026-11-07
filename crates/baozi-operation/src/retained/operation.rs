use super::operation_start::RetainedFailureTerminalWorkRequirement;
use super::ownership::FallbackPublicationBaselineLoan;
use super::ownership::finish_pre_attribution_batch_cleanup;
use super::publication_contract::{
    RetainedDeferredStage, RetainedPublicationOpenDisposition,
    retained_open_failure_is_rollback_safe,
};
use super::startup;
use super::terminal_loan::TerminalPhysicalSlot;
use super::{
    AccountPoisonReason, AssemblyContainmentProvenance, CanonicalLineageAuthority,
    EvidenceOnlySuccessPrimaryTransition, FailureTerminalWorkState, FallbackPublicationBaseline,
    PhysicalStorageAccount, PreparationFailureClass, PreparationFailureCode,
    PreparedRetainedEvidenceOnlyOutcome, PreparedRetainedPublication,
    PreparedRetainedPublicationOutcome, PublishedCanonicalAggregate,
    QualifiedRetainedBlockTablePlan, RetainedAllocationError, RetainedAllocationPlan,
    RetainedBatchLimits, RetainedBuildFailureCode, RetainedFailureTerminalWorkPlan,
    RetainedHandleSharePlan, RetainedMembershipDestination, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedOutcomeMode, RetainedPublicationAssembly,
    RetainedPublicationOpenError, RetainedPublicationPlan, RetainedPublicationPrepareError,
    RetainedStorageDropOutcome, RetainedWriterError, RetainedWriterPlan,
    StartupCrossPlaneAdmissionFailure, StartupCrossPlanePlan, StartupPhysicalManifest,
    StartupPhysicalNoMutation, StartupRetainedPartGuard, StartupTerminalEvidenceBuildError,
    TerminalEvidenceBackingPlan, TerminalEvidenceKey, TerminalFallbackAttribution,
    TerminalFallbackGuard, TerminalLedgerEvidence, TerminalPrimaryGuard,
    drop_retained_table_with_account_anchor, map_batch_error, try_create_retained_block_table,
    try_open_publication_account, try_qualify_retained_block_table_plan,
    unreachable_final_owner_state,
};
use super::{AccountPoisoned, FinalizationContainmentError};
#[cfg(test)]
use super::{
    STARTUP_FAILURE_LEDGER_SNAPSHOT, STARTUP_READY_FINAL_OWNER_ALLOCATOR_ENTRIES,
    record_start_evidence_build_call, take_injected_start_evidence_build_failure,
    take_injected_start_evidence_build_panic, take_injected_start_evidence_structural_failure,
    test_retained_final_owner_allocator_entries,
};
use crate::control::discard_untrusted_panic_payload;
use crate::coordinator::{CoordinatorView, OperationCoordinator, OperationPollCheckpoint};
use crate::ledger::{
    AttributionBatchPlan, CanonicalOutput, LedgerCause, LedgerLimits, LiveReservation,
    OutcomeEvidenceDestination, PhaseCompletionError, StartupAccountShareEnvelope,
    StartupAttributionBatchPlans, StartupLedgerPlanParts, StartupRetainedAttribution,
};
use crate::ledger::{DimensionPolicy, DimensionSnapshot};
use crate::{
    ControlCause, FinalProgress, OperationControl, OperationOwner, ProgressPhase, ProgressUnit,
    PublicationCause,
};
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

mod borrowed_work;
pub(crate) mod finite_root;
mod invocation;
mod publication_open;
pub(crate) use borrowed_work::{
    BorrowedRetainedWorkPermission, RetainedFailureWorkBinding, RetainedFailureWorkConsumeError,
    RetainedFailureWorkControl, RetainedFailureWorkFinishError, RetainedFailureWorkPermission,
    RetainedFailureWorkStartError, RetainedWorkBinding, RetainedWorkConsumeError,
    RetainedWorkControl, RetainedWorkFinishError, RetainedWorkPermission, RetainedWorkRoot,
    RetainedWorkStartError,
};
use invocation::InvocationLifecycle;
pub(crate) use invocation::{AttachedTerminalDecision, AttachedTerminalStamp};
pub(super) use invocation::{DynamicTerminalRootFields, InvocationSlot};
pub(crate) use invocation::{FiniteOperationInvocation, OperationInvocationView};
pub(crate) use publication_open::{RetainedAssemblyOpened, RetainedPublicationBorrowFailure};
mod execution;
#[cfg(test)]
mod root_tests;
pub(crate) use execution::{
    ExecutionError, ExecutionOrdinaryStorage, check_execution_state, poll_execution,
};

/// Execution has stopped independently of the owner's eventual terminal classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreExecutionState {
    Running,
    KnownError,
    IncompleteSuccess,
    UnwindPending,
}

impl StoreExecutionState {
    pub(crate) const ABANDONED: u32 = 0x4558_4142;

    pub(crate) fn stop(slot: &Cell<Self>, reason: Self) {
        if slot.get() == Self::Running {
            slot.set(reason);
        }
    }

    pub(crate) fn settle_caught_unwind(
        slot: &Cell<Self>,
        failure: &Cell<Option<PreparationFailureCode>>,
    ) {
        if slot.get() == Self::UnwindPending {
            record_terminal_cause_with_precedence(
                failure,
                PreparationFailureCode::product_graph(Self::ABANDONED),
            );
        }
    }
}

pub(crate) struct ExclusiveOperationParts<'op, O, P, PP, U, D> {
    pub(crate) work: crate::ledger::ExclusiveWork<'op, D>,
    pub(crate) ordinary: ExecutionOrdinaryStorage<'op, O, P, PP, U, D>,
    pub(crate) progress: crate::coordinator::OperationProgressFields<'op, PP, U>,
    pub(crate) account: &'op PhysicalStorageAccount,
    pub(crate) share_work: RetainedHandleSharePlan<D>,
}

/// One checked weighted-debt calculation for the exclusive runner and migration adapters.
/// Err is an invalid action cost; None means a full checkpoint is required first.
pub(crate) fn checked_action_debt(current: u16, cost: u16) -> Result<Option<u16>, ()> {
    if cost == 0 || usize::from(cost) > crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS {
        return Err(());
    }
    Ok(current
        .checked_add(cost)
        .filter(|next| usize::from(*next) <= crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS))
}

const FAILURE_WORK_PERMIT_ABANDONED_CODE: u32 = 0x4657_5041;
const FAILURE_WORK_PERMIT_OVERRUN_CODE: u32 = 0x4657_504f;
pub(crate) const ORDINARY_WORK_PERMIT_ABANDONED_CODE: u32 = 0x4f57_5041;
const ORDINARY_WORK_PERMIT_OVERRUN_CODE: u32 = 0x4f57_504f;

#[cfg(test)]
thread_local! {
    static PANIC_NEXT_PUBLICATION_ACCOUNT_OPEN: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn panic_next_publication_account_open() {
    PANIC_NEXT_PUBLICATION_ACCOUNT_OPEN.with(|injected| injected.set(true));
}

#[cfg(test)]
fn inject_publication_account_open_panic() {
    let panic = PANIC_NEXT_PUBLICATION_ACCOUNT_OPEN.with(|injected| injected.replace(false));
    if panic {
        panic!("injected retained publication-account open panic");
    }
}

/// Invocation authority shared by every retained lifecycle stage, independently of its payloads.
pub(crate) struct OperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) coordinator: OperationCoordinator<O, P, PP, U, D>,
    lifecycle: InvocationLifecycle<P>,
}

impl<O, P, PP, U, D> OperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn record_terminal_gate(
        &self,
        result: Result<(), PublicationCause<D>>,
    ) -> Option<ControlCause> {
        self.lifecycle.record_terminal_gate(result)
    }

    pub(crate) fn terminal_ledger_mut(&mut self) -> &mut crate::ledger::Ledger<O, P, D> {
        self.coordinator.retained_ledger_mut()
    }

    #[cfg(test)]
    pub(crate) fn test_terminal_ledger(&self) -> &crate::ledger::Ledger<O, P, D> {
        self.coordinator.retained_ledger()
    }

    pub(crate) fn retire_after_detach(self, state: &mut crate::ledger::LedgerState<P, D>) {
        self.coordinator.retire_after_detach(state);
    }

    pub(crate) fn known_administrative_bytes(&self) -> Option<u64> {
        self.coordinator.known_administrative_bytes()
    }
}

/// The entry frame retains its original account admission while common stays in the root.
/// Client work only receives shorter loans of this frame's execution state.
pub(crate) struct PendingRetainedAttachment<
    'inv,
    'account,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    admission: startup::PendingOperationStartAdmission<'account>,
    plan: RetainedOperationStartPlan<P, D>,
    failure_terminal_work: Option<(
        RetainedFailureTerminalWorkPlan<D>,
        RetainedFailureTerminalWorkRequirement,
    )>,
    dimension_count: usize,
    phase_count: usize,
    local: PhantomData<Rc<()>>,
}

type RetainedAttachmentResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> = Result<
    PendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CB, EB>,
    RetainedOperationStartError<D>,
>;

type RetainedAttachmentFieldsResult<O, P, D, F, CB, EB> =
    Result<RetainedOperationFields<O, P, D, F, CB, EB>, RetainedOperationStartError<D>>;

type RetainedAssemblyOpenResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
    RetainedPublicationOpenError<RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>, D>,
>;

/// Original account admission retained after a terminal request ends the numeric loan.
/// The actual common/control stays in its outer root through client capture cleanup.
pub(crate) struct DeferredUnattachedBinding<'account> {
    admission: startup::PendingOperationStartAdmission<'account>,
    identity: *const (),
}

/// The existing pre-admitted snapshot buffers, with their exact terminal classification.
pub(crate) struct UnattachedTerminalEvidence<P, D> {
    pub(crate) snapshot: crate::LedgerSnapshot<P, D>,
    pub(crate) cause: Option<PreparationFailureCode>,
    pub(crate) contained: bool,
}

struct AttachmentGuard<'loan, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    common: &'loan mut OperationInvocation<O, P, PP, U, D>,
    resolved: bool,
}

impl<O, P, PP, U, D> AttachmentGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn reject(&mut self, cause: PreparationFailureCode) {
        reject_retained_attachment(&mut self.common.view(), cause);
        self.resolved = true;
    }
}

impl<O, P, PP, U, D> Drop for AttachmentGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if !self.resolved {
            self.reject(PreparationFailureCode::product_graph(
                StoreExecutionState::ABANDONED,
            ));
        }
    }
}

impl<O, P, PP, U, D> OperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn preflight_exclusive(&self) -> Result<(), ExecutionError<D>> {
        self.lifecycle.preflight_exclusive(
            || self.coordinator.recorded_control_cause(),
            self.coordinator.retained_ledger().as_read(),
        )
    }

    fn borrow_exclusive_parts<'loan>(
        &'loan mut self,
        account: &'loan PhysicalStorageAccount,
        share_work: RetainedHandleSharePlan<D>,
    ) -> Result<ExclusiveOperationParts<'loan, O, P, PP, U, D>, ExecutionError<D>> {
        self.view().into_exclusive_parts(account, share_work)
    }
}

#[allow(
    clippy::type_complexity,
    clippy::result_large_err,
    reason = "the entry frame retains actual root/account loans and inline attachment causes"
)]
impl<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    PendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(crate) fn invocation_identity(&self) -> *const () {
        self.root.invocation_identity()
    }

    /// Stops ordinary work now; only the outer entry may later prepare terminal facts.
    pub(crate) fn defer_unattached(
        self,
        requested: Option<PreparationFailureCode>,
    ) -> Result<DeferredUnattachedBinding<'account>, Self> {
        if self.root.retained.is_some() || self.root.assembly.is_some() {
            return Err(self);
        }
        let Some(common) = self.root.common.as_mut() else {
            return Err(self);
        };
        if let Some(cause) = requested {
            record_terminal_cause_with_precedence(&common.lifecycle.sticky_terminal_cause, cause);
        } else if common.lifecycle.execution_state.get() != StoreExecutionState::Running {
            record_terminal_cause_with_precedence(
                &common.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::product_graph(StoreExecutionState::ABANDONED),
            );
        }
        if common
            .coordinator
            .retained_ledger_mut()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &common.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        }
        Ok(DeferredUnattachedBinding {
            admission: self.admission,
            identity: self.root.invocation_identity(),
        })
    }

    pub(crate) fn exclusive_parts(
        &mut self,
    ) -> Result<ExclusiveOperationParts<'_, O, P, PP, U, D>, ExecutionError<D>> {
        let common = self.root.common.as_mut().ok_or(ExecutionError::Stopped)?;
        common.preflight_exclusive()?;
        if common
            .coordinator
            .retained_ledger()
            .ordinary_work_is_sealed()
        {
            return Err(ExecutionError::Stopped);
        }
        common.borrow_exclusive_parts(
            self.admission.account(),
            self.plan.terminal_account_share_work,
        )
    }

    pub(crate) fn attach(
        self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<
        RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedOperationStartError<D>,
    > {
        let Self {
            root,
            admission,
            plan,
            failure_terminal_work,
            dimension_count,
            phase_count,
            ..
        } = self;
        let Some(common) = root.common.as_mut() else {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ));
        };
        let mut guard = AttachmentGuard {
            common,
            resolved: false,
        };
        let result = RetainedOperation::prepare_attachment(
            guard.common,
            admission,
            plan,
            failure_terminal_work,
            dimension_count,
            phase_count,
            build_fallback,
        );
        let fields = match result {
            Ok(fields) => fields,
            Err(cause) => {
                let code = match &cause {
                    RetainedOperationStartError::Control(cause) => {
                        PreparationFailureCode::from_control(cause)
                    }
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(_)) => {
                        PreparationFailureCode::ledger()
                    }
                    _ => PreparationFailureCode::product_graph(0x4154_5443),
                };
                guard.reject(code);
                return Err(cause);
            }
        };
        guard.resolved = true;
        drop(guard);
        root.retained = Some(fields);
        Ok(RetainedOperation::from_loan(RetainedOperationLoan {
            retained: root
                .retained
                .as_mut()
                .expect("attachment installs fully qualified retained fields")
                .into(),
            assembly: Some(&mut root.assembly),
            terminal: TerminalPhysicalSlot::Dynamic(&mut root.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Dynamic(&mut root.common),
                contained: &mut root.contained,
                not_send_or_sync: PhantomData,
            },
        }))
    }
}

/// The outer entry retains the actual invocation even when a short stage loan is forgotten.
pub(crate) struct InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    terminal: Option<super::TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>>,
    // Physical owners precede common so their cleanup retains the invocation's ledger.
    assembly: super::PublicationAssemblySlot<O, P, D, CanonicalBlock, EvidenceBlock>,
    retained: Option<RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
    common: Option<OperationInvocation<O, P, PP, U, D>>,
    started: bool,
    contained: bool,
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(crate) const fn new() -> Self {
        Self {
            retained: None,
            assembly: None,
            terminal: None,
            common: None,
            started: false,
            contained: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn install_ledger_for_test(&mut self, ledger: crate::ledger::Ledger<O, P, D>) {
        assert!(!self.started);
        self.started = true;
        self.common = Some(OperationInvocation {
            coordinator: OperationCoordinator::from_ledger_for_test(ledger),
            lifecycle: InvocationLifecycle::new(CanonicalLineageAuthority::fresh()),
        });
    }

    #[cfg(test)]
    pub(crate) fn take_terminal_invocation_for_test(
        &mut self,
    ) -> OperationInvocation<O, P, PP, U, D> {
        // Numeric fixtures own their batches separately; keep their outer containment duty armed.
        self.contained = true;
        let common = self
            .common
            .as_mut()
            .expect("the fixture installed its original invocation");
        common.lifecycle.terminal_outstanding = true;
        self.common
            .take()
            .expect("the fixture takes its actual invocation once")
    }

    #[cfg(test)]
    pub(crate) fn terminal_loan_for_test(&mut self) -> TerminalInvocationLoan<'_, O, P, PP, U, D> {
        InvocationLoan {
            common: InvocationSlot::Dynamic(&mut self.common),
            contained: &mut self.contained,
            not_send_or_sync: PhantomData,
        }
        .into_terminal()
    }

    pub(crate) fn cleanup_owned_state(&mut self) {
        if let Some(physical) = self.terminal.take() {
            self.contained |= !physical.is_empty();
            drop(physical);
        }
        let retain = cleanup_retained_root_graph(
            self.retained
                .as_mut()
                .map(RetainedOperationFieldsLoan::from),
            &mut self.assembly,
            TerminalPhysicalSlot::Dynamic(&mut self.terminal),
            Some(InvocationSlot::Dynamic(&mut self.common)),
            &mut self.contained,
        );
        if retain {
            std::mem::forget(self.assembly.take());
            std::mem::forget(self.retained.take());
            std::mem::forget(self.common.take());
        } else {
            drop(self.retained.take());
            let unresolved = InvocationLoan {
                common: InvocationSlot::Dynamic(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            }
            .close_abandoned_common();
            if unresolved {
                std::mem::forget(self.common.take());
            }
        }
    }

    pub(crate) fn must_retain_domain(&self) -> bool {
        self.contained
    }

    pub(crate) fn has_started(&self) -> bool {
        self.started
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        std::ptr::from_ref(&self.common).cast()
    }

    pub(crate) fn terminal_catalog(&self) -> Option<&crate::ledger::Ledger<O, P, D>> {
        self.common
            .as_ref()
            .map(|common| common.coordinator.retained_ledger())
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(crate) fn contain_payload_failure(&mut self) -> Option<FinalizationContainmentError> {
        let retained = self.retained.as_mut()?;
        if self.common.is_none() || retained.baseline.first_account().is_none() {
            return None;
        }
        retained.metadata.deferred_stage = None;
        let operation = RetainedOperationLoan {
            retained: retained.into(),
            assembly: None,
            terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Dynamic(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        };
        if let Some(fields) = self.assembly.as_mut()
            && !fields.is_empty()
        {
            Some(
                super::RetainedPublicationAssemblyLoan {
                    fields: fields.into(),
                    operation: Some(operation),
                }
                .contain_activation_fault(),
            )
        } else {
            Some(operation.contain_activation_fault())
        }
    }

    /// Runs only after the real body and its captures have been disposed by the outer host.
    pub(crate) fn prepare_unattached_terminal<'account>(
        &mut self,
        binding: DeferredUnattachedBinding<'account>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<UnattachedTerminalEvidence<P, D>, DeferredUnattachedBinding<'account>> {
        if binding.identity != self.invocation_identity()
            || self.retained.is_some()
            || self.assembly.is_some()
            || self.common.is_none()
        {
            return Err(binding);
        }
        let account = binding.admission.account();
        let prepared = {
            let _preparation_unwind = scopeguard::guard_on_unwind(account, |account| {
                let _ = account.record_poison(super::AccountPoisonReason::OwnershipGraphUnverified);
            });
            self.common
                .as_mut()
                .expect("the original common was checked")
                .view()
                .prepare_unattached_terminal(account, final_progress)
        };
        let stamp = match prepared {
            Ok(stamp) => stamp,
            Err(()) => return Err(binding),
        };
        // All preparation loans have ended. The concrete leaf owns capture and dirty state.
        // Keep the original domain protected if capture unwinds after taking common.
        let already_contained = self.contained;
        self.contained = true;
        let common = self
            .common
            .take()
            .expect("unattached close consumes the actual state once");
        match common.finish_unattached(stamp, account) {
            Ok(prepared) => {
                self.contained = already_contained || prepared.contained;
                Ok(prepared)
            }
            Err(common) => {
                // A private stamp mismatch returns the same state, never fresh start authority.
                self.common = Some(common);
                Err(binding)
            }
        }
    }

    pub(crate) fn record_payload_failure(&mut self, code: u32) {
        let Some(common) = self.common.as_mut() else {
            return;
        };
        StoreExecutionState::stop(
            &common.lifecycle.execution_state,
            StoreExecutionState::KnownError,
        );
        record_terminal_cause_with_precedence(
            &common.lifecycle.sticky_terminal_cause,
            PreparationFailureCode::product_graph(code),
        );
        if common
            .coordinator
            .retained_ledger_mut()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &common.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        }
    }

    fn deferred_assembly_is_available(&self) -> bool {
        let (Some(retained), Some(fields)) = (self.retained.as_ref(), self.assembly.as_ref())
        else {
            return false;
        };
        self.common.is_some()
            && retained.has_deferred_storage()
            && fields.has_deferred_storage()
            && fields
                .account
                .as_ref()
                .zip(retained.baseline.first_account())
                .is_some_and(|(account, baseline)| account.is_same_account(baseline))
    }

    #[allow(
        clippy::type_complexity,
        reason = "restoration lends the existing operation without erasing its block types"
    )]
    pub(crate) fn resume_deferred_operation(
        &mut self,
    ) -> Option<RetainedOperation<'_, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>> {
        if self.common.is_none() || self.assembly.is_some() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        if !retained.has_deferred_storage()
            || !matches!(
                retained.metadata.deferred_stage,
                Some(RetainedDeferredStage::Operation)
            )
        {
            return None;
        }
        retained.metadata.deferred_stage = Some(RetainedDeferredStage::TerminalOperation);
        Some(RetainedOperation::from_loan(RetainedOperationLoan {
            retained: retained.into(),
            assembly: Some(&mut self.assembly),
            terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Dynamic(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        }))
    }

    #[allow(
        clippy::type_complexity,
        reason = "restoration lends the existing physical assembly without erasing its block types"
    )]
    pub(crate) fn resume_deferred_assembly(
        &mut self,
    ) -> Option<RetainedPublicationAssembly<'_, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>
    {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let fields = self.assembly.as_mut()?;
        if !matches!(
            retained.metadata.deferred_stage,
            Some(RetainedDeferredStage::Assembly)
        ) {
            return None;
        }
        retained.metadata.deferred_stage = None;
        Some(RetainedPublicationAssembly::from_loan(
            super::RetainedPublicationAssemblyLoan {
                fields: fields.into(),
                operation: Some(RetainedOperationLoan {
                    retained: retained.into(),
                    assembly: None,
                    terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
                    common: InvocationLoan {
                        common: InvocationSlot::Dynamic(&mut self.common),
                        contained: &mut self.contained,
                        not_send_or_sync: PhantomData,
                    },
                }),
            },
        ))
    }

    #[allow(
        clippy::type_complexity,
        reason = "restoration lends the existing failure assembly without erasing its block types"
    )]
    pub(crate) fn resume_deferred_failure(
        &mut self,
    ) -> Option<super::RetainedFailureAssembly<'_, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>
    {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let fields = self.assembly.as_mut()?;
        let RetainedDeferredStage::Failure { fallback_cause } =
            retained.metadata.deferred_stage.as_ref()?
        else {
            return None;
        };
        let fallback_cause = *fallback_cause;
        retained.metadata.deferred_stage = None;
        Some(super::RetainedFailureAssembly {
            assembly: RetainedPublicationAssembly::from_loan(
                super::RetainedPublicationAssemblyLoan {
                    fields: fields.into(),
                    operation: Some(RetainedOperationLoan {
                        retained: retained.into(),
                        assembly: None,
                        terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
                        common: InvocationLoan {
                            common: InvocationSlot::Dynamic(&mut self.common),
                            contained: &mut self.contained,
                            not_send_or_sync: PhantomData,
                        },
                    }),
                },
            ),
            fallback_cause,
        })
    }

    #[allow(
        clippy::type_complexity,
        reason = "restoration lends the existing fallback assembly without erasing its block types"
    )]
    pub(crate) fn resume_deferred_fallback(
        &mut self,
    ) -> Option<super::RetainedFallbackOnly<'_, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>
    {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let fields = self.assembly.as_mut()?;
        let RetainedDeferredStage::Fallback {
            cause,
            activation_containment_required,
        } = retained.metadata.deferred_stage.as_ref()?
        else {
            return None;
        };
        let cause = *cause;
        let activation_containment_required = *activation_containment_required;
        retained.metadata.deferred_stage = None;
        Some(super::RetainedFallbackOnly {
            assembly: RetainedPublicationAssembly::from_loan(
                super::RetainedPublicationAssemblyLoan {
                    fields: fields.into(),
                    operation: Some(RetainedOperationLoan {
                        retained: retained.into(),
                        assembly: None,
                        terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
                        common: InvocationLoan {
                            common: InvocationSlot::Dynamic(&mut self.common),
                            contained: &mut self.contained,
                            not_send_or_sync: PhantomData,
                        },
                    }),
                },
            ),
            cause,
            activation_containment_required,
        })
    }

    #[allow(
        clippy::type_complexity,
        reason = "restoration lends the existing physical assembly without erasing its block types"
    )]
    pub(crate) fn resume_deferred_success(
        &mut self,
    ) -> Option<super::ValidatedRetainedSuccess<'_, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>
    {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let fields = self.assembly.as_mut()?;
        if !matches!(
            retained.metadata.deferred_stage,
            Some(RetainedDeferredStage::Success { .. })
        ) {
            return None;
        }
        let RetainedDeferredStage::Success { proof } = retained.metadata.deferred_stage.take()?
        else {
            return None;
        };
        Some(super::ValidatedRetainedSuccess {
            assembly: RetainedPublicationAssembly::from_loan(
                super::RetainedPublicationAssemblyLoan {
                    fields: fields.into(),
                    operation: Some(RetainedOperationLoan {
                        retained: retained.into(),
                        assembly: None,
                        terminal: TerminalPhysicalSlot::Dynamic(&mut self.terminal),
                        common: InvocationLoan {
                            common: InvocationSlot::Dynamic(&mut self.common),
                            contained: &mut self.contained,
                            not_send_or_sync: PhantomData,
                        },
                    }),
                },
            ),
            proof,
        })
    }
}

impl<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.cleanup_owned_state();
    }
}

/// Private transition loan. This never owns the coordinator, control or numeric state.
pub(super) struct InvocationLoan<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    common: InvocationSlot<'inv, O, P, PP, U, D>,
    contained: &'inv mut bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'inv, O, P, PP, U, D> InvocationLoan<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn reborrow(&mut self) -> InvocationLoan<'_, O, P, PP, U, D> {
        InvocationLoan {
            common: self.common.reborrow(),
            contained: self.contained,
            not_send_or_sync: PhantomData,
        }
    }

    fn close_abandoned_common(&mut self) -> bool {
        if !self.common.is_installed() {
            return false;
        }
        // The original control owner closes after physical disposal. The shared view
        // preserves scoped outstanding authority if an owner-defined callback escapes.
        self.view().close_terminal_owners();
        let unresolved = self.as_read().terminal_residue_cause().is_some();
        *self.contained |= unresolved;
        unresolved
    }

    pub(super) fn view(&mut self) -> OperationInvocationView<'_, O, P, PP, U, D> {
        self.common.reborrow().into_view()
    }

    pub(super) fn as_read(&self) -> crate::ledger::LedgerReadView<'_, O, P, D> {
        self.common.as_read()
    }

    pub(super) fn lifecycle(&self) -> &InvocationLifecycle<P> {
        self.common.lifecycle()
    }

    pub(super) fn lifecycle_mut(&mut self) -> &mut InvocationLifecycle<P> {
        self.common.lifecycle_mut()
    }

    pub(super) fn recorded_control_cause(&self) -> Option<ControlCause> {
        self.common.recorded_control_cause()
    }

    pub(super) fn preflight_exclusive(&self) -> Result<(), ExecutionError<D>> {
        self.lifecycle()
            .preflight_exclusive(|| self.recorded_control_cause(), self.as_read())
    }

    #[cfg(test)]
    pub(super) fn test_ledger(&self) -> &crate::ledger::Ledger<O, P, D> {
        let InvocationSlot::Dynamic(common) = &self.common else {
            panic!("the dynamic test hook requires its original dynamic slot");
        };
        common
            .as_ref()
            .expect("the test inspects the original installed ledger")
            .coordinator
            .retained_ledger()
    }

    #[cfg(test)]
    pub(super) fn test_ledger_mut(&mut self) -> &mut crate::ledger::Ledger<O, P, D> {
        let InvocationSlot::Dynamic(common) = &mut self.common else {
            panic!("the dynamic test hook requires its original dynamic slot");
        };
        common
            .as_mut()
            .expect("the test mutates the original installed ledger")
            .coordinator
            .retained_ledger_mut()
    }

    #[cfg(test)]
    pub(super) fn test_shared_probe(&self) -> crate::ledger::LedgerTestProbe<P, D> {
        match &self.common {
            InvocationSlot::Dynamic(common) => common
                .as_ref()
                .expect("the dynamic fixture retains its original invocation")
                .coordinator
                .retained_ledger()
                .test_shared_probe(),
            InvocationSlot::Finite(common) => common
                .as_ref()
                .expect("the finite fixture retains its original invocation")
                .coordinator
                .retained_ledger()
                .test_shared_probe()
                .expect("the finite physical fixture has real promoted storage"),
        }
    }

    pub(super) fn terminal_is_outstanding(&self) -> bool {
        self.common.is_installed() && self.common.lifecycle().terminal_outstanding
    }

    pub(super) fn into_terminal(self) -> TerminalInvocationLoan<'inv, O, P, PP, U, D> {
        self.common.into_terminal(self.contained)
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        self.common.invocation_identity()
    }
}

pub(crate) struct TerminalInvocationLoan<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    common: InvocationSlot<'inv, O, P, PP, U, D>,
    contained: &'inv mut bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'inv, O, P, PP, U, D> TerminalInvocationLoan<'inv, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn is_finite(&self) -> bool {
        matches!(self.common, InvocationSlot::Finite(_))
    }

    pub(crate) fn as_read(&self) -> crate::ledger::LedgerReadView<'_, O, P, D> {
        self.common.as_read()
    }

    pub(crate) fn view(&mut self) -> crate::ledger::LedgerView<'_, O, P, D> {
        self.common.ledger_view()
    }

    pub(crate) fn known_administrative_bytes(&self) -> Option<u64> {
        self.common.known_administrative_bytes()
    }

    pub(crate) fn retain_fail_closed(mut self) {
        *self.contained = true;
        // The actual concrete root retains its original state until physical containment.
        self.common.lifecycle_mut().terminal_outstanding = true;
    }
}

/// Actual retained state kept in the outer invocation across short permissions.
pub(super) struct RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(super) baseline: FallbackPublicationBaseline<O, P, D, F>,
    pub(super) metadata: RetainedOperationMetadata<P, D, CanonicalBlock, EvidenceBlock>,
}

/// Original plans, deferred permission and permit state, independent of baseline representation.
pub(super) struct RetainedOperationMetadata<P, D, CanonicalBlock, EvidenceBlock> {
    pub(super) deferred_stage: Option<RetainedDeferredStage<CanonicalBlock, EvidenceBlock>>,
    terminal_writer: RetainedWriterPlan<P, D>,
    canonical_assembly_limits: RetainedBatchLimits,
    terminal_account_share_work: RetainedHandleSharePlan<D>,
    failure_terminal_work: Option<FailureTerminalWorkCapability<D>>,
    failure_terminal_work_dimension: Option<D>,
    failure_work_permit_outstanding: bool,
    ordinary_work_permit_outstanding: bool,
    ordinary_work_permit_partially_resolved: bool,
}

impl<P: Copy, D: Copy, CB, EB> RetainedOperationMetadata<P, D, CB, EB> {
    fn from_qualified_plan(
        terminal_writer: RetainedWriterPlan<P, D>,
        terminal_account_share_work: RetainedHandleSharePlan<D>,
        canonical_assembly_limits: RetainedBatchLimits,
        failure_work: super::operation_start::QualifiedFailureTerminalWork<D>,
    ) -> Self {
        Self {
            deferred_stage: None,
            terminal_writer,
            canonical_assembly_limits,
            terminal_account_share_work,
            failure_terminal_work: failure_work
                .map(|(plan, _)| FailureTerminalWorkCapability::from_qualified(plan)),
            failure_terminal_work_dimension: failure_work.map(|(plan, _)| plan.dimension()),
            failure_work_permit_outstanding: false,
            ordinary_work_permit_outstanding: false,
            ordinary_work_permit_partially_resolved: false,
        }
    }
}

fn checked_terminal_assembly_limits<D>(
    limits: RetainedBatchLimits,
) -> Result<(RetainedBatchLimits, RetainedBatchLimits), RetainedOperationStartError<D>> {
    let evidence = RetainedBatchLimits::new(
        limits
            .attribution_slots
            .checked_add(1)
            .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
            .max(2),
        limits
            .commitment_slots
            .checked_add(1)
            .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
            .max(2),
    );
    Ok((limits, evidence))
}

fn check_retained_attachment_entry<O, P, PP, U, D>(
    common: &OperationInvocationView<'_, O, P, PP, U, D>,
    account: &PhysicalStorageAccount,
) -> Result<(), RetainedOperationStartError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    check_execution_state(
        &common.lifecycle.sticky_terminal_cause,
        &common.lifecycle.execution_state,
        || common.coordinator.recorded_control_cause(),
    )
    .map_err(|cause| match cause {
        ExecutionError::Control(cause) => RetainedOperationStartError::Control(cause),
        ExecutionError::Ledger(cause) => {
            RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
        }
        ExecutionError::Stopped => {
            RetainedOperationStartError::Writer(RetainedWriterError::OwnershipUnverified)
        }
    })?;
    let health = account.try_begin_account_preflight().map_err(|cause| {
        RetainedOperationStartError::Writer(RetainedWriterError::AccountPoisoned(cause))
    })?;
    drop(health);
    Ok(())
}

fn preflight_retained_failure_work<O, P, D>(
    ledger: crate::ledger::LedgerReadView<'_, O, P, D>,
    failure_work: super::operation_start::QualifiedFailureTerminalWork<D>,
) -> Result<(), RetainedOperationStartError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    if let Some((plan, requirement)) = failure_work {
        ledger
            .preflight_protected_work_attachment(
                plan.dimension(),
                &plan.share_work().charges(),
                requirement.maximum_share_actions(),
            )
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
            })?;
    }
    Ok(())
}

/// The plans remain in their original owner while the baseline lends its concrete representation.
pub(super) struct RetainedOperationFieldsLoan<'inv, O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) baseline: FallbackPublicationBaselineLoan<'inv, O, P, D, F>,
    pub(super) metadata: &'inv mut RetainedOperationMetadata<P, D, CB, EB>,
}

impl<'inv, O, P, D, F, CB, EB> RetainedOperationFieldsLoan<'inv, O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn reborrow(&mut self) -> RetainedOperationFieldsLoan<'_, O, P, D, F, CB, EB> {
        RetainedOperationFieldsLoan {
            baseline: match &mut self.baseline {
                FallbackPublicationBaselineLoan::Dynamic(baseline) => {
                    FallbackPublicationBaselineLoan::Dynamic(baseline)
                }
                FallbackPublicationBaselineLoan::Finite(baseline) => {
                    FallbackPublicationBaselineLoan::Finite(baseline)
                }
            },
            metadata: self.metadata,
        }
    }

    fn has_deferred_storage(&self) -> bool {
        self.baseline.first_account().is_some() && self.baseline.has_batches()
    }
}

fn reject_retained_attachment<O, P, PP, U, D>(
    common: &mut OperationInvocationView<'_, O, P, PP, U, D>,
    cause: PreparationFailureCode,
) where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    StoreExecutionState::stop(
        &common.lifecycle.execution_state,
        StoreExecutionState::KnownError,
    );
    record_terminal_cause_with_precedence(&common.lifecycle.sticky_terminal_cause, cause);
    if common
        .coordinator
        .ledger_mut()
        .seal_ordinary_work()
        .is_err()
    {
        record_terminal_cause_with_precedence(
            &common.lifecycle.sticky_terminal_cause,
            PreparationFailureCode::ledger(),
        );
    }
}

/// Disposes or quarantines the actual borrowed graph; only the concrete root takes owners.
fn cleanup_retained_root_graph<O, P, PP, U, D, F, CB, EB>(
    mut retained: Option<RetainedOperationFieldsLoan<'_, O, P, D, F, CB, EB>>,
    assembly: &mut super::PublicationAssemblySlot<O, P, D, CB, EB>,
    mut terminal: TerminalPhysicalSlot<'_, P, D, F, CB, EB>,
    mut common: Option<InvocationSlot<'_, O, P, PP, U, D>>,
    contained: &mut bool,
) -> bool
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    let installed = common.as_ref().is_some_and(InvocationSlot::is_installed);
    let quarantine = *contained
        || common
            .as_ref()
            .is_some_and(|common| common.is_installed() && common.lifecycle().terminal_outstanding);
    if quarantine {
        *contained = true;
        if let Some(common) = common.as_mut().filter(|common| common.is_installed()) {
            let view = common.reborrow().into_view();
            StoreExecutionState::stop(
                &view.lifecycle.execution_state,
                StoreExecutionState::KnownError,
            );
            record_terminal_cause_with_precedence(
                &view.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::product_graph(StoreExecutionState::ABANDONED),
            );
        }
    }
    if let Some(fields) = assembly.as_mut()
        && !fields.is_empty()
        && (quarantine || (retained.is_some() && installed))
    {
        let operation = if let Some(common) = common.as_mut().filter(|common| common.is_installed())
        {
            retained.as_mut().map(|retained| RetainedOperationLoan {
                retained: retained.reborrow(),
                assembly: None,
                terminal: match &mut terminal {
                    TerminalPhysicalSlot::Dynamic(slot) => TerminalPhysicalSlot::Dynamic(slot),
                    TerminalPhysicalSlot::Finite(slot) => TerminalPhysicalSlot::Finite(slot),
                },
                common: InvocationLoan {
                    common: common.reborrow(),
                    contained,
                    not_send_or_sync: PhantomData,
                },
            })
        } else {
            None
        };
        let mut loan = super::RetainedPublicationAssemblyLoan {
            fields: fields.into(),
            operation,
        };
        if quarantine {
            if loan.first_containment_account().is_some() {
                let _ = loan.poison_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            } else {
                loan.fail_closed_after_recorded_poison();
            }
        }
        // Healthy pre-Ready destruction uses the existing exact graph/destructor policy,
        // including verified physical destruction on an already poisoned account.
        drop(loan);
    }
    if quarantine && let Some(retained) = retained.as_mut() {
        if let Some(account) = retained.baseline.first_account() {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        retained.baseline.record_forgotten_physical_edges();
        retained.baseline.fail_closed_after_recorded_poison();
    }
    quarantine
}

impl<'inv, O, P, D, F, CB, EB> From<&'inv mut RetainedOperationFields<O, P, D, F, CB, EB>>
    for RetainedOperationFieldsLoan<'inv, O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    fn from(fields: &'inv mut RetainedOperationFields<O, P, D, F, CB, EB>) -> Self {
        Self {
            baseline: FallbackPublicationBaselineLoan::Dynamic(&mut fields.baseline),
            metadata: &mut fields.metadata,
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    fn has_deferred_storage(&self) -> bool {
        self.baseline.first_account().is_some() && self.baseline.with_batches(|_| ()).is_some()
    }
}

/// Concrete dynamic permission; consuming result types remain dynamic.
#[must_use = "the operation permission must be resolved by its invocation"]
pub(crate) struct RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(super) loan: RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(super) fn from_loan(
        loan: RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self { loan }
    }

    pub(super) fn into_loan(
        self,
    ) -> RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.loan
    }

    pub(crate) fn loan(
        &self,
    ) -> &RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        &self.loan
    }

    pub(crate) fn loan_mut(
        &mut self,
    ) -> &mut RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        &mut self.loan
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub fn try_start(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        let phase_catalog = [plan.terminal_writer.phase];
        Self::try_start_inner(
            root,
            control,
            (ledger_limits, &phase_catalog),
            account,
            plan,
            CanonicalLineageAuthority::fresh(),
            build_fallback,
        )
    }

    pub fn try_start_with_phase_catalog(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        Self::try_start_inner(
            root,
            control,
            ledger,
            account,
            plan,
            CanonicalLineageAuthority::fresh(),
            build_fallback,
        )
    }

    pub(crate) fn try_start_derived_with_phase_catalog<SourceBlock>(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &PhysicalStorageAccount,
        source: &PublishedCanonicalAggregate<SourceBlock>,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        let preflight = account.try_begin_account_preflight().map_err(|cause| {
            RetainedOperationStartError::Writer(RetainedWriterError::AccountPoisoned(cause))
        })?;
        if !source.belongs_to(account) {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        let canonical_lineage = CanonicalLineageAuthority::derived(source);
        drop(preflight);
        Self::try_start_inner(
            root,
            control,
            ledger,
            account,
            plan,
            canonical_lineage,
            build_fallback,
        )
    }

    fn try_start_inner(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        canonical_lineage: CanonicalLineageAuthority,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        Self::bind_inner(root, control, ledger, account, plan, canonical_lineage)?
            .attach(build_fallback)
    }

    pub(crate) fn bind_with_phase_catalog<'account>(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &'account PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
    ) -> RetainedAttachmentResult<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        Self::bind_inner(
            root,
            control,
            ledger,
            account,
            plan,
            CanonicalLineageAuthority::fresh(),
        )
    }

    pub(crate) fn bind_derived_with_phase_catalog<'account, SourceBlock>(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &'account PhysicalStorageAccount,
        source: &PublishedCanonicalAggregate<SourceBlock>,
        plan: RetainedOperationStartPlan<P, D>,
    ) -> RetainedAttachmentResult<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        let preflight = account.try_begin_account_preflight().map_err(|cause| {
            RetainedOperationStartError::Writer(RetainedWriterError::AccountPoisoned(cause))
        })?;
        if !source.belongs_to(account) {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        let lineage = CanonicalLineageAuthority::derived(source);
        drop(preflight);
        Self::bind_inner(root, control, ledger, account, plan, lineage)
    }

    fn bind_inner<'account>(
        root: &'inv mut InvocationRoot<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        control: OperationControl<PP, U>,
        ledger: (LedgerLimits<D>, &[P]),
        account: &'account PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        canonical_lineage: CanonicalLineageAuthority,
    ) -> RetainedAttachmentResult<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    {
        let (ledger_limits, phase_catalog) = ledger;
        if root.started {
            return Err(RetainedOperationStartError::InvocationAlreadyStarted);
        }
        root.started = true;
        let failure_terminal_work = Self::qualify_terminal_plan(&plan)?;
        let dimension_count = ledger_limits.dimension_count();
        let phase_count = phase_catalog.len();
        let admission = account.try_admit_operation_start().map_err(|cause| {
            RetainedOperationStartError::Writer(match cause {
                startup::OperationStartAdmissionError::AccountPoisoned(poisoned) => {
                    RetainedWriterError::AccountPoisoned(poisoned)
                }
                startup::OperationStartAdmissionError::AccountingOverflow => {
                    RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow)
                }
            })
        })?;
        let coordinator = OperationCoordinator::try_new(control, ledger_limits, phase_catalog)
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
            })?;
        root.common = Some(OperationInvocation {
            coordinator,
            lifecycle: InvocationLifecycle::new(canonical_lineage),
        });
        let common = root
            .common
            .as_mut()
            .expect("binding installs its actual common");
        if !super::terminal_evidence_catalog_is_qualified(
            common.coordinator.retained_ledger(),
            dimension_count,
            phase_count,
        ) {
            root.record_payload_failure(0x4154_4341);
            return Err(RetainedOperationStartError::TerminalEvidenceCatalogInvariant);
        }
        if let Some((failure_plan, _)) = failure_terminal_work {
            common
                .view()
                .coordinator
                .ledger_mut()
                .configure_protected_work(failure_plan.dimension())
                .map_err(|cause| {
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
                })?;
        }
        Ok(PendingRetainedAttachment {
            root,
            admission,
            plan,
            failure_terminal_work,
            dimension_count,
            phase_count,
            local: PhantomData,
        })
    }

    fn qualify_terminal_plan(
        plan: &RetainedOperationStartPlan<P, D>,
    ) -> Result<
        Option<(
            RetainedFailureTerminalWorkPlan<D>,
            RetainedFailureTerminalWorkRequirement,
        )>,
        RetainedOperationStartError<D>,
    > {
        super::operation_start::qualify_terminal_plan(
            plan.terminal_writer,
            plan.terminal_account_share_work,
            &[plan.fallback_commitment, plan.snapshot_commitment],
            plan.failure_terminal_work,
        )
    }

    fn prepare_attachment(
        common: &mut OperationInvocation<O, P, PP, U, D>,
        admission: startup::PendingOperationStartAdmission<'_>,
        plan: RetainedOperationStartPlan<P, D>,
        failure_terminal_work: Option<(
            RetainedFailureTerminalWorkPlan<D>,
            RetainedFailureTerminalWorkRequirement,
        )>,
        dimension_count: usize,
        phase_count: usize,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> RetainedAttachmentFieldsResult<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let account = admission.account();
        check_retained_attachment_entry(&common.view(), account)?;
        let ledger = common.coordinator.retained_ledger();
        let (canonical_assembly, evidence_assembly) =
            checked_terminal_assembly_limits(plan.terminal_writer.limits)?;
        let terminal_slot_plan = RetainedWriterPlan::new(
            plan.terminal_writer.phase,
            plan.terminal_writer.capacity_dimension,
            plan.terminal_writer.attribution_dimension,
            RetainedBatchLimits::new(1, 1),
        );
        let fallback_batch_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                terminal_slot_plan.phase,
                terminal_slot_plan.capacity_dimension,
                terminal_slot_plan.limits.attribution_slots,
                terminal_slot_plan.limits.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let evidence_batch_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                terminal_slot_plan.phase,
                terminal_slot_plan.capacity_dimension,
                terminal_slot_plan.limits.attribution_slots,
                terminal_slot_plan.limits.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let canonical_assembly_plan = AttributionBatchPlan::<O, P, D, CanonicalOutput>::try_new(
            plan.terminal_writer.phase,
            plan.terminal_writer.capacity_dimension,
            canonical_assembly.attribution_slots,
            canonical_assembly.commitment_slots,
        )
        .map_err(map_batch_error)?;
        let evidence_assembly_plan =
            AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
                plan.terminal_writer.phase,
                plan.terminal_writer.capacity_dimension,
                evidence_assembly.attribution_slots,
                evidence_assembly.commitment_slots,
            )
            .map_err(map_batch_error)?;
        let terminal_evidence_backing =
            TerminalEvidenceBackingPlan::<P, D>::try_new(dimension_count, phase_count).map_err(
                |_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity),
            )?;
        let terminal_evidence_footprint = terminal_evidence_backing.footprint();
        let evidence_plan = RetainedAllocationPlan::new(
            terminal_evidence_footprint.bytes(),
            terminal_evidence_footprint.allocations(),
        )
        .map_err(|_| RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity))?;
        let fallback_charge = account
            .checked_retained_charge::<F>(plan.fallback_plan)
            .map_err(RetainedWriterError::Physical)?;
        let evidence_charge = account
            .checked_retained_charge::<TerminalLedgerEvidence<P, D>>(evidence_plan)
            .map_err(RetainedWriterError::Physical)?;
        let startup_parts = StartupLedgerPlanParts {
            batches: StartupAttributionBatchPlans::new(
                canonical_assembly_plan,
                evidence_assembly_plan,
                fallback_batch_plan,
                evidence_batch_plan,
            ),
            retained_attributions: [
                StartupRetainedAttribution::new(
                    terminal_slot_plan.phase,
                    terminal_slot_plan.attribution_dimension,
                    fallback_charge.bytes,
                ),
                StartupRetainedAttribution::new(
                    terminal_slot_plan.phase,
                    terminal_slot_plan.attribution_dimension,
                    evidence_charge.bytes,
                ),
            ],
            account_shares: StartupAccountShareEnvelope::four_identical(
                plan.terminal_account_share_work.charges(),
            ),
            commitments: [plan.fallback_commitment, plan.snapshot_commitment],
        };
        let physical_manifest = StartupPhysicalManifest::try_new(fallback_charge, evidence_charge)
            .map_err(|_| {
                RetainedOperationStartError::Writer(RetainedWriterError::Physical(
                    RetainedAllocationError::AccountingOverflow,
                ))
            })?;
        let cross_plane = StartupCrossPlanePlan::try_new(startup_parts, physical_manifest)
            .map_err(|_| {
                RetainedOperationStartError::Writer(RetainedWriterError::OwnershipUnverified)
            })?;
        preflight_retained_failure_work(ledger.as_read(), failure_terminal_work)?;
        let ready = {
            let (_, _, ledger) = common.coordinator.split_exclusive_progress_and_ledger();
            admission.try_prepare_startup_cross_plane(ledger, cross_plane)
        }
        .map_err(map_startup_cross_plane_error)?;
        #[cfg(test)]
        STARTUP_READY_FINAL_OWNER_ALLOCATOR_ENTRIES.with(|entries| {
            entries.set(Some(test_retained_final_owner_allocator_entries()));
        });
        let (baseline, start_guard) = ready.split();
        let startup_guard =
            StartupRetainedPartGuard::new(start_guard, baseline, plan.fallback_plan, evidence_plan);
        // The protected dimension was qualified before graph effects. These four disjoint
        // ordinary share steps cannot consume or change that dormant pool.
        let ledger = common.coordinator.retained_ledger();
        let startup_owner_result = startup_guard.try_build(build_fallback, || {
            #[cfg(test)]
            {
                record_start_evidence_build_call();
                if take_injected_start_evidence_build_panic() {
                    panic!("inject startup evidence builder panic");
                }
                if let Some(cause) = take_injected_start_evidence_build_failure() {
                    return Err(StartupTerminalEvidenceBuildError::Rejected(cause));
                }
                if take_injected_start_evidence_structural_failure() {
                    return Err(StartupTerminalEvidenceBuildError::StructuralInvariant);
                }
            }
            terminal_evidence_backing
                .try_build_indexed(
                    |index| ledger.dimension_at(index),
                    |index| ledger.phase_at(index),
                )
                .map_err(StartupTerminalEvidenceBuildError::from)
        });
        #[cfg(test)]
        let ledger = {
            if startup_owner_result.is_err() {
                let probe = common.coordinator.retained_ledger_mut().test_probe();
                STARTUP_FAILURE_LEDGER_SNAPSHOT.with(|snapshot| {
                    snapshot.set(Some((
                        probe.active_reservations(),
                        probe.current_live_bytes(),
                    )));
                });
            }
            common.coordinator.retained_ledger()
        };
        let startup_owner = startup_owner_result.map_err(RetainedOperationStartError::Writer)?;
        let baseline = startup_owner
            .try_into_baseline()
            .map_err(RetainedOperationStartError::Writer)?;
        let baseline = baseline
            .try_attach(ledger)
            .map_err(RetainedOperationStartError::Writer)?;
        Ok(RetainedOperationFields {
            baseline,
            metadata: RetainedOperationMetadata::from_qualified_plan(
                plan.terminal_writer,
                plan.terminal_account_share_work,
                canonical_assembly,
                failure_terminal_work,
            ),
        })
    }
    pub(crate) fn defer_until_body_cleanup(self) -> Result<(), Self> {
        self.loan
            .defer_until_body_cleanup()
            .map_err(Self::from_loan)
    }

    pub fn try_publication_assembly(
        self,
        plan: RetainedPublicationPlan<P, D>,
    ) -> RetainedAssemblyOpenResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        match self.loan.try_publication_assembly(plan) {
            Ok(loan) => Ok(RetainedPublicationAssembly::from_loan(loan)),
            Err(RetainedPublicationOpenError {
                operation,
                cause,
                disposition,
            }) => Err(RetainedPublicationOpenError {
                operation: Self::from_loan(operation),
                cause,
                disposition,
            }),
        }
    }

    /// Freezes a successful terminal ledger without publishing canonical or domain evidence tables.
    #[must_use = "the prepared evidence-only outcome must be handed off or deliberately dropped"]
    pub fn prepare_evidence_only_success(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedRetainedEvidenceOnlyOutcome<F, P, D>, RetainedPublicationPrepareError> {
        DynamicTerminalRootFields::from_operation(self).finish_evidence_only(final_progress)
    }

    /// Freezes the pre-admitted terminal baseline without opening a block-table assembly.
    ///
    /// Recoverable assembly-open failures retain this baseline. Remaining control owners close
    /// before physical transfer; terminal qualification still reports containment when ownership
    /// cannot be verified. Fail-closed open errors do not return this operation permission.
    #[must_use = "the prepared fallback must be handed off or deliberately dropped"]
    pub fn prepare_minimal_fallback(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        DynamicTerminalRootFields::from_operation(self).finish_minimal(cause)
    }
}

/// Short permission over the invocation and its actual retained state.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[must_use = "the operation permission must be resolved by its invocation"]
pub(crate) struct RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(super) terminal: TerminalPhysicalSlot<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    pub(super) retained:
        RetainedOperationFieldsLoan<'inv, O, P, D, F, CanonicalBlock, EvidenceBlock>,
    assembly:
        Option<&'inv mut super::PublicationAssemblySlot<O, P, D, CanonicalBlock, EvidenceBlock>>,
    pub(super) common: InvocationLoan<'inv, O, P, PP, U, D>,
}

impl<O, P, PP, U, D, F, CB, EB> RetainedOperationLoan<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(crate) fn reborrow(&mut self) -> RetainedOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        RetainedOperationLoan {
            retained: self.retained.reborrow(),
            terminal: match &mut self.terminal {
                TerminalPhysicalSlot::Dynamic(slot) => TerminalPhysicalSlot::Dynamic(slot),
                TerminalPhysicalSlot::Finite(slot) => TerminalPhysicalSlot::Finite(slot),
            },
            assembly: self.assembly.as_deref_mut(),
            common: self.common.reborrow(),
        }
    }
}

/// Allocation-free operation ledger state used to prove scoped planning cleanup.
///
/// This deliberately exposes neither the ledger nor any phase/dimension key. The initial snapshot
/// is the planning baseline; open-time snapshots derive the exact state that may remain owned by
/// canonical and evidence publication storage.
#[derive(Clone, Copy)]
pub(crate) struct RetainedPlanningBaseline {
    current_live_bytes: u64,
    active_reservations: u64,
}

impl RetainedPlanningBaseline {
    pub(crate) fn checked_apply_rebind_delta(self, before: Self, after: Self) -> Option<Self> {
        Some(Self {
            current_live_bytes: checked_apply_delta(
                self.current_live_bytes,
                before.current_live_bytes,
                after.current_live_bytes,
            )?,
            active_reservations: checked_apply_delta(
                self.active_reservations,
                before.active_reservations,
                after.active_reservations,
            )?,
        })
    }

    fn checked_add_publication_storage(
        self,
        publication_live_bytes: u64,
        publication_reservations: u64,
    ) -> Option<Self> {
        Some(Self {
            current_live_bytes: self
                .current_live_bytes
                .checked_add(publication_live_bytes)?,
            active_reservations: self
                .active_reservations
                .checked_add(publication_reservations)?,
        })
    }
}

fn checked_apply_delta(value: u64, before: u64, after: u64) -> Option<u64> {
    if after >= before {
        value.checked_add(after - before)
    } else {
        value.checked_sub(before - after)
    }
}

/// Poison-only account anchor retained while an activation owner changes representation.
///
/// The anchor cannot reserve, publish, or expose the account. Its sole purpose is to turn an
/// owner-consuming unwind into typed containment while reusing the account's first poison evidence.
pub(crate) struct RetainedPlanningContainmentAnchor {
    account: PhysicalStorageAccount,
}

impl RetainedPlanningContainmentAnchor {
    pub(crate) fn existing_containment(&self) -> Option<FinalizationContainmentError> {
        self.account.snapshot().poisoned().then(|| self.contain())
    }
    pub(crate) fn share(&self) -> Self {
        Self {
            account: PhysicalStorageAccount {
                inner: self.account.inner.share(),
            },
        }
    }

    pub(crate) fn contain(&self) -> FinalizationContainmentError {
        let snapshot = self.account.snapshot();
        let reason = snapshot
            .first_poison_evidence()
            .map_or(AccountPoisonReason::OwnershipGraphUnverified, |evidence| {
                evidence.reason()
            });
        let account = if snapshot.poisoned() {
            AccountPoisoned::new(snapshot)
        } else {
            self.account.record_poison(reason)
        };
        FinalizationContainmentError::new(account, reason)
    }
}

/// Start-qualified, finite, single-use authority for terminal failure evidence.
///
/// A success-capable assembly retains this authority until success becomes irreversible or the
/// candidate is downgraded. Ordinary control, writer, and commitment APIs cannot address its
/// protected ledger dimension.
struct FailureTerminalWorkCapability<D> {
    handle_share_work: RetainedHandleSharePlan<D>,
    max_evidence_members: usize,
    construction_work_limit: u64,
    validation_work_limit: u64,
    validation_work_qualified: bool,
}

/// Records the first trusted terminal cause while keeping a requested cause provisional.
///
/// A requested semantic cause remains authoritative only until the operation observes its first
/// trusted runtime failure. Once Control, ProductGraph, or Ledger is recorded, later causes cannot
/// replace it.
pub(crate) fn record_terminal_cause_with_precedence(
    slot: &Cell<Option<PreparationFailureCode>>,
    cause: PreparationFailureCode,
) {
    match slot.get() {
        None => slot.set(Some(cause)),
        Some(existing)
            if existing.class() == PreparationFailureClass::Requested
                && cause.class() != PreparationFailureClass::Requested =>
        {
            slot.set(Some(cause));
        }
        Some(_) => {}
    }
}

impl<D: Copy> FailureTerminalWorkCapability<D> {
    fn from_qualified(plan: RetainedFailureTerminalWorkPlan<D>) -> Self {
        Self {
            handle_share_work: plan.share_work(),
            max_evidence_members: plan.max_evidence_members(),
            construction_work_limit: plan.construction_work_limit(),
            validation_work_limit: plan.validation_work_limit(),
            validation_work_qualified: false,
        }
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(super) fn assert_backend(&self) {
        let finite = matches!(&self.common.common, InvocationSlot::Finite(_));
        assert_eq!(
            self.retained.baseline.is_finite(),
            finite,
            "the original common and baseline backends must match"
        );
        assert_eq!(
            self.terminal.is_finite(),
            finite,
            "the original common and terminal backends must match"
        );
    }

    pub(super) fn prepare_body_deferral(&mut self) -> Option<Result<(), LedgerCause<D>>> {
        if self.retained.metadata.deferred_stage.is_some()
            || !self.retained.has_deferred_storage()
            || self.retained.metadata.failure_work_permit_outstanding
            || self.retained.metadata.ordinary_work_permit_outstanding
        {
            return None;
        }
        if !self.common.common.is_installed() {
            return None;
        }
        Some(
            self.common
                .view()
                .coordinator
                .ledger_mut()
                .seal_ordinary_work(),
        )
    }

    pub(crate) fn defer_until_body_cleanup(mut self) -> Result<(), Self> {
        if !self.assembly.as_ref().is_some_and(|slot| slot.is_none())
            || !matches!(self.prepare_body_deferral(), Some(Ok(())))
        {
            return Err(self);
        }
        self.retained.metadata.deferred_stage = Some(RetainedDeferredStage::Operation);
        Ok(())
    }

    pub(crate) fn exclusive_parts(
        &mut self,
    ) -> Result<ExclusiveOperationParts<'_, O, P, PP, U, D>, ExecutionError<D>>
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey,
    {
        self.common.preflight_exclusive()?;
        if self.is_terminal_only() {
            return Err(ExecutionError::Stopped);
        }
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            baseline,
            metadata:
                RetainedOperationMetadata {
                    terminal_account_share_work,
                    ..
                },
            ..
        } = &mut *retained;
        common
            .view()
            .into_exclusive_parts(baseline.account(), *terminal_account_share_work)
    }

    pub(crate) fn check_execution_running(&self) -> Result<(), ExecutionError<D>> {
        check_execution_state(
            &self.common.lifecycle().sticky_terminal_cause,
            &self.common.lifecycle().execution_state,
            || self.common.recorded_control_cause(),
        )
    }

    pub(crate) fn settle_importer_return(&self) {
        StoreExecutionState::settle_caught_unwind(
            &self.common.lifecycle().execution_state,
            &self.common.lifecycle().sticky_terminal_cause,
        );
    }

    pub(crate) fn record_terminal_cause(&mut self, cause: PreparationFailureCode) {
        record_terminal_cause_with_precedence(
            &self.common.lifecycle().sticky_terminal_cause,
            cause,
        );
    }

    pub(super) fn stop_for_containment(&mut self) {
        StoreExecutionState::stop(
            &self.common.lifecycle().execution_state,
            StoreExecutionState::KnownError,
        );
        if self
            .common
            .view()
            .coordinator
            .ledger_mut()
            .seal_ordinary_work()
            .is_err()
        {
            self.record_terminal_cause(PreparationFailureCode::ledger());
        }
    }

    pub(crate) fn poll_terminal_control(&mut self) {
        let cause = self
            .common
            .view()
            .coordinator
            .poll()
            .err()
            .map(|cause| PreparationFailureCode::from_control(&cause));
        if let Some(cause) = cause {
            self.record_terminal_cause(cause);
        }
    }

    pub(crate) fn planning_baseline(&self) -> RetainedPlanningBaseline {
        let state = self.common.as_read().planning_state();
        RetainedPlanningBaseline {
            current_live_bytes: state.current_live_bytes(),
            active_reservations: state.active_reservations(),
        }
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        self.common.invocation_identity()
    }

    pub(crate) fn planning_containment_anchor(&self) -> RetainedPlanningContainmentAnchor {
        RetainedPlanningContainmentAnchor {
            account: PhysicalStorageAccount {
                inner: self.retained.baseline.account().inner.share(),
            },
        }
    }

    fn planning_has_outstanding_work_permit(&self) -> bool {
        self.retained.metadata.failure_work_permit_outstanding
            || self.retained.metadata.ordinary_work_permit_outstanding
            || self.common.as_read().check_work_quiescent().is_err()
    }

    pub(crate) fn planning_matches_baseline(&self, baseline: RetainedPlanningBaseline) -> bool {
        if self.planning_has_outstanding_work_permit() {
            return false;
        }
        let state = self.common.as_read().planning_state();
        state.current_live_bytes() == baseline.current_live_bytes
            && state.active_reservations() == baseline.active_reservations
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    super::RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    /// Derives the exact ledger state allowed after materialization releases every scratch owner.
    ///
    /// Publication attributions are structurally owned by the two batches. Summing those batches
    /// admits table roots and materialized members while keeping unrelated scoped reservations out
    /// of the baseline.
    pub(crate) fn publication_storage_baseline(
        &self,
        planning_baseline: RetainedPlanningBaseline,
    ) -> Option<RetainedPlanningBaseline> {
        let canonical = self.fields.canonical_batch.as_ref()?;
        let evidence = self.fields.evidence_batch.as_ref()?;
        let publication_live_bytes = canonical
            .live_bytes()?
            .checked_add(evidence.live_bytes()?)?;
        let publication_reservations = canonical
            .active_count()?
            .checked_add(evidence.active_count()?)?;
        planning_baseline
            .checked_add_publication_storage(publication_live_bytes, publication_reservations)
    }

    pub(crate) fn planning_matches_baseline(&self, baseline: RetainedPlanningBaseline) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.planning_matches_baseline(baseline))
    }
}

/// Borrowed control-and-ledger capability for retained producers.
///
/// This facade deliberately excludes progress reporting, publication, terminal evidence, and
/// retained-storage mutation.
pub(crate) struct RetainedControlCheckpoint<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    poll: OperationPollCheckpoint<'a, PP, U>,
    ledger: crate::ledger::LedgerView<'a, O, P, D>,
    failure_terminal_work_dimension: Option<D>,
    sticky_terminal_cause: &'a Cell<Option<PreparationFailureCode>>,
}

/// Borrowed host authority for producer work before publication begins.
///
/// This capability can report semantic progress, inspect narrow scalar accounting, tighten policy,
/// and admit one linear observed-work envelope. It exposes neither raw ledger ownership nor any
/// publication, retained-table, or terminal-evidence authority.
pub(crate) struct RetainedProducerHost<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    coordinator: CoordinatorView<'a, O, P, PP, U, D>,
    failure_terminal_work_dimension: Option<D>,
    permit_outstanding: &'a mut bool,
    sticky_terminal_cause: &'a Cell<Option<PreparationFailureCode>>,
}

pub(crate) enum RetainedProducerWorkStartError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Outstanding,
}

impl<O, P, PP, U, D> RetainedProducerHost<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        let result = self.coordinator.poll();
        if let Err(cause) = &result {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::from_control(cause),
            );
        }
        result
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: PP,
        unit: U,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ControlCause> {
        let result = self
            .coordinator
            .report_progress(phase, unit, completed, total);
        if let Err(cause) = &result {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::from_control(cause),
            );
        }
        result
    }

    pub(crate) fn dimension_evidence(
        &mut self,
        dimension: D,
    ) -> Result<DimensionSnapshot, LedgerCause<D>> {
        self.validate_dimension(dimension)?;
        let result = self.coordinator.ledger_read().dimension_evidence(dimension);
        if result.is_err() {
            self.record_ledger_failure();
        }
        result
    }

    pub(crate) fn tighten_dimension_policy(
        &mut self,
        dimension: D,
        upper_bound: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_dimension(dimension)?;
        let result = self
            .coordinator
            .ledger_mut()
            .tighten_dimension_policy(dimension, upper_bound);
        if result.is_err() {
            self.record_ledger_failure();
        }
        result
    }

    pub(crate) fn charge_attempted(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedProducerWorkStartError<D>> {
        self.ensure_direct_accounting_available()?;
        let result = self.coordinator.ledger_mut().charge_producer_attempted(
            dimension,
            amount,
            self.failure_terminal_work_dimension,
        );
        if result.is_err() {
            self.record_ledger_failure();
        }
        result.map_err(RetainedProducerWorkStartError::Ledger)
    }

    pub(crate) fn charge_consumed(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), RetainedProducerWorkStartError<D>> {
        self.ensure_direct_accounting_available()?;
        self.validate_dimension(dimension)
            .map_err(RetainedProducerWorkStartError::Ledger)?;
        let result = self
            .coordinator
            .ledger_mut()
            .charge_consumed(dimension, amount);
        if result.is_err() {
            self.record_ledger_failure();
        }
        result.map_err(RetainedProducerWorkStartError::Ledger)
    }

    pub(crate) fn charge_observed_batch(
        &mut self,
        charges: &[(D, u64)],
    ) -> Result<(), RetainedProducerWorkStartError<D>> {
        self.ensure_direct_accounting_available()?;
        for &(dimension, _) in charges {
            self.validate_dimension(dimension)
                .map_err(RetainedProducerWorkStartError::Ledger)?;
        }
        let result = self.coordinator.ledger_mut().charge_observed_batch(charges);
        if result.is_err() {
            self.record_ledger_failure();
        }
        result.map_err(RetainedProducerWorkStartError::Ledger)
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
    ) -> Result<(), RetainedProducerWorkStartError<D>> {
        self.ensure_direct_accounting_available()?;
        let result = self
            .coordinator
            .ledger_mut()
            .charge_producer_consumed_batch(charges, self.failure_terminal_work_dimension);
        if result.is_err() {
            self.record_ledger_failure();
        }
        result.map_err(RetainedProducerWorkStartError::Ledger)
    }

    pub(crate) fn record_ledger_failure(&mut self) {
        record_terminal_cause_with_precedence(
            self.sticky_terminal_cause,
            PreparationFailureCode::ledger(),
        );
    }

    fn ensure_direct_accounting_available(
        &mut self,
    ) -> Result<(), RetainedProducerWorkStartError<D>> {
        if *self.permit_outstanding {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            Err(RetainedProducerWorkStartError::Outstanding)
        } else {
            Ok(())
        }
    }

    fn validate_dimension(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        if self.failure_terminal_work_dimension != Some(dimension)
            && self.coordinator.ledger_read().contains_dimension(dimension)
        {
            Ok(())
        } else {
            self.record_ledger_failure();
            Err(LedgerCause::UnknownDimension { dimension })
        }
    }
}

impl<O, P, PP, U, D> RetainedControlCheckpoint<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.poll.poll().inspect_err(|cause| {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::from_control(cause),
            );
        })
    }

    pub(crate) fn contains_phase(&self, phase: P) -> bool {
        self.ledger.as_read().contains_phase(phase)
    }

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        self.failure_terminal_work_dimension != Some(dimension)
            && self.ledger.as_read().contains_dimension(dimension)
    }

    pub(crate) fn charge_observed(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_dimension(dimension)?;
        self.ledger
            .charge_observed_batch(&[(dimension, amount)])
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    pub(crate) fn reserve(
        &mut self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.validate_dimension(dimension)?;
        self.ledger
            .reserve(phase, dimension, bytes)
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    pub(crate) fn resize(
        &mut self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger
            .resize_reservation(reservation, bytes)
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    pub(crate) fn transition(
        &mut self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_dimension(dimension)?;
        self.ledger
            .transition_reservation(reservation, phase, dimension, bytes)
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    pub(crate) fn validate_live_owner(
        &mut self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.ledger
            .validate_live_reservation(reservation)
            .inspect_err(|_| {
                record_terminal_cause_with_precedence(
                    self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
            })
    }

    fn validate_dimension(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        if self.contains_dimension(dimension) {
            Ok(())
        } else {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            Err(LedgerCause::UnknownDimension { dimension })
        }
    }
}

#[allow(
    dead_code,
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "operation startup preserves every admitted owner on rejection without allocating"
)]
impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedOperationLoan<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CanonicalBlock: Send + Sync + 'static,
    EvidenceBlock: Send + Sync + 'static,
{
    pub(crate) fn control_checkpoint(&mut self) -> RetainedControlCheckpoint<'_, O, P, PP, U, D> {
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            metadata:
                RetainedOperationMetadata {
                    failure_terminal_work_dimension,
                    ..
                },
            ..
        } = &mut *retained;
        let OperationInvocationView {
            coordinator,
            lifecycle,
        } = common.view();
        let sticky_terminal_cause = &lifecycle.sticky_terminal_cause;
        let (poll, ledger) = coordinator.into_poll_and_ledger();
        RetainedControlCheckpoint {
            poll,
            ledger,
            failure_terminal_work_dimension: *failure_terminal_work_dimension,
            sticky_terminal_cause,
        }
    }

    pub(crate) fn producer_host(&mut self) -> RetainedProducerHost<'_, O, P, PP, U, D> {
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            metadata:
                RetainedOperationMetadata {
                    failure_terminal_work_dimension,
                    ordinary_work_permit_outstanding,
                    ..
                },
            ..
        } = &mut *retained;
        let OperationInvocationView {
            coordinator,
            lifecycle,
        } = common.view();
        let sticky_terminal_cause = &lifecycle.sticky_terminal_cause;
        RetainedProducerHost {
            coordinator,
            failure_terminal_work_dimension: *failure_terminal_work_dimension,
            permit_outstanding: ordinary_work_permit_outstanding,
            sticky_terminal_cause,
        }
    }

    pub(crate) fn internal_work_root(&mut self) -> RetainedWorkRoot<'_, O, P, D> {
        let (_, _, ledger) = self
            .common
            .view()
            .coordinator
            .into_exclusive_progress_and_ledger();
        let (work, storage) = ledger.into_internal();
        RetainedWorkRoot { work, storage }
    }

    pub(crate) fn split_work_control(
        &mut self,
        dimension: D,
    ) -> (
        RetainedWorkBinding<'_, PP, U, D>,
        RetainedWorkRoot<'_, O, P, D>,
    ) {
        self.split_work_and_reservation_control(dimension, dimension)
    }

    pub(crate) fn split_work_and_reservation_control(
        &mut self,
        work_dimension: D,
        reservation_dimension: D,
    ) -> (
        RetainedWorkBinding<'_, PP, U, D>,
        RetainedWorkRoot<'_, O, P, D>,
    ) {
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            metadata:
                RetainedOperationMetadata {
                    failure_terminal_work_dimension,
                    ordinary_work_permit_outstanding,
                    ordinary_work_permit_partially_resolved,
                    ..
                },
            ..
        } = &mut *retained;
        let OperationInvocationView {
            coordinator,
            lifecycle,
        } = common.view();
        let sticky_terminal_cause = &lifecycle.sticky_terminal_cause;
        let (poll, _, ledger) = coordinator.into_exclusive_progress_and_ledger();
        let (work, storage) = ledger.into_internal();
        let binding = if *failure_terminal_work_dimension == Some(work_dimension) {
            Err(LedgerCause::UnknownDimension {
                dimension: work_dimension,
            })
        } else {
            work.bind_ordinary([work_dimension])
        };
        (
            RetainedWorkBinding::ordinary(
                poll,
                work_dimension,
                reservation_dimension,
                binding,
                ordinary_work_permit_outstanding,
                ordinary_work_permit_partially_resolved,
                sticky_terminal_cause,
            ),
            RetainedWorkRoot { work, storage },
        )
    }

    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.retained.metadata.failure_terminal_work_dimension != Some(dimension)
            && self.common.as_read().contains_dimension(dimension)
    }

    pub fn contains_phase(&self, phase: P) -> bool {
        self.common.as_read().contains_phase(phase)
    }

    pub(crate) fn require_canonical_source<SourceBlock>(
        &self,
        source: &PublishedCanonicalAggregate<SourceBlock>,
    ) -> Result<(), super::RetainedPublicationError<D>> {
        self.common
            .lifecycle()
            .canonical_lineage
            .require_source(self.account(), source)
    }

    /// Records semantic completion against the operation's frozen phase catalog.
    pub(crate) fn try_complete_phase(&mut self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        self.common
            .view()
            .coordinator
            .ledger_mut()
            .try_complete_phase(phase)
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.retained.baseline.account()
    }

    /// Admits backing storage before allocator entry without allocating a shared payload header.
    pub(crate) fn reserve_retained_storage(
        &mut self,
        phase: P,
        dimension: D,
        bytes: u64,
        allocations: u64,
    ) -> Result<(LiveReservation<O, P, D>, super::RetainedStorageLease), RetainedWriterError<D>>
    {
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            baseline,
            metadata:
                RetainedOperationMetadata {
                    terminal_account_share_work,
                    ..
                },
            ..
        } = &mut *retained;
        let OperationInvocationView {
            coordinator,
            lifecycle,
        } = common.view();
        let sticky_terminal_cause = &lifecycle.sticky_terminal_cause;
        let (_, _, ledger) = coordinator.into_exclusive_progress_and_ledger();
        ledger
            .ensure_work_open()
            .map_err(RetainedWriterError::Ledger)?;
        let (mut work, mut storage) = ledger.into_internal();
        let share_plan = *terminal_account_share_work;
        let share_work = &mut work;
        let result = super::reserve_exclusive_backing(
            &mut storage,
            baseline.account(),
            phase,
            dimension,
            bytes,
            allocations,
            move || {
                let share_work = share_work;
                super::operation_start::admit_internal_share(share_work, share_plan)
            },
        );
        if let Err(cause) = &result {
            let code = match cause {
                RetainedWriterError::Ledger(_) => PreparationFailureCode::ledger(),
                _ => PreparationFailureCode::product_graph(0x5354_4f52),
            };
            record_terminal_cause_with_precedence(sticky_terminal_cause, code);
        }
        result
    }

    pub(super) fn activate_failure_terminal_work(
        &mut self,
    ) -> Result<Option<RetainedHandleSharePlan<D>>, LedgerCause<D>> {
        self.common
            .view()
            .coordinator
            .ledger_mut()
            .seal_ordinary_work()?;
        let Some(capability) = self.retained.metadata.failure_terminal_work.as_ref() else {
            return Ok(None);
        };
        let dimension = capability.handle_share_work.charges()[0].0;
        self.common
            .view()
            .coordinator
            .ledger_mut()
            .activate_protected_work(dimension)?;
        Ok(self
            .retained
            .metadata
            .failure_terminal_work
            .take()
            .map(|capability| capability.handle_share_work))
    }

    pub(super) fn prepare_terminal_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Option<ControlCause> {
        self.common.view().prepare_terminal_control(final_progress)
    }

    pub(super) fn close_terminal_owners(&mut self) -> Option<ControlCause> {
        self.common.view().close_terminal_owners()
    }

    pub(crate) fn failure_terminal_work_limits(&self) -> Option<(usize, u64, u64)> {
        self.retained
            .metadata
            .failure_terminal_work
            .as_ref()
            .map(|capability| {
                (
                    capability.max_evidence_members,
                    capability.construction_work_limit,
                    capability.validation_work_limit,
                )
            })
    }

    pub(crate) fn preflight_failure_validation_work(&self) -> Result<(), RetainedWriterError<D>> {
        let preflight = self
            .account()
            .try_begin_account_preflight()
            .map_err(RetainedWriterError::AccountPoisoned)?;
        drop(preflight);
        Ok(())
    }

    pub(crate) fn qualify_failure_validation_work(
        &mut self,
        ordinary_dimension: D,
        max_evidence_members: usize,
        validation_work_limit: u64,
    ) -> Result<(), RetainedWriterError<D>> {
        let result = (|| {
            let Self {
                retained, common, ..
            } = self;
            let preflight = retained
                .baseline
                .account()
                .try_begin_account_preflight()
                .map_err(RetainedWriterError::AccountPoisoned)?;
            let capability = retained
                .metadata
                .failure_terminal_work
                .as_ref()
                .filter(|capability| !capability.validation_work_qualified)
                .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
            if retained.metadata.failure_work_permit_outstanding
                || retained.metadata.ordinary_work_permit_outstanding
                || common.as_read().check_work_quiescent().is_err()
                || common.lifecycle().sticky_terminal_cause.get().is_some()
            {
                return Err(RetainedWriterError::InvalidPublicationPlan);
            }
            if retained.metadata.failure_terminal_work_dimension == Some(ordinary_dimension)
                || !common.as_read().contains_dimension(ordinary_dimension)
            {
                return Err(RetainedWriterError::Ledger(LedgerCause::UnknownDimension {
                    dimension: ordinary_dimension,
                }));
            }
            let terminal_dimension = capability.handle_share_work.charges()[0].0;
            let original_max_evidence_members = capability.max_evidence_members;
            let construction_work_limit = capability.construction_work_limit;
            let original_validation_work_limit = capability.validation_work_limit;
            let max_evidence_members = max_evidence_members.max(original_max_evidence_members);
            let validation_work_limit = validation_work_limit.max(original_validation_work_limit);
            let original = RetainedFailureTerminalWorkPlan::with_evidence_work(
                terminal_dimension,
                original_max_evidence_members,
                construction_work_limit,
                original_validation_work_limit,
            )
            .requirement()
            .map_err(|()| RetainedWriterError::InvalidPublicationPlan)?;
            let qualified = RetainedFailureTerminalWorkPlan::with_evidence_work(
                terminal_dimension,
                max_evidence_members,
                construction_work_limit,
                validation_work_limit,
            )
            .requirement()
            .map_err(|()| RetainedWriterError::InvalidPublicationPlan)?;
            let additional_work = qualified
                .observed_work_limit()
                .checked_sub(original.observed_work_limit())
                .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
            common
                .view()
                .coordinator
                .ledger_mut()
                .transfer_failure_terminal_work(
                    ordinary_dimension,
                    terminal_dimension,
                    additional_work,
                )
                .map_err(RetainedWriterError::Ledger)?;
            drop(preflight);
            let capability = retained
                .metadata
                .failure_terminal_work
                .as_mut()
                .expect("qualification retains the dormant failure capability");
            capability.max_evidence_members = max_evidence_members;
            capability.validation_work_limit = validation_work_limit;
            capability.validation_work_qualified = true;
            Ok(())
        })();
        if let Err(cause) = &result {
            match cause {
                RetainedWriterError::Ledger(_) => self.record_ledger_failure(),
                RetainedWriterError::InvalidPublicationPlan => {
                    self.record_product_graph_failure(0x4657_5155);
                }
                _ => {}
            }
        }
        result
    }

    pub(crate) fn try_rebind_canonical_assembly(
        &mut self,
        writer: RetainedWriterPlan<P, D>,
    ) -> Result<(), RetainedWriterError<D>> {
        let replacement_plan = AttributionBatchPlan::<O, P, D, CanonicalOutput>::try_new(
            writer.phase,
            writer.capacity_dimension,
            writer.limits.attribution_slots,
            writer.limits.commitment_slots,
        )
        .map_err(map_batch_error)?;
        let replacement = self
            .common
            .view()
            .coordinator
            .ledger_mut()
            .try_attribution_batch_from_plan(replacement_plan)
            .map_err(map_batch_error)?;
        self.retained
            .baseline
            .try_replace_canonical_assembly(replacement)?;
        self.retained.metadata.canonical_assembly_limits = writer.limits;
        Ok(())
    }

    pub(super) fn split_failure_terminal_work_control(
        &mut self,
        work: RetainedHandleSharePlan<D>,
    ) -> (
        RetainedFailureWorkBinding<'_, PP, U, D>,
        RetainedWorkRoot<'_, O, P, D>,
    ) {
        let [(dimension, _), (paired_dimension, _)] = work.charges();
        if dimension != paired_dimension {
            self.record_product_graph_failure(0x4657_444d);
        }
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFieldsLoan {
            metadata:
                RetainedOperationMetadata {
                    failure_work_permit_outstanding,
                    ordinary_work_permit_partially_resolved,
                    ..
                },
            ..
        } = &mut *retained;
        let OperationInvocationView {
            coordinator,
            lifecycle,
        } = common.view();
        let sticky_terminal_cause = &lifecycle.sticky_terminal_cause;
        let (poll, ledger) = coordinator.into_terminal_poll_and_ledger();
        let (work, storage) = ledger.into_internal();
        (
            RetainedWorkBinding::protected(
                poll,
                dimension,
                failure_work_permit_outstanding,
                ordinary_work_permit_partially_resolved,
                sticky_terminal_cause,
            ),
            RetainedWorkRoot { work, storage },
        )
    }

    pub(crate) fn sticky_terminal_cause(&self) -> Option<PreparationFailureCode> {
        let permit_outstanding = self.planning_has_outstanding_work_permit();
        let abandoned_code = if self.retained.metadata.failure_work_permit_outstanding {
            FAILURE_WORK_PERMIT_ABANDONED_CODE
        } else {
            ORDINARY_WORK_PERMIT_ABANDONED_CODE
        };
        match (
            self.common.lifecycle().sticky_terminal_cause.get(),
            permit_outstanding,
        ) {
            (Some(cause), true) if matches!(cause.class(), PreparationFailureClass::Requested) => {
                Some(PreparationFailureCode::product_graph(abandoned_code))
            }
            (None, true) => Some(PreparationFailureCode::product_graph(abandoned_code)),
            (cause, _) => cause,
        }
    }

    pub(super) fn has_outstanding_work_permit(&self) -> bool {
        self.planning_has_outstanding_work_permit()
    }

    pub(super) const fn has_partially_resolved_work_permit(&self) -> bool {
        self.retained
            .metadata
            .ordinary_work_permit_partially_resolved
    }

    pub(super) fn record_ledger_failure(&mut self) {
        self.record_terminal_cause(PreparationFailureCode::ledger());
    }

    pub(super) fn record_product_graph_failure(&mut self, code: u32) {
        self.record_terminal_cause(PreparationFailureCode::product_graph(code));
    }

    pub(super) const fn has_failure_terminal_work_configuration(&self) -> bool {
        self.retained
            .metadata
            .failure_terminal_work_dimension
            .is_some()
    }

    pub(crate) fn is_terminal_only(&self) -> bool {
        self.common.as_read().ordinary_work_is_sealed()
            || (self
                .retained
                .metadata
                .failure_terminal_work_dimension
                .is_some()
                && self.retained.metadata.failure_terminal_work.is_none())
    }

    pub(super) fn validate_ordinary_dimension(
        &mut self,
        dimension: D,
    ) -> Result<(), LedgerCause<D>> {
        if self.contains_dimension(dimension) {
            Ok(())
        } else {
            self.record_ledger_failure();
            Err(LedgerCause::UnknownDimension { dimension })
        }
    }

    fn into_publication_open_error(
        mut self,
        cause: RetainedWriterError<D>,
        rollback_safe: bool,
    ) -> RetainedPublicationOpenError<Self, D> {
        let disposition = if !rollback_safe {
            RetainedPublicationOpenDisposition::FailClosed
        } else if self.is_terminal_only() {
            RetainedPublicationOpenDisposition::FallbackOnly
        } else {
            RetainedPublicationOpenDisposition::Retry
        };
        if matches!(&cause, RetainedWriterError::Ledger(_)) {
            self.record_ledger_failure();
        }
        RetainedPublicationOpenError {
            operation: self,
            cause,
            disposition,
        }
    }

    pub(super) fn prepare_evidence_only_deferred(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<super::DeferredAttachedTerminal, RetainedPublicationPrepareError> {
        match self.prepare_evidence_only_transition(final_progress)? {
            EvidenceOnlySuccessPrimaryTransition::Success(success) => success
                .into_deferred()
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            EvidenceOnlySuccessPrimaryTransition::Fallback(fallback, cause) => {
                fallback.try_prepare_deferred(cause)
            }
        }
    }

    #[expect(
        clippy::type_complexity,
        reason = "the short transition lends the exact original graph without moving its owners"
    )]
    fn prepare_evidence_only_transition(
        mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        EvidenceOnlySuccessPrimaryTransition<
            'inv,
            O,
            P,
            PP,
            U,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
        RetainedPublicationPrepareError,
    > {
        self.assert_backend();
        if self.common.lifecycle().execution_state.get() != StoreExecutionState::Running {
            self.record_product_graph_failure(StoreExecutionState::ABANDONED);
        }
        self.prepare_terminal_control(final_progress);
        self.close_terminal_owners();
        let terminal_cause = self.sticky_terminal_cause();
        let RetainedOperationLoan {
            retained,
            common,
            terminal,
            ..
        } = self;
        let ledger = common.into_terminal();
        let (physical, batches, primary) = terminal.install(retained.baseline, None);
        if let Some(cause) = terminal_cause {
            return Ok(EvidenceOnlySuccessPrimaryTransition::Fallback(
                TerminalFallbackGuard::new(
                    TerminalFallbackAttribution::new(ledger, batches, primary),
                    physical,
                ),
                cause,
            ));
        }
        TerminalPrimaryGuard::new(ledger, batches, primary, physical)
            .try_advance_evidence_only_success()
    }

    pub(super) fn prepare_minimal_fallback_deferred(
        self,
        cause: PreparationFailureCode,
    ) -> Result<super::DeferredAttachedTerminal, RetainedPublicationPrepareError> {
        let (guard, cause) = self.minimal_fallback_guard(cause);
        guard.try_prepare_deferred(cause)
    }

    #[expect(
        clippy::type_complexity,
        reason = "the short guard lends the exact original physical and numerical fields"
    )]
    fn minimal_fallback_guard(
        mut self,
        cause: PreparationFailureCode,
    ) -> (
        TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        PreparationFailureCode,
    ) {
        self.assert_backend();
        self.close_terminal_owners();
        let cause = self.sticky_terminal_cause().unwrap_or(cause);
        let RetainedOperationLoan {
            retained,
            common,
            terminal,
            ..
        } = self;
        let ledger = common.into_terminal();
        let (physical, batches, primary) = terminal.install(retained.baseline, None);
        let guard = TerminalFallbackGuard::new(
            TerminalFallbackAttribution::new(ledger, batches, primary),
            physical,
        );
        (guard, cause)
    }

    pub fn try_publication_assembly(
        mut self,
        mut plan: RetainedPublicationPlan<P, D>,
    ) -> Result<
        super::RetainedPublicationAssemblyLoan<
            'inv,
            O,
            P,
            PP,
            U,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
        RetainedPublicationOpenError<Self, D>,
    >
    where
        CanonicalBlock: Send + Sync + 'static,
        EvidenceBlock: Send + Sync + 'static,
    {
        let failure_only = plan.canonical.is_none();
        let writer_is_admitted = |writer: RetainedWriterPlan<P, D>| {
            self.contains_phase(writer.phase)
                && self.contains_dimension(writer.capacity_dimension)
                && self.contains_dimension(writer.attribution_dimension)
        };
        if !writer_is_admitted(plan.outcome_evidence.writer)
            || plan
                .canonical
                .is_some_and(|canonical| !writer_is_admitted(canonical.writer))
        {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        let terminal_failure_open = failure_only
            && matches!(
                self.retained.metadata.deferred_stage,
                Some(RetainedDeferredStage::TerminalOperation)
            );
        if self.is_terminal_only() && !terminal_failure_open {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        if let Some(capacity) = self
            .common
            .lifecycle()
            .canonical_lineage
            .source_mapping_capacity()
            && let Some(canonical) = plan.canonical.as_mut()
        {
            *canonical = canonical.with_canonical_source_mapping_capacity(capacity);
        }
        let terminal_limits = self.retained.metadata.terminal_writer.limits;
        let canonical_assembly_limits = self.retained.metadata.canonical_assembly_limits;
        let evidence_qualified = match try_qualify_retained_block_table_plan::<
            O,
            P,
            D,
            OutcomeEvidenceDestination,
            EvidenceBlock,
        >(
            self.account(),
            plan.outcome_evidence,
            terminal_limits,
            RetainedMembershipDestination::OutcomeEvidence,
        ) {
            Ok(qualified) => qualified,
            Err(cause) => {
                return Err(self.into_publication_open_error(cause, true));
            }
        };
        let canonical_qualified = match plan.canonical {
            Some(canonical) => match try_qualify_retained_block_table_plan::<
                O,
                P,
                D,
                CanonicalOutput,
                CanonicalBlock,
            >(
                self.account(),
                canonical,
                canonical_assembly_limits,
                RetainedMembershipDestination::Canonical,
            ) {
                Ok(qualified) => Some(qualified),
                Err(cause) => {
                    return Err(self.into_publication_open_error(cause, true));
                }
            },
            None => None,
        };
        if let Some(capability) = self.retained.metadata.failure_terminal_work.as_ref()
            && plan.outcome_evidence.total_blocks() > capability.max_evidence_members
        {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        let ordinary_handle_share_work = self.retained.metadata.terminal_account_share_work;
        let (
            open_handle_share_work,
            failure_evidence_handle_share_work,
            failure_terminal_work_state,
        ) = if failure_only {
            match self.activate_failure_terminal_work() {
                Ok(Some(work)) => (work, Some(work), FailureTerminalWorkState::Active),
                Ok(None) => {
                    return Err(self.into_publication_open_error(
                        RetainedWriterError::InvalidPublicationPlan,
                        true,
                    ));
                }
                Err(cause) => {
                    return Err(
                        self.into_publication_open_error(RetainedWriterError::Ledger(cause), true)
                    );
                }
            }
        } else {
            (
                ordinary_handle_share_work,
                None,
                FailureTerminalWorkState::Inactive,
            )
        };
        let open_result = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            inject_publication_account_open_panic();
            let Self {
                common, retained, ..
            } = &mut self;
            let (_, _, ledger) = common
                .view()
                .coordinator
                .into_exclusive_progress_and_ledger();
            let (mut work, _) = ledger.into_internal();
            try_open_publication_account(
                &mut work,
                retained.baseline.account(),
                open_handle_share_work,
            )
        }));
        let (account, set_id) = match open_result {
            Err(payload) => {
                let cause = RetainedWriterError::AccountPoisoned(
                    self.account()
                        .record_poison(AccountPoisonReason::OwnershipGraphUnverified),
                );
                discard_untrusted_panic_payload(payload);
                return Err(self.into_publication_open_error(cause, false));
            }
            Ok(Err(cause)) => {
                let rollback_safe = retained_open_failure_is_rollback_safe(&cause);
                return Err(self.into_publication_open_error(cause, rollback_safe));
            }
            Ok(Ok(open)) => open,
        };
        let (canonical_plan, mut canonical_batch, canonical_storage) =
            if let Some(qualified) = canonical_qualified {
                let QualifiedRetainedBlockTablePlan {
                    source,
                    batch: batch_plan,
                    storage,
                } = qualified;
                let batch = match self
                    .common
                    .view()
                    .coordinator
                    .ledger_mut()
                    .try_attribution_batch_from_plan(batch_plan)
                {
                    Ok(batch) => batch,
                    Err(error) => {
                        drop(account);
                        return Err(self.into_publication_open_error(map_batch_error(error), true));
                    }
                };
                (Some(source), Some(batch), Some(storage))
            } else {
                (None, None, None)
            };
        let QualifiedRetainedBlockTablePlan {
            source: evidence_plan,
            batch: evidence_batch_plan,
            storage: evidence_storage,
        } = evidence_qualified;
        let mut evidence_batch = match self
            .common
            .view()
            .coordinator
            .ledger_mut()
            .try_attribution_batch_from_plan(evidence_batch_plan)
        {
            Ok(batch) => batch,
            Err(error) => {
                drop(canonical_batch);
                drop(account);
                return Err(self.into_publication_open_error(map_batch_error(error), true));
            }
        };
        let canonical_table = if let Some(storage) = canonical_storage {
            let batch = match canonical_batch.as_mut() {
                Some(batch) => batch,
                None => unreachable_final_owner_state(),
            };
            let table = match try_create_retained_block_table(
                &mut self.internal_work_root(),
                &account,
                open_handle_share_work,
                batch,
                storage,
                set_id,
                RetainedMembershipDestination::Canonical,
            ) {
                Ok(batch) => batch,
                Err(cause) => {
                    drop(evidence_batch);
                    drop(canonical_batch);
                    drop(account);
                    let rollback_safe = retained_open_failure_is_rollback_safe(&cause);
                    return Err(self.into_publication_open_error(cause, rollback_safe));
                }
            };
            Some(table)
        } else {
            None
        };
        let evidence_table = match try_create_retained_block_table(
            &mut self.internal_work_root(),
            &account,
            open_handle_share_work,
            &mut evidence_batch,
            evidence_storage,
            set_id,
            RetainedMembershipDestination::OutcomeEvidence,
        ) {
            Ok(table) => table,
            Err(cause) => {
                let canonical_cleanup = match canonical_table {
                    Some(table) => drop_retained_table_with_account_anchor(table, &account),
                    None => RetainedStorageDropOutcome::Clean,
                };
                let cause =
                    finish_pre_attribution_batch_cleanup(canonical_batch, canonical_cleanup, cause);
                drop(evidence_batch);
                drop(account);
                let rollback_safe = retained_open_failure_is_rollback_safe(&cause);
                return Err(self.into_publication_open_error(cause, rollback_safe));
            }
        };
        let canonical_lineage = self.common.lifecycle().canonical_lineage;
        let fields = self
            .assembly
            .take()
            .expect("an unopened permission retains its assembly slot")
            .insert(super::PublicationAssemblyFields {
                canonical_table,
                evidence_table: Some(evidence_table),
                canonical_batch,
                evidence_batch: Some(evidence_batch),
                account: Some(account),
                canonical_lineage,
                set_id,
                canonical_plan,
                evidence_plan,
                ordinary_handle_share_work,
                failure_evidence_handle_share_work,
                failure_terminal_work_state,
                success_outcome_rejected: failure_only,
                mode: if canonical_plan.is_some() {
                    RetainedOutcomeMode::SuccessCapable
                } else {
                    RetainedOutcomeMode::FailureOnly
                },
                containment: AssemblyContainmentProvenance::Healthy,
                last_validation_epoch: 0,
            });
        Ok(super::RetainedPublicationAssemblyLoan {
            fields: fields.into(),
            operation: Some(self),
        })
    }
}

fn map_startup_cross_plane_error<O, P, D>(
    error: StartupCrossPlaneAdmissionFailure<O, P, D>,
) -> RetainedOperationStartError<D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: TerminalEvidenceKey + fmt::Debug,
{
    let writer = match error {
        StartupCrossPlaneAdmissionFailure::LedgerAdmission(cause) => map_batch_error(cause),
        StartupCrossPlaneAdmissionFailure::OwnerShare(cause) => cause,
        StartupCrossPlaneAdmissionFailure::PhysicalNoMutation(cause) => {
            map_startup_physical_no_mutation(cause)
        }
        StartupCrossPlaneAdmissionFailure::Uncertain(containment) => {
            let error = containment.poisoned().map_or(
                RetainedWriterError::OwnershipUnverified,
                RetainedWriterError::AccountPoisoned,
            );
            drop(containment);
            error
        }
    };
    RetainedOperationStartError::Writer(writer)
}

fn map_startup_physical_no_mutation<D>(cause: StartupPhysicalNoMutation) -> RetainedWriterError<D> {
    match cause {
        StartupPhysicalNoMutation::ForeignStartAdmission
        | StartupPhysicalNoMutation::ForeignOwner(_) => RetainedWriterError::ForeignAccount,
        StartupPhysicalNoMutation::RetainedOwnerOverflow
        | StartupPhysicalNoMutation::AccountingOverflow => {
            RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow)
        }
        StartupPhysicalNoMutation::ByteLimitExceeded { requested, limit } => {
            RetainedWriterError::Physical(RetainedAllocationError::ByteLimitExceeded {
                requested,
                limit,
            })
        }
        StartupPhysicalNoMutation::AllocationLimitExceeded { requested, limit } => {
            RetainedWriterError::Physical(RetainedAllocationError::AllocationLimitExceeded {
                requested,
                limit,
            })
        }
    }
}
