use super::ownership::finish_pre_attribution_batch_cleanup;
use super::publication_contract::{
    RetainedPublicationOpenDisposition, retained_open_failure_is_rollback_safe,
};
use super::startup;
use super::{
    AccountPoisonReason, AssemblyContainmentProvenance, CanonicalLineageAuthority,
    FailureTerminalWorkState, FallbackPublicationBaseline, PhysicalStorageAccount,
    PreparationFailureClass, PreparationFailureCode, PreparedRetainedPublication,
    PrimaryPublicationBatches, PublishedCanonicalAggregate, QualifiedRetainedBlockTablePlan,
    RetainedAllocationError, RetainedAllocationPlan, RetainedBatchLimits, RetainedBuildFailureCode,
    RetainedFailureTerminalWorkPlan, RetainedHandleSharePlan, RetainedMembershipDestination,
    RetainedOperationStartError, RetainedOperationStartPlan, RetainedOutcomeMode,
    RetainedPublicationAssembly, RetainedPublicationOpenError, RetainedPublicationPlan,
    RetainedPublicationPrepareError, RetainedStorageDropOutcome, RetainedWriterError,
    RetainedWriterPlan, StartupCrossPlaneAdmissionFailure, StartupCrossPlanePlan,
    StartupPhysicalManifest, StartupPhysicalNoMutation, StartupRetainedPartGuard,
    StartupTerminalEvidenceBuildError, TerminalEvidenceBackingPlan, TerminalEvidenceKey,
    TerminalFallbackAttribution, TerminalFallbackGuard, TerminalLedgerEvidence,
    TerminalPhysicalOwnershipManifest, drop_retained_table_with_account_anchor, map_batch_error,
    try_create_retained_block_table, try_open_publication_account,
    try_qualify_retained_block_table_plan, unreachable_final_owner_state,
};
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
    StartupAttributionBatchPlans, StartupRetainedAttribution,
};
use crate::{ControlCause, OperationControl, OperationOwner, ProgressPhase, ProgressUnit};
use std::fmt;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};

const FAILURE_WORK_PERMIT_ABANDONED_CODE: u32 = 0x4657_5041;
const FAILURE_WORK_PERMIT_OVERRUN_CODE: u32 = 0x4657_504f;
const ORDINARY_WORK_PERMIT_ABANDONED_CODE: u32 = 0x4f57_5041;
#[cfg(feature = "_u4-activation")]
const ORDINARY_WORK_PERMIT_OVERRUN_CODE: u32 = 0x4f57_504f;

/// Linear owner for one coordinator, one physical account, and its pre-admitted fallback.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[must_use = "the operation owns its coordinator and pre-admitted terminal fallback"]
pub(crate) struct RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(super) coordinator: OperationCoordinator<O, P, PP, U, D>,
    pub(super) baseline: FallbackPublicationBaseline<O, P, D, F>,
    terminal_writer: RetainedWriterPlan<P, D>,
    terminal_account_share_work: RetainedHandleSharePlan<D>,
    failure_terminal_work: Option<FailureTerminalWorkCapability<D>>,
    failure_terminal_work_dimension: Option<D>,
    failure_work_permit_outstanding: bool,
    ordinary_work_permit_outstanding: bool,
    sticky_terminal_cause: Option<PreparationFailureCode>,
    canonical_lineage: CanonicalLineageAuthority,
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
}

/// Records the first trusted terminal cause while keeping a requested cause provisional.
///
/// A requested semantic cause remains authoritative only until the operation observes its first
/// trusted runtime failure. Once Control, ProductGraph, or Ledger is recorded, later causes cannot
/// replace it.
fn record_terminal_cause_with_precedence(
    slot: &mut Option<PreparationFailureCode>,
    cause: PreparationFailureCode,
) {
    match *slot {
        None => *slot = Some(cause),
        Some(existing)
            if existing.class() == PreparationFailureClass::Requested
                && cause.class() != PreparationFailureClass::Requested =>
        {
            *slot = Some(cause);
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
        }
    }
}

impl<O, P, PP, U, D, F> RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(crate) fn record_terminal_cause(&mut self, cause: PreparationFailureCode) {
        record_terminal_cause_with_precedence(&mut self.sticky_terminal_cause, cause);
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn poll_terminal_control(&mut self) {
        let cause = self
            .coordinator
            .poll()
            .err()
            .map(|cause| PreparationFailureCode::from_control(&cause));
        if let Some(cause) = cause {
            self.record_terminal_cause(cause);
        }
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
    sticky_terminal_cause: &'a mut Option<PreparationFailureCode>,
}

/// Poll-and-observed-work authority bound to one prevalidated dimension.
///
/// This private capability is moved into a retained builder or validator closure. It cannot
/// reserve storage, report progress, mutate tables, or enter publication.
pub(crate) struct RetainedWorkControl<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    poll: OperationPollCheckpoint<'a, PP, U>,
    ledger: &'a crate::ledger::Ledger<O, P, D>,
    dimension: D,
    failure_terminal_work_dimension: Option<D>,
    #[allow(
        dead_code,
        reason = "ordinary linear permits are enabled only for the U4 activation consumer"
    )]
    ordinary_work_permit_outstanding: &'a mut bool,
    sticky_terminal_cause: &'a mut Option<PreparationFailureCode>,
}

#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedWorkStartError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Outstanding,
}

#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedWorkConsumeError<D> {
    Ledger(LedgerCause<D>),
    ExceedsAttempted,
}

#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedWorkFinishError {
    Incomplete,
}

/// Linear completion authority for one admitted ordinary work action.
///
/// Admission records the complete attempted envelope before the action begins and confirms that
/// the same consumed prefix still fits. Completed chunks alone advance consumed evidence. Dropping
/// the permit without explicit completion records a product-graph fault.
#[cfg(feature = "_u4-activation")]
#[must_use = "the ordinary work permit must be completed or remain attempted-only"]
pub(crate) struct RetainedWorkPermit<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    control: Option<RetainedWorkControl<'a, O, P, PP, U, D>>,
    remaining: u64,
}

/// Poll-and-work authority bound to the protected failure-terminal dimension.
///
/// Only the retained failure path can construct this capability. The protected dimension never
/// appears in an ordinary checkpoint or caller-selected work control.
pub(crate) struct RetainedFailureWorkControl<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    poll: OperationPollCheckpoint<'a, PP, U>,
    ledger: &'a crate::ledger::Ledger<O, P, D>,
    dimension: D,
    permit_outstanding: &'a mut bool,
    sticky_terminal_cause: &'a mut Option<PreparationFailureCode>,
}

pub(crate) enum RetainedFailureWorkStartError<D> {
    Control(ControlCause),
    Ledger(LedgerCause<D>),
    Outstanding,
}

pub(crate) enum RetainedFailureWorkConsumeError<D> {
    Ledger(LedgerCause<D>),
    ExceedsAttempted,
}

pub(crate) enum RetainedFailureWorkFinishError {
    Incomplete,
}

/// Linear completion authority for one admitted protected failure-work action.
///
/// Creating the permit records the complete attempted work for one count-driven action. Bounded
/// progress calls record only work that actually completes. A semantic rejection may close a
/// partially completed permit explicitly; dropping or forgetting one instead preserves the
/// attempted-only suffix and prevents another protected action from starting.
#[must_use = "the protected failure-work permit must be completed or remain attempted-only"]
pub(crate) struct RetainedFailureWorkPermit<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    control: Option<RetainedFailureWorkControl<'a, O, P, PP, U, D>>,
    remaining: u64,
}

impl<O, P, PP, U, D> RetainedWorkControl<'_, O, P, PP, U, D>
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

    pub(crate) fn charge_observed(&mut self, amount: u64) -> Result<(), LedgerCause<D>> {
        let result = if self.failure_terminal_work_dimension == Some(self.dimension) {
            Err(LedgerCause::UnknownDimension {
                dimension: self.dimension,
            })
        } else {
            self.ledger
                .charge_observed_batch(&[(self.dimension, amount)])
        };
        result.inspect_err(|_| {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        })
    }
}

#[cfg(feature = "_u4-activation")]
impl<'a, O, P, PP, U, D> RetainedWorkControl<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn try_begin(
        mut self,
        amount: NonZeroU64,
    ) -> Result<RetainedWorkPermit<'a, O, P, PP, U, D>, RetainedWorkStartError<D>> {
        if *self.ordinary_work_permit_outstanding {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            return Err(RetainedWorkStartError::Outstanding);
        }
        self.poll().map_err(RetainedWorkStartError::Control)?;
        let result = if self.failure_terminal_work_dimension == Some(self.dimension) {
            Err(LedgerCause::UnknownDimension {
                dimension: self.dimension,
            })
        } else {
            self.ledger
                .try_admit_linear_work(self.dimension, amount.get())
        };
        if let Err(cause) = result {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            return Err(RetainedWorkStartError::Ledger(cause));
        }
        *self.ordinary_work_permit_outstanding = true;
        Ok(RetainedWorkPermit {
            control: Some(self),
            remaining: amount.get(),
        })
    }
}

#[cfg(feature = "_u4-activation")]
impl<'a, O, P, PP, U, D> RetainedWorkPermit<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.control
            .as_mut()
            .expect("an incomplete ordinary work permit retains its control")
            .poll()
    }

    pub(crate) const fn remaining_units(&self) -> u64 {
        self.remaining
    }

    pub(crate) fn complete_units(
        &mut self,
        amount: NonZeroU64,
    ) -> Result<(), RetainedWorkConsumeError<D>> {
        if amount.get() > self.remaining {
            let control = self
                .control
                .as_mut()
                .expect("an incomplete ordinary work permit retains its control");
            record_terminal_cause_with_precedence(
                control.sticky_terminal_cause,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_OVERRUN_CODE),
            );
            return Err(RetainedWorkConsumeError::ExceedsAttempted);
        }
        let control = self
            .control
            .as_mut()
            .expect("an incomplete ordinary work permit retains its control");
        if let Err(cause) = control
            .ledger
            .charge_consumed(control.dimension, amount.get())
        {
            record_terminal_cause_with_precedence(
                control.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            return Err(RetainedWorkConsumeError::Ledger(cause));
        }
        self.remaining -= amount.get();
        Ok(())
    }

    pub(crate) fn complete(
        mut self,
    ) -> Result<RetainedWorkControl<'a, O, P, PP, U, D>, RetainedWorkFinishError> {
        if self.remaining != 0 {
            let control = self
                .control
                .as_mut()
                .expect("an incomplete ordinary work permit retains its control");
            record_terminal_cause_with_precedence(
                control.sticky_terminal_cause,
                PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
            );
            return Err(RetainedWorkFinishError::Incomplete);
        }
        let control = self
            .control
            .take()
            .expect("an ordinary work permit completes at most once");
        *control.ordinary_work_permit_outstanding = false;
        Ok(control)
    }
}

#[cfg(feature = "_u4-activation")]
impl<O, P, PP, U, D> Drop for RetainedWorkPermit<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        record_terminal_cause_with_precedence(
            control.sticky_terminal_cause,
            PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
        );
    }
}

impl<'a, O, P, PP, U, D> RetainedFailureWorkControl<'a, O, P, PP, U, D>
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

    pub(crate) fn try_begin(
        mut self,
        amount: NonZeroU64,
    ) -> Result<RetainedFailureWorkPermit<'a, O, P, PP, U, D>, RetainedFailureWorkStartError<D>>
    {
        if *self.permit_outstanding {
            self.record_product_graph_failure(FAILURE_WORK_PERMIT_ABANDONED_CODE);
            return Err(RetainedFailureWorkStartError::Outstanding);
        }
        self.poll()
            .map_err(RetainedFailureWorkStartError::Control)?;
        if let Err(cause) = self.ledger.charge_attempted(self.dimension, amount.get()) {
            record_terminal_cause_with_precedence(
                self.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            return Err(RetainedFailureWorkStartError::Ledger(cause));
        }
        *self.permit_outstanding = true;
        Ok(RetainedFailureWorkPermit {
            control: Some(self),
            remaining: amount.get(),
        })
    }

    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        record_terminal_cause_with_precedence(
            self.sticky_terminal_cause,
            PreparationFailureCode::product_graph(code),
        );
    }
}

impl<'a, O, P, PP, U, D> RetainedFailureWorkPermit<'a, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn poll(&mut self) -> Result<(), ControlCause> {
        self.control
            .as_mut()
            .expect("an incomplete failure-work permit retains its control")
            .poll()
    }

    pub(crate) const fn remaining_units(&self) -> u64 {
        self.remaining
    }

    pub(crate) fn record_product_graph_failure(&mut self, code: u32) {
        self.control
            .as_mut()
            .expect("an incomplete failure-work permit retains its control")
            .record_product_graph_failure(code);
    }

    pub(crate) fn complete_units(
        &mut self,
        amount: NonZeroU64,
    ) -> Result<(), RetainedFailureWorkConsumeError<D>> {
        if amount.get() > self.remaining {
            let control = self
                .control
                .as_mut()
                .expect("an incomplete failure-work permit retains its control");
            control.record_product_graph_failure(FAILURE_WORK_PERMIT_OVERRUN_CODE);
            return Err(RetainedFailureWorkConsumeError::ExceedsAttempted);
        }
        let control = self
            .control
            .as_mut()
            .expect("an incomplete failure-work permit retains its control");
        if let Err(cause) = control
            .ledger
            .charge_consumed(control.dimension, amount.get())
        {
            record_terminal_cause_with_precedence(
                control.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
            return Err(RetainedFailureWorkConsumeError::Ledger(cause));
        }
        self.remaining -= amount.get();
        Ok(())
    }

    pub(crate) fn complete(
        mut self,
    ) -> Result<RetainedFailureWorkControl<'a, O, P, PP, U, D>, RetainedFailureWorkFinishError>
    {
        if self.remaining != 0 {
            let control = self
                .control
                .as_mut()
                .expect("an incomplete failure-work permit retains its control");
            control.record_product_graph_failure(FAILURE_WORK_PERMIT_ABANDONED_CODE);
            return Err(RetainedFailureWorkFinishError::Incomplete);
        }
        let control = self
            .control
            .take()
            .expect("a failure-work permit completes at most once");
        *control.permit_outstanding = false;
        Ok(control)
    }

    pub(crate) fn finish_validation(
        mut self,
        accepted: bool,
    ) -> Result<RetainedFailureWorkControl<'a, O, P, PP, U, D>, RetainedFailureWorkFinishError>
    {
        if accepted {
            return self.complete();
        }
        let control = self
            .control
            .take()
            .expect("a validation work permit resolves at most once");
        *control.permit_outstanding = false;
        Ok(control)
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
impl<O, P, PP, U, D, F> RetainedOperation<O, P, PP, U, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(crate) fn control_checkpoint(&mut self) -> RetainedControlCheckpoint<'_, O, P, PP, U, D> {
        let Self {
            coordinator,
            failure_terminal_work_dimension,
            sticky_terminal_cause,
            ..
        } = self;
        let (poll, ledger) = coordinator.split_poll_checkpoint_and_retained_ledger();
        RetainedControlCheckpoint {
            poll,
            ledger,
            failure_terminal_work_dimension: *failure_terminal_work_dimension,
            sticky_terminal_cause,
        }
    }

    pub(crate) fn split_work_control(
        &mut self,
        dimension: D,
    ) -> (
        RetainedWorkControl<'_, O, P, PP, U, D>,
        &crate::ledger::Ledger<O, P, D>,
    ) {
        let Self {
            coordinator,
            failure_terminal_work_dimension,
            ordinary_work_permit_outstanding,
            sticky_terminal_cause,
            ..
        } = self;
        let (poll, ledger) = coordinator.split_poll_checkpoint_and_retained_ledger();
        (
            RetainedWorkControl {
                poll,
                ledger,
                dimension,
                failure_terminal_work_dimension: *failure_terminal_work_dimension,
                ordinary_work_permit_outstanding,
                sticky_terminal_cause,
            },
            ledger,
        )
    }

    pub fn contains_dimension(&self, dimension: D) -> bool {
        self.failure_terminal_work_dimension != Some(dimension)
            && self
                .coordinator
                .retained_ledger()
                .contains_dimension(dimension)
    }

    pub fn contains_phase(&self, phase: P) -> bool {
        self.coordinator.retained_ledger().contains_phase(phase)
    }

    /// Records semantic completion against the operation's frozen phase catalog.
    pub(crate) fn try_complete_phase(&self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        self.coordinator.retained_ledger().try_complete_phase(phase)
    }

    pub fn try_start(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        let phase_catalog = [plan.terminal_writer.phase];
        Self::try_start_inner(
            control,
            ledger_limits,
            &phase_catalog,
            account,
            plan,
            CanonicalLineageAuthority::fresh(),
            build_fallback,
        )
    }

    pub fn try_start_with_phase_catalog(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: &[P],
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        Self::try_start_inner(
            control,
            ledger_limits,
            phase_catalog,
            account,
            plan,
            CanonicalLineageAuthority::fresh(),
            build_fallback,
        )
    }

    pub(crate) fn try_start_derived_with_phase_catalog<CanonicalBlock>(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: &[P],
        account: &PhysicalStorageAccount,
        source: &PublishedCanonicalAggregate<CanonicalBlock>,
        plan: RetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        if !source.belongs_to(account) {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        Self::try_start_inner(
            control,
            ledger_limits,
            phase_catalog,
            account,
            plan,
            CanonicalLineageAuthority::derived(source),
            build_fallback,
        )
    }

    fn try_start_inner(
        control: OperationControl<PP, U>,
        ledger_limits: LedgerLimits<D>,
        phase_catalog: &[P],
        account: &PhysicalStorageAccount,
        plan: RetainedOperationStartPlan<P, D>,
        canonical_lineage: CanonicalLineageAuthority,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> Result<Self, RetainedOperationStartError<D>> {
        if plan.terminal_writer.limits.attribution_slots == 0
            || plan.terminal_writer.limits.commitment_slots == 0
        {
            return Err(RetainedOperationStartError::InvalidTerminalLimits);
        }
        let failure_terminal_work = match plan.failure_terminal_work {
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
                Some((failure_plan, requirement))
            }
            None => None,
        };
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
        let dimension_count = ledger_limits.dimension_count();
        let terminal_evidence_backing =
            TerminalEvidenceBackingPlan::<P, D>::try_new(dimension_count, phase_catalog.len())
                .map_err(|_| {
                    RetainedOperationStartError::Writer(RetainedWriterError::BatchCapacity)
                })?;
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
        let startup_plan = ledger_limits
            .try_qualify_startup(
                StartupAttributionBatchPlans::new(
                    canonical_assembly_plan,
                    evidence_assembly_plan,
                    fallback_batch_plan,
                    evidence_batch_plan,
                ),
                [
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
                StartupAccountShareEnvelope::four_identical(
                    plan.terminal_account_share_work.charges(),
                ),
                [plan.fallback_commitment, plan.snapshot_commitment],
            )
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
            })?;
        let physical_manifest = StartupPhysicalManifest::try_new(fallback_charge, evidence_charge)
            .map_err(|_| {
                RetainedOperationStartError::Writer(RetainedWriterError::Physical(
                    RetainedAllocationError::AccountingOverflow,
                ))
            })?;
        let cross_plane =
            StartupCrossPlanePlan::try_new(startup_plan, physical_manifest).map_err(|_| {
                RetainedOperationStartError::Writer(RetainedWriterError::OwnershipUnverified)
            })?;
        let ready = account
            .try_prepare_startup_cross_plane(phase_catalog, cross_plane)
            .map_err(map_startup_cross_plane_error)?;
        #[cfg(test)]
        STARTUP_READY_FINAL_OWNER_ALLOCATOR_ENTRIES.with(|entries| {
            entries.set(Some(test_retained_final_owner_allocator_entries()));
        });
        let (started_ledger, baseline, start_guard) = ready.split();
        let startup_guard =
            StartupRetainedPartGuard::new(start_guard, baseline, plan.fallback_plan, evidence_plan);
        let failure_terminal_work = match failure_terminal_work {
            Some((failure_plan, requirement)) => {
                started_ledger
                    .preflight_repeated_observed_batch(
                        &failure_plan.share_work().charges(),
                        requirement.maximum_share_actions(),
                    )
                    .map_err(|cause| {
                        RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
                    })?;
                Some(FailureTerminalWorkCapability::from_qualified(failure_plan))
            }
            None => None,
        };
        let coordinator = OperationCoordinator::from_started_ledger(control, started_ledger);
        let ledger = coordinator.retained_ledger();
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
            .try_bind(ledger)
            .map_err(RetainedOperationStartError::Writer)?;

        Ok(Self {
            coordinator,
            baseline,
            terminal_writer: plan.terminal_writer,
            terminal_account_share_work: plan.terminal_account_share_work,
            failure_terminal_work,
            failure_terminal_work_dimension,
            failure_work_permit_outstanding: false,
            ordinary_work_permit_outstanding: false,
            sticky_terminal_cause: None,
            canonical_lineage,
        })
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.baseline.account()
    }

    pub(super) fn activate_failure_terminal_work(&mut self) -> Option<RetainedHandleSharePlan<D>> {
        self.failure_terminal_work
            .take()
            .map(|capability| capability.handle_share_work)
    }

    pub(crate) fn failure_terminal_work_limits(&self) -> Option<(usize, u64, u64)> {
        self.failure_terminal_work.as_ref().map(|capability| {
            (
                capability.max_evidence_members,
                capability.construction_work_limit,
                capability.validation_work_limit,
            )
        })
    }

    pub(super) fn split_failure_terminal_work_control(
        &mut self,
        work: RetainedHandleSharePlan<D>,
    ) -> (
        RetainedFailureWorkControl<'_, O, P, PP, U, D>,
        &crate::ledger::Ledger<O, P, D>,
    ) {
        let [(dimension, _), (paired_dimension, _)] = work.charges();
        if dimension != paired_dimension {
            self.record_product_graph_failure(0x4657_444d);
        }
        let Self {
            coordinator,
            failure_work_permit_outstanding,
            sticky_terminal_cause,
            ..
        } = self;
        let (poll, ledger) = coordinator.split_terminal_poll_checkpoint_and_retained_ledger();
        (
            RetainedFailureWorkControl {
                poll,
                ledger,
                dimension,
                permit_outstanding: failure_work_permit_outstanding,
                sticky_terminal_cause,
            },
            ledger,
        )
    }

    pub(super) const fn sticky_terminal_cause(&self) -> Option<PreparationFailureCode> {
        let permit_outstanding =
            self.failure_work_permit_outstanding || self.ordinary_work_permit_outstanding;
        let abandoned_code = if self.failure_work_permit_outstanding {
            FAILURE_WORK_PERMIT_ABANDONED_CODE
        } else {
            ORDINARY_WORK_PERMIT_ABANDONED_CODE
        };
        match (self.sticky_terminal_cause, permit_outstanding) {
            (Some(cause), true) if matches!(cause.class(), PreparationFailureClass::Requested) => {
                Some(PreparationFailureCode::product_graph(abandoned_code))
            }
            (None, true) => Some(PreparationFailureCode::product_graph(abandoned_code)),
            (cause, _) => cause,
        }
    }

    pub(super) fn record_ledger_failure(&mut self) {
        self.record_terminal_cause(PreparationFailureCode::ledger());
    }

    pub(super) fn record_product_graph_failure(&mut self, code: u32) {
        self.record_terminal_cause(PreparationFailureCode::product_graph(code));
    }

    pub(super) const fn has_failure_terminal_work_configuration(&self) -> bool {
        self.failure_terminal_work_dimension.is_some()
    }

    pub(crate) const fn is_terminal_only(&self) -> bool {
        self.failure_terminal_work_dimension.is_some() && self.failure_terminal_work.is_none()
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

    /// Freezes the pre-admitted terminal baseline without opening a block-table assembly.
    ///
    /// This path performs no allocation, table construction, root validation, control callback, or
    /// other fallible action. It is therefore available after every recoverable assembly-open
    /// failure. Fail-closed open errors deliberately do not return the operation and cannot reach
    /// this method.
    #[must_use = "the prepared fallback must be handed off or deliberately dropped"]
    pub fn prepare_minimal_fallback<CanonicalBlock, EvidenceBlock>(
        self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        let cause = self.sticky_terminal_cause().unwrap_or(cause);
        let RetainedOperation {
            coordinator,
            baseline,
            ..
        } = self;
        let ledger = coordinator.into_ledger_for_typed_publication();
        let (fallback, evidence, batches) = baseline.into_parts();
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

    pub fn try_publication_assembly<CanonicalBlock, EvidenceBlock>(
        mut self,
        mut plan: RetainedPublicationPlan<P, D>,
    ) -> Result<
        RetainedPublicationAssembly<O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
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
        if let Some(capability) = self.failure_terminal_work.as_ref()
            && plan.outcome_evidence.total_blocks() > capability.max_evidence_members
        {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        if self.is_terminal_only() {
            return Err(
                self.into_publication_open_error(RetainedWriterError::InvalidPublicationPlan, true)
            );
        }
        if let Some(capacity) = self.canonical_lineage.source_mapping_capacity()
            && let Some(canonical) = plan.canonical.as_mut()
        {
            *canonical = canonical.with_canonical_source_mapping_capacity(capacity);
        }
        let terminal_limits = self.terminal_writer.limits;
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
                terminal_limits,
                RetainedMembershipDestination::Canonical,
            ) {
                Ok(qualified) => Some(qualified),
                Err(cause) => {
                    return Err(self.into_publication_open_error(cause, true));
                }
            },
            None => None,
        };
        let ordinary_handle_share_work = self.terminal_account_share_work;
        let (
            open_handle_share_work,
            failure_evidence_handle_share_work,
            failure_terminal_work_state,
        ) = if failure_only {
            self.failure_terminal_work.take().map_or(
                (
                    ordinary_handle_share_work,
                    None,
                    FailureTerminalWorkState::Inactive,
                ),
                |capability| {
                    (
                        capability.handle_share_work,
                        Some(capability.handle_share_work),
                        FailureTerminalWorkState::Active,
                    )
                },
            )
        } else {
            (
                ordinary_handle_share_work,
                None,
                FailureTerminalWorkState::Inactive,
            )
        };
        let open_result = catch_unwind(AssertUnwindSafe(|| {
            try_open_publication_account(
                self.coordinator.retained_ledger(),
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
                self.coordinator.retained_ledger(),
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
            self.coordinator.retained_ledger(),
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
        let canonical_lineage = self.canonical_lineage;
        Ok(RetainedPublicationAssembly {
            canonical_table,
            evidence_table: Some(evidence_table),
            canonical_batch,
            evidence_batch: Some(evidence_batch),
            account: Some(account),
            operation: Some(self),
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
        StartupCrossPlaneAdmissionFailure::OperationStart(cause) => match cause {
            startup::OperationStartAdmissionError::AccountPoisoned(poisoned) => {
                RetainedWriterError::AccountPoisoned(poisoned)
            }
            startup::OperationStartAdmissionError::AccountingOverflow => {
                RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow)
            }
        },
        StartupCrossPlaneAdmissionFailure::LedgerInitialization(cause) => {
            RetainedWriterError::Ledger(cause)
        }
        StartupCrossPlaneAdmissionFailure::TerminalEvidenceCatalogInvariant => {
            return RetainedOperationStartError::TerminalEvidenceCatalogInvariant;
        }
        StartupCrossPlaneAdmissionFailure::LedgerAdmission(cause) => map_batch_error(cause),
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
