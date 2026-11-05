use super::operation_start::RetainedFailureTerminalWorkRequirement;
use super::ownership::finish_pre_attribution_batch_cleanup;
use super::publication_contract::{
    RetainedDeferredStage, RetainedPublicationOpenDisposition,
    retained_open_failure_is_rollback_safe,
};
use super::startup;
use super::{
    AccountPoisonReason, AssemblyContainmentProvenance, CanonicalLineageAuthority,
    EvidenceOnlySuccessPrimaryTransition, FailureTerminalWorkState, FallbackPublicationBaseline,
    PhysicalStorageAccount, PreparationFailureClass, PreparationFailureCode,
    PreparedRetainedEvidenceOnlyOutcome, PreparedRetainedPublication,
    PreparedRetainedPublicationOutcome, PrimaryPublicationBatches, PublishedCanonicalAggregate,
    QualifiedRetainedBlockTablePlan, RetainedAllocationError, RetainedAllocationPlan,
    RetainedBatchLimits, RetainedBuildFailureCode, RetainedFailureTerminalWorkPlan,
    RetainedHandleSharePlan, RetainedMembershipDestination, RetainedOperationStartError,
    RetainedOperationStartPlan, RetainedOutcomeMode, RetainedPublicationAssembly,
    RetainedPublicationOpenError, RetainedPublicationPlan, RetainedPublicationPrepareError,
    RetainedStorageDropOutcome, RetainedWriterError, RetainedWriterPlan,
    StartupCrossPlaneAdmissionFailure, StartupCrossPlanePlan, StartupPhysicalManifest,
    StartupPhysicalNoMutation, StartupRetainedPartGuard, StartupTerminalEvidenceBuildError,
    TerminalEvidenceBackingPlan, TerminalEvidenceKey, TerminalFallbackAttribution,
    TerminalFallbackGuard, TerminalLedgerEvidence, TerminalPhysicalOwnershipManifest,
    TerminalPrimaryGuard, drop_retained_table_with_account_anchor, map_batch_error,
    try_create_retained_block_table, try_open_publication_account,
    try_qualify_retained_block_table_plan, unreachable_final_owner_state,
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
use crate::coordinator::{OperationCoordinator, OperationPollCheckpoint};
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
pub(crate) use borrowed_work::{
    BorrowedRetainedWorkPermission, RetainedFailureWorkBinding, RetainedFailureWorkConsumeError,
    RetainedFailureWorkControl, RetainedFailureWorkFinishError, RetainedFailureWorkPermission,
    RetainedFailureWorkStartError, RetainedWorkBinding, RetainedWorkConsumeError,
    RetainedWorkControl, RetainedWorkFinishError, RetainedWorkPermission, RetainedWorkRoot,
    RetainedWorkStartError,
};
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
    pub(crate) progress: crate::coordinator::OperationProgressState<'op, O, P, PP, U, D>,
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
pub(super) struct OperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) coordinator: OperationCoordinator<O, P, PP, U, D>,
    sticky_terminal_cause: Cell<Option<PreparationFailureCode>>,
    control_action_debt: Cell<u16>,
    execution_state: Cell<StoreExecutionState>,
    canonical_lineage: CanonicalLineageAuthority,
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
        StoreExecutionState::stop(
            &self.common.execution_state,
            StoreExecutionState::KnownError,
        );
        record_terminal_cause_with_precedence(&self.common.sticky_terminal_cause, cause);
        if self
            .common
            .coordinator
            .retained_ledger()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &self.common.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        }
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
    fn record_terminal_gate(
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

    fn preflight_exclusive(&self) -> Result<(), ExecutionError<D>> {
        check_execution_state(&self.sticky_terminal_cause, &self.execution_state, || {
            self.coordinator.recorded_control_cause()
        })?;
        self.coordinator
            .retained_ledger()
            .check_work_quiescent()
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    &self.sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
                ExecutionError::Ledger(cause)
            })
    }

    fn borrow_exclusive_parts<'loan>(
        &'loan mut self,
        account: &'loan PhysicalStorageAccount,
        share_work: RetainedHandleSharePlan<D>,
    ) -> Result<ExclusiveOperationParts<'loan, O, P, PP, U, D>, ExecutionError<D>> {
        let Self {
            coordinator,
            sticky_terminal_cause,
            control_action_debt,
            execution_state,
            ..
        } = self;
        let (poll, progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (work, storage) = ledger.exclusive().map_err(|cause| {
            record_terminal_cause_with_precedence(
                sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            ExecutionError::Ledger(cause)
        })?;
        let progress = progress.bind(storage.progress_evidence());
        Ok(ExclusiveOperationParts {
            work,
            progress,
            ordinary: ExecutionOrdinaryStorage {
                storage,
                poll,
                failure: sticky_terminal_cause,
                debt: control_action_debt,
                execution_state,
            },
            account,
            share_work,
        })
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
            record_terminal_cause_with_precedence(&common.sticky_terminal_cause, cause);
        } else if common.execution_state.get() != StoreExecutionState::Running {
            record_terminal_cause_with_precedence(
                &common.sticky_terminal_cause,
                PreparationFailureCode::product_graph(StoreExecutionState::ABANDONED),
            );
        }
        if common
            .coordinator
            .retained_ledger()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &common.sticky_terminal_cause,
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
        Ok(RetainedOperation {
            retained: root
                .retained
                .as_mut()
                .expect("attachment installs fully qualified retained fields"),
            assembly: Some(&mut root.assembly),
            common: InvocationLoan {
                common: &mut root.common,
                not_send_or_sync: PhantomData,
            },
        })
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
    // Physical owners precede common so their cleanup retains the invocation's ledger.
    assembly: super::PublicationAssemblySlot<O, P, D, CanonicalBlock, EvidenceBlock>,
    retained: Option<RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
    common: Option<OperationInvocation<O, P, PP, U, D>>,
    started: bool,
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
            common: None,
            started: false,
        }
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
        retained.deferred_stage = None;
        let operation = RetainedOperation {
            retained,
            assembly: None,
            common: InvocationLoan {
                common: &mut self.common,
                not_send_or_sync: PhantomData,
            },
        };
        if let Some(fields) = self.assembly.as_mut()
            && !fields.is_empty()
        {
            Some(
                RetainedPublicationAssembly {
                    fields,
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
        let common = self
            .common
            .as_mut()
            .expect("the original common was checked");
        let account = binding.admission.account();
        let account_ready = match account.try_begin_account_preflight() {
            Ok(permit) => {
                drop(permit);
                true
            }
            Err(_) => {
                record_terminal_cause_with_precedence(
                    &common.sticky_terminal_cause,
                    PreparationFailureCode::product_graph(0x5541_4343),
                );
                false
            }
        };
        if account_ready && common.sticky_terminal_cause.get().is_none() {
            let result = common.coordinator.try_prepare_typed_control(final_progress);
            common.record_terminal_gate(result);
        }
        let close = common.coordinator.close_for_terminal_extraction();
        common.record_terminal_gate(close);
        // Extension disposal may quarantine the same account. No client callback follows
        // this gate, and no replacement admission or retained graph is constructed.
        match account.try_begin_account_preflight() {
            Ok(permit) => drop(permit),
            Err(_) => record_terminal_cause_with_precedence(
                &common.sticky_terminal_cause,
                PreparationFailureCode::product_graph(0x5541_4343),
            ),
        }
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = self
            .common
            .take()
            .expect("the outer root retains its original common");
        let ledger = coordinator.into_ledger_for_typed_publication();
        let snapshot = match ledger.finish() {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                record_terminal_cause_with_precedence(
                    &sticky_terminal_cause,
                    PreparationFailureCode::ledger(),
                );
                failure.into_parts().1
            }
        };
        Ok(UnattachedTerminalEvidence {
            snapshot,
            cause: sticky_terminal_cause.get(),
        })
    }

    pub(crate) fn record_payload_failure(&mut self, code: u32) {
        let Some(common) = self.common.as_mut() else {
            return;
        };
        StoreExecutionState::stop(&common.execution_state, StoreExecutionState::KnownError);
        record_terminal_cause_with_precedence(
            &common.sticky_terminal_cause,
            PreparationFailureCode::product_graph(code),
        );
        if common
            .coordinator
            .retained_ledger()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &common.sticky_terminal_cause,
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
                retained.deferred_stage,
                Some(RetainedDeferredStage::Operation)
            )
        {
            return None;
        }
        retained.deferred_stage = Some(RetainedDeferredStage::TerminalOperation);
        Some(RetainedOperation {
            retained,
            assembly: Some(&mut self.assembly),
            common: InvocationLoan {
                common: &mut self.common,
                not_send_or_sync: PhantomData,
            },
        })
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
            retained.deferred_stage,
            Some(RetainedDeferredStage::Assembly)
        ) {
            return None;
        }
        retained.deferred_stage = None;
        Some(RetainedPublicationAssembly {
            fields,
            operation: Some(RetainedOperation {
                retained,
                assembly: None,
                common: InvocationLoan {
                    common: &mut self.common,
                    not_send_or_sync: PhantomData,
                },
            }),
        })
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
        let RetainedDeferredStage::Failure { fallback_cause } = retained.deferred_stage.as_ref()?
        else {
            return None;
        };
        let fallback_cause = *fallback_cause;
        retained.deferred_stage = None;
        Some(super::RetainedFailureAssembly {
            assembly: RetainedPublicationAssembly {
                fields,
                operation: Some(RetainedOperation {
                    retained,
                    assembly: None,
                    common: InvocationLoan {
                        common: &mut self.common,
                        not_send_or_sync: PhantomData,
                    },
                }),
            },
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
        } = retained.deferred_stage.as_ref()?
        else {
            return None;
        };
        let cause = *cause;
        let activation_containment_required = *activation_containment_required;
        retained.deferred_stage = None;
        Some(super::RetainedFallbackOnly {
            assembly: RetainedPublicationAssembly {
                fields,
                operation: Some(RetainedOperation {
                    retained,
                    assembly: None,
                    common: InvocationLoan {
                        common: &mut self.common,
                        not_send_or_sync: PhantomData,
                    },
                }),
            },
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
            retained.deferred_stage,
            Some(RetainedDeferredStage::Success { .. })
        ) {
            return None;
        }
        let RetainedDeferredStage::Success { proof } = retained.deferred_stage.take()? else {
            return None;
        };
        Some(super::ValidatedRetainedSuccess {
            assembly: RetainedPublicationAssembly {
                fields,
                operation: Some(RetainedOperation {
                    retained,
                    assembly: None,
                    common: InvocationLoan {
                        common: &mut self.common,
                        not_send_or_sync: PhantomData,
                    },
                }),
            },
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
        let (Some(fields), Some(retained)) = (self.assembly.as_mut(), self.retained.as_mut())
        else {
            return;
        };
        if fields.is_empty() || self.common.is_none() {
            return;
        }
        // Reuse the checked table-before-attribution cleanup even if its permission was forgotten.
        drop(RetainedPublicationAssembly {
            fields,
            operation: Some(RetainedOperation {
                retained,
                assembly: None,
                common: InvocationLoan {
                    common: &mut self.common,
                    not_send_or_sync: PhantomData,
                },
            }),
        });
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
    common: &'inv mut Option<OperationInvocation<O, P, PP, U, D>>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<O, P, PP, U, D> std::ops::Deref for InvocationLoan<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    type Target = OperationInvocation<O, P, PP, U, D>;

    fn deref(&self) -> &Self::Target {
        self.common
            .as_ref()
            .expect("a live stage borrows the outer invocation")
    }
}

impl<O, P, PP, U, D> std::ops::DerefMut for InvocationLoan<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.common
            .as_mut()
            .expect("a live stage borrows the outer invocation")
    }
}

impl<O, P, PP, U, D> InvocationLoan<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn into_terminal(self) -> OperationInvocation<O, P, PP, U, D> {
        self.common
            .take()
            .expect("terminal consumes the outer invocation once")
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        std::ptr::from_ref(&*self.common).cast()
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

/// Short permission over the invocation and its actual retained state.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
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
    pub(super) retained:
        &'inv mut RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    assembly:
        Option<&'inv mut super::PublicationAssemblySlot<O, P, D, CanonicalBlock, EvidenceBlock>>,
    pub(super) common: InvocationLoan<'inv, O, P, PP, U, D>,
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
    RetainedOperation<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    pub(super) fn prepare_body_deferral(&self) -> Option<Result<(), LedgerCause<D>>> {
        if self.retained.deferred_stage.is_some()
            || !self.retained.has_deferred_storage()
            || self.retained.failure_work_permit_outstanding
            || self.retained.ordinary_work_permit_outstanding
        {
            return None;
        }
        let common = self.common.common.as_ref()?;
        Some(common.coordinator.retained_ledger().seal_ordinary_work())
    }

    pub(crate) fn defer_until_body_cleanup(self) -> Result<(), Self> {
        if !self.assembly.as_ref().is_some_and(|slot| slot.is_none())
            || !matches!(self.prepare_body_deferral(), Some(Ok(())))
        {
            return Err(self);
        }
        self.retained.deferred_stage = Some(RetainedDeferredStage::Operation);
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
        let RetainedOperationFields {
            baseline,
            terminal_account_share_work,
            ..
        } = &mut **retained;
        common.borrow_exclusive_parts(baseline.account(), *terminal_account_share_work)
    }

    pub(crate) fn check_execution_running(&self) -> Result<(), ExecutionError<D>> {
        check_execution_state(
            &self.common.sticky_terminal_cause,
            &self.common.execution_state,
            || self.common.coordinator.recorded_control_cause(),
        )
    }

    pub(crate) fn settle_importer_return(&self) {
        StoreExecutionState::settle_caught_unwind(
            &self.common.execution_state,
            &self.common.sticky_terminal_cause,
        );
    }

    pub(crate) fn record_terminal_cause(&mut self, cause: PreparationFailureCode) {
        record_terminal_cause_with_precedence(&self.common.sticky_terminal_cause, cause);
    }

    pub(super) fn stop_for_containment(&mut self) {
        StoreExecutionState::stop(
            &self.common.execution_state,
            StoreExecutionState::KnownError,
        );
        if self
            .common
            .coordinator
            .retained_ledger()
            .seal_ordinary_work()
            .is_err()
        {
            self.record_terminal_cause(PreparationFailureCode::ledger());
        }
    }

    pub(crate) fn poll_terminal_control(&mut self) {
        let cause = self
            .common
            .coordinator
            .poll()
            .err()
            .map(|cause| PreparationFailureCode::from_control(&cause));
        if let Some(cause) = cause {
            self.record_terminal_cause(cause);
        }
    }

    pub(crate) fn planning_baseline(&self) -> RetainedPlanningBaseline {
        let state = self.common.coordinator.retained_ledger().planning_state();
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
        self.retained.failure_work_permit_outstanding
            || self.retained.ordinary_work_permit_outstanding
            || self
                .common
                .coordinator
                .retained_ledger()
                .check_work_quiescent()
                .is_err()
    }

    pub(crate) fn planning_matches_baseline(&self, baseline: RetainedPlanningBaseline) -> bool {
        if self.planning_has_outstanding_work_permit() {
            return false;
        }
        let state = self.common.coordinator.retained_ledger().planning_state();
        state.current_live_bytes() == baseline.current_live_bytes
            && state.active_reservations() == baseline.active_reservations
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
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
    ledger: &'a crate::ledger::Ledger<O, P, D>,
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
    coordinator: &'a mut OperationCoordinator<O, P, PP, U, D>,
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
        let result = self.coordinator.dimension_evidence(dimension);
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
        let result = self
            .coordinator
            .retained_ledger()
            .charge_producer_attempted(dimension, amount, self.failure_terminal_work_dimension);
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
            .retained_ledger()
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
        let result = self
            .coordinator
            .retained_ledger()
            .charge_observed_batch(charges);
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
            .retained_ledger()
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
            && self
                .coordinator
                .retained_ledger()
                .contains_dimension(dimension)
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
        self.ledger.contains_phase(phase)
    }

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        self.failure_terminal_work_dimension != Some(dimension)
            && self.ledger.contains_dimension(dimension)
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
    pub(crate) fn control_checkpoint(&mut self) -> RetainedControlCheckpoint<'_, O, P, PP, U, D> {
        let Self {
            retained, common, ..
        } = self;
        let RetainedOperationFields {
            failure_terminal_work_dimension,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = &mut **common;
        let (poll, ledger) = coordinator.split_poll_checkpoint_and_retained_ledger();
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
        let RetainedOperationFields {
            failure_terminal_work_dimension,
            ordinary_work_permit_outstanding,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = &mut **common;
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
            .coordinator
            .split_exclusive_progress_and_ledger();
        let (work, storage) = ledger.split_internal();
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
        let RetainedOperationFields {
            failure_terminal_work_dimension,
            ordinary_work_permit_outstanding,
            ordinary_work_permit_partially_resolved,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = &mut **common;
        let (poll, _, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (work, storage) = ledger.split_internal();
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
        self.retained.failure_terminal_work_dimension != Some(dimension)
            && self
                .common
                .coordinator
                .retained_ledger()
                .contains_dimension(dimension)
    }

    pub fn contains_phase(&self, phase: P) -> bool {
        self.common
            .coordinator
            .retained_ledger()
            .contains_phase(phase)
    }

    pub(crate) fn require_canonical_source<SourceBlock>(
        &self,
        source: &PublishedCanonicalAggregate<SourceBlock>,
    ) -> Result<(), super::RetainedPublicationError<D>> {
        self.common
            .canonical_lineage
            .require_source(self.account(), source)
    }

    /// Records semantic completion against the operation's frozen phase catalog.
    pub(crate) fn try_complete_phase(&self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        self.common
            .coordinator
            .retained_ledger()
            .try_complete_phase(phase)
    }

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
    ) -> Result<
        PendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedOperationStartError<D>,
    > {
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
    ) -> Result<
        PendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedOperationStartError<D>,
    > {
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
    ) -> Result<
        PendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedOperationStartError<D>,
    > {
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
            sticky_terminal_cause: Cell::new(None),
            control_action_debt: Cell::new(0),
            execution_state: Cell::new(StoreExecutionState::Running),
            canonical_lineage,
        });
        let common = root
            .common
            .as_ref()
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
                .coordinator
                .retained_ledger()
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
        if plan.terminal_writer.limits.attribution_slots == 0
            || plan.terminal_writer.limits.commitment_slots == 0
        {
            return Err(RetainedOperationStartError::InvalidTerminalLimits);
        }
        match plan.failure_terminal_work {
            Some(failure_plan) => {
                let failure_dimension = failure_plan.dimension();
                let ordinary_share_charges = plan.terminal_account_share_work.charges();
                if plan.terminal_writer.capacity_dimension == failure_dimension
                    || plan.terminal_writer.attribution_dimension == failure_dimension
                    || ordinary_share_charges
                        .iter()
                        .any(|(dimension, _)| *dimension == failure_dimension)
                    || plan.fallback_commitment.dimension() == failure_dimension
                    || plan.snapshot_commitment.dimension() == failure_dimension
                {
                    return Err(RetainedOperationStartError::InvalidTerminalLimits);
                }
                let requirement = failure_plan
                    .requirement()
                    .map_err(|()| RetainedOperationStartError::InvalidTerminalLimits)?;
                Ok(Some((failure_plan, requirement)))
            }
            None => Ok(None),
        }
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
    ) -> Result<
        RetainedOperationFields<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedOperationStartError<D>,
    > {
        check_execution_state(
            &common.sticky_terminal_cause,
            &common.execution_state,
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
        let account = admission.account();
        let health = account.try_begin_account_preflight().map_err(|cause| {
            RetainedOperationStartError::Writer(RetainedWriterError::AccountPoisoned(cause))
        })?;
        drop(health);
        let ledger = common.coordinator.retained_ledger();
        let failure_terminal_work_dimension = failure_terminal_work
            .as_ref()
            .map(|(plan, _)| plan.dimension());
        let evidence_assembly = RetainedBatchLimits::new(
            plan.terminal_writer
                .limits
                .attribution_slots
                .checked_add(1)
                .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
                .max(2),
            plan.terminal_writer
                .limits
                .commitment_slots
                .checked_add(1)
                .ok_or(RetainedOperationStartError::InvalidTerminalLimits)?
                .max(2),
        );
        let canonical_assembly = plan.terminal_writer.limits;
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
        if let Some((failure_plan, requirement)) = failure_terminal_work {
            ledger
                .preflight_protected_work_attachment(
                    failure_plan.dimension(),
                    &failure_plan.share_work().charges(),
                    requirement.maximum_share_actions(),
                )
                .map_err(|cause| {
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
                })?;
        }
        let ready = admission
            .try_prepare_startup_cross_plane(ledger, cross_plane)
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
        let failure_terminal_work = failure_terminal_work
            .map(|(failure_plan, _)| FailureTerminalWorkCapability::from_qualified(failure_plan));
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
        if startup_owner_result.is_err() {
            let probe = ledger.test_probe();
            STARTUP_FAILURE_LEDGER_SNAPSHOT.with(|snapshot| {
                snapshot.set(Some((
                    probe.active_reservations(),
                    probe.current_live_bytes(),
                )));
            });
        }
        let startup_owner = startup_owner_result.map_err(RetainedOperationStartError::Writer)?;
        let baseline = startup_owner
            .try_into_baseline()
            .map_err(RetainedOperationStartError::Writer)?;
        let baseline = baseline
            .try_attach(ledger)
            .map_err(RetainedOperationStartError::Writer)?;
        Ok(RetainedOperationFields {
            baseline,
            deferred_stage: None,
            terminal_writer: plan.terminal_writer,
            canonical_assembly_limits: canonical_assembly,
            terminal_account_share_work: plan.terminal_account_share_work,
            failure_terminal_work,
            failure_terminal_work_dimension,
            failure_work_permit_outstanding: false,
            ordinary_work_permit_outstanding: false,
            ordinary_work_permit_partially_resolved: false,
        })
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
        let RetainedOperationFields {
            baseline,
            terminal_account_share_work,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = &mut **common;
        let (_, _, ledger) = coordinator.split_exclusive_progress_and_ledger();
        ledger
            .ensure_work_open()
            .map_err(RetainedWriterError::Ledger)?;
        let (mut work, storage) = ledger.split_internal();
        let share_plan = *terminal_account_share_work;
        let share_work = &mut work;
        let result = super::reserve_exclusive_backing(
            &storage,
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
            .coordinator
            .retained_ledger()
            .seal_ordinary_work()?;
        let Some(capability) = self.retained.failure_terminal_work.as_ref() else {
            return Ok(None);
        };
        let dimension = capability.handle_share_work.charges()[0].0;
        self.common
            .coordinator
            .retained_ledger()
            .activate_protected_work(dimension)?;
        Ok(self
            .retained
            .failure_terminal_work
            .take()
            .map(|capability| capability.handle_share_work))
    }

    fn record_terminal_gate(
        &mut self,
        result: Result<(), PublicationCause<D>>,
    ) -> Option<ControlCause> {
        self.common.record_terminal_gate(result)
    }

    pub(super) fn prepare_terminal_control(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Option<ControlCause> {
        let result = self
            .common
            .coordinator
            .try_prepare_typed_control(final_progress);
        self.record_terminal_gate(result)
    }

    pub(super) fn close_terminal_owners(&mut self) -> Option<ControlCause> {
        let result = self.common.coordinator.close_for_terminal_extraction();
        self.record_terminal_gate(result)
    }

    pub(crate) fn failure_terminal_work_limits(&self) -> Option<(usize, u64, u64)> {
        self.retained
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
            let preflight = self
                .account()
                .try_begin_account_preflight()
                .map_err(RetainedWriterError::AccountPoisoned)?;
            let capability = self
                .retained
                .failure_terminal_work
                .as_ref()
                .filter(|capability| !capability.validation_work_qualified)
                .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
            if self.has_outstanding_work_permit() || self.sticky_terminal_cause().is_some() {
                return Err(RetainedWriterError::InvalidPublicationPlan);
            }
            if !self.contains_dimension(ordinary_dimension) {
                return Err(RetainedWriterError::Ledger(LedgerCause::UnknownDimension {
                    dimension: ordinary_dimension,
                }));
            }
            let max_evidence_members = max_evidence_members.max(capability.max_evidence_members);
            let validation_work_limit = validation_work_limit.max(capability.validation_work_limit);
            let terminal_dimension = capability.handle_share_work.charges()[0].0;
            let original = RetainedFailureTerminalWorkPlan::with_evidence_work(
                terminal_dimension,
                capability.max_evidence_members,
                capability.construction_work_limit,
                capability.validation_work_limit,
            )
            .requirement()
            .map_err(|()| RetainedWriterError::InvalidPublicationPlan)?;
            let qualified = RetainedFailureTerminalWorkPlan::with_evidence_work(
                terminal_dimension,
                max_evidence_members,
                capability.construction_work_limit,
                validation_work_limit,
            )
            .requirement()
            .map_err(|()| RetainedWriterError::InvalidPublicationPlan)?;
            let additional_work = qualified
                .observed_work_limit()
                .checked_sub(original.observed_work_limit())
                .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
            self.common
                .coordinator
                .retained_ledger()
                .transfer_failure_terminal_work(
                    ordinary_dimension,
                    terminal_dimension,
                    additional_work,
                )
                .map_err(RetainedWriterError::Ledger)?;
            drop(preflight);
            let capability = self
                .retained
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
            .coordinator
            .retained_ledger()
            .try_attribution_batch_from_plan(replacement_plan)
            .map_err(map_batch_error)?;
        self.retained
            .baseline
            .try_replace_canonical_assembly(replacement)?;
        self.retained.canonical_assembly_limits = writer.limits;
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
        let RetainedOperationFields {
            failure_work_permit_outstanding,
            ordinary_work_permit_partially_resolved,
            ..
        } = &mut **retained;
        let OperationInvocation {
            coordinator,
            sticky_terminal_cause,
            ..
        } = &mut **common;
        let (poll, ledger) = coordinator.split_terminal_poll_checkpoint_and_retained_ledger();
        let (work, storage) = ledger.split_internal();
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
        let abandoned_code = if self.retained.failure_work_permit_outstanding {
            FAILURE_WORK_PERMIT_ABANDONED_CODE
        } else {
            ORDINARY_WORK_PERMIT_ABANDONED_CODE
        };
        match (self.common.sticky_terminal_cause.get(), permit_outstanding) {
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
        self.retained.ordinary_work_permit_partially_resolved
    }

    pub(super) fn record_ledger_failure(&mut self) {
        self.record_terminal_cause(PreparationFailureCode::ledger());
    }

    pub(super) fn record_product_graph_failure(&mut self, code: u32) {
        self.record_terminal_cause(PreparationFailureCode::product_graph(code));
    }

    pub(super) const fn has_failure_terminal_work_configuration(&self) -> bool {
        self.retained.failure_terminal_work_dimension.is_some()
    }

    pub(crate) fn is_terminal_only(&self) -> bool {
        self.common
            .coordinator
            .retained_ledger()
            .ordinary_work_is_sealed()
            || (self.retained.failure_terminal_work_dimension.is_some()
                && self.retained.failure_terminal_work.is_none())
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

    /// Freezes a successful terminal ledger without publishing canonical or domain evidence tables.
    ///
    /// This is the exact terminal for a read-only operation whose admitted work completed but whose
    /// compiled result requires no retained output. The pre-admitted fallback is discarded before
    /// terminal freeze. A final control or ledger rejection instead publishes the minimal fallback.
    #[must_use = "the prepared evidence-only outcome must be handed off or deliberately dropped"]
    pub fn prepare_evidence_only_success(
        mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedRetainedEvidenceOnlyOutcome<F, P, D>, RetainedPublicationPrepareError> {
        // Success cannot consume an unclassified stopped invocation. Real rejection adapters
        // supply their owned requested cause before the terminal failure path instead.
        if self.common.execution_state.get() != StoreExecutionState::Running {
            self.record_product_graph_failure(StoreExecutionState::ABANDONED);
        }
        self.prepare_terminal_control(final_progress);
        self.close_terminal_owners();
        let terminal_cause = self.sticky_terminal_cause();
        let RetainedOperation {
            retained, common, ..
        } = self;
        let OperationInvocation { coordinator, .. } = common.into_terminal();
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, batches) = retained.baseline.take_parts();
        let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), ()>::new(
            fallback, evidence, None, None, None,
        );
        if let Some(cause) = terminal_cause {
            let prepared = TerminalFallbackGuard::new(
                TerminalFallbackAttribution::new(
                    ledger,
                    batches,
                    PrimaryPublicationBatches::new(None, None),
                ),
                physical,
            )
            .try_prepare(cause)?;
            return Ok(match prepared.into_outcome() {
                PreparedRetainedPublicationOutcome::MinimalFallback(fallback) => {
                    PreparedRetainedEvidenceOnlyOutcome::MinimalFallback(fallback)
                }
                PreparedRetainedPublicationOutcome::Success(_)
                | PreparedRetainedPublicationOutcome::RichFailure(_) => {
                    unreachable_final_owner_state()
                }
            });
        }

        let primary = TerminalPrimaryGuard::new(
            ledger,
            batches,
            PrimaryPublicationBatches::new(None, None),
            physical,
        );
        match primary.try_advance_evidence_only_success()? {
            EvidenceOnlySuccessPrimaryTransition::Success(success) => success
                .into_prepared()
                .map(PreparedRetainedEvidenceOnlyOutcome::Success)
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            EvidenceOnlySuccessPrimaryTransition::Fallback(fallback, cause) => {
                let prepared = fallback.try_prepare(cause)?;
                Ok(match prepared.into_outcome() {
                    PreparedRetainedPublicationOutcome::MinimalFallback(fallback) => {
                        PreparedRetainedEvidenceOnlyOutcome::MinimalFallback(fallback)
                    }
                    PreparedRetainedPublicationOutcome::Success(_)
                    | PreparedRetainedPublicationOutcome::RichFailure(_) => {
                        unreachable_final_owner_state()
                    }
                })
            }
        }
    }

    /// Freezes the pre-admitted terminal baseline without opening a block-table assembly.
    ///
    /// This path performs no allocation, table construction, root validation, control callback, or
    /// other fallible action. It is therefore available after every recoverable assembly-open
    /// failure. Fail-closed open errors deliberately do not return the operation and cannot reach
    /// this method.
    #[must_use = "the prepared fallback must be handed off or deliberately dropped"]
    pub fn prepare_minimal_fallback(
        mut self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        self.close_terminal_owners();
        let cause = self.sticky_terminal_cause().unwrap_or(cause);
        let RetainedOperation {
            retained, common, ..
        } = self;
        let OperationInvocation { coordinator, .. } = common.into_terminal();
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, batches) = retained.baseline.take_parts();
        let physical = TerminalPhysicalOwnershipManifest::new(fallback, evidence, None, None, None);
        TerminalFallbackGuard::new(
            TerminalFallbackAttribution::new(
                ledger,
                batches,
                PrimaryPublicationBatches::new(None, None),
            ),
            physical,
        )
        .try_prepare(cause)
    }

    pub fn try_publication_assembly(
        mut self,
        mut plan: RetainedPublicationPlan<P, D>,
    ) -> Result<
        RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
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
                self.retained.deferred_stage,
                Some(RetainedDeferredStage::TerminalOperation)
            );
        if self.is_terminal_only() && !terminal_failure_open {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        if let Some(capacity) = self.common.canonical_lineage.source_mapping_capacity()
            && let Some(canonical) = plan.canonical.as_mut()
        {
            *canonical = canonical.with_canonical_source_mapping_capacity(capacity);
        }
        let terminal_limits = self.retained.terminal_writer.limits;
        let canonical_assembly_limits = self.retained.canonical_assembly_limits;
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
        if let Some(capability) = self.retained.failure_terminal_work.as_ref()
            && plan.outcome_evidence.total_blocks() > capability.max_evidence_members
        {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        let ordinary_handle_share_work = self.retained.terminal_account_share_work;
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
            try_open_publication_account(
                self.common.coordinator.retained_ledger(),
                self.account(),
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
                    .coordinator
                    .retained_ledger()
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
            .coordinator
            .retained_ledger()
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
                self.common.coordinator.retained_ledger(),
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
            self.common.coordinator.retained_ledger(),
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
        let canonical_lineage = self.common.canonical_lineage;
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
        Ok(RetainedPublicationAssembly {
            fields,
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
        StartupCrossPlaneAdmissionFailure::PhysicalNoMutation(cause) => match cause {
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
        },
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
