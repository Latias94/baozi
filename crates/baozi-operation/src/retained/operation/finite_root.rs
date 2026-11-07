//! Concrete finite invocation ownership; ordinary permissions only borrow its actual slots.

use super::*;
use crate::ledger::finite::FiniteLedgerLimits;
use crate::retained::operation_start::{
    FiniteRetainedOperationStartPlan, FiniteStartupRetainedResidue, QualifiedFailureTerminalWork,
};
use crate::retained::ownership::FiniteFallbackPublicationBaseline;
use crate::retained::startup::{
    FiniteStartupAdmissionCause, FiniteStartupCrossPlaneAdmissionFailure,
    FiniteStartupCrossPlaneContainment, FiniteStartupCrossPlanePlan,
};

mod start;
mod terminal;
pub(crate) use terminal::{FiniteRootTerminalFailure, FiniteRootTerminalRejection};
mod remainder;
pub(crate) use remainder::FiniteInvocationRemainder;
#[cfg(test)]
mod tests;

pub(crate) struct FiniteRetainedOperationFields<O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    baseline: FiniteFallbackPublicationBaseline<O, P, D, F>,
    metadata: RetainedOperationMetadata<P, D, CB, EB>,
}

impl<'inv, O, P, D, F, CB, EB> From<&'inv mut FiniteRetainedOperationFields<O, P, D, F, CB, EB>>
    for RetainedOperationFieldsLoan<'inv, O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn from(fields: &'inv mut FiniteRetainedOperationFields<O, P, D, F, CB, EB>) -> Self {
        Self {
            baseline: FallbackPublicationBaselineLoan::Finite(&mut fields.baseline),
            metadata: &mut fields.metadata,
        }
    }
}

/// The sole concrete numerical and physical owner of a finite invocation.
pub(crate) struct FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
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
    terminal: Option<super::super::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: super::super::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    common: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    started: bool,
    contained: bool,
}

/// Entry-owned admission. A pending permission borrows this guard; it never owns it.
pub(crate) struct FiniteEntryAdmission<'account> {
    account: &'account PhysicalStorageAccount,
    pending: Option<startup::PendingOperationStartAdmission<'account>>,
    identity: Option<*const ()>,
}

impl<'account> FiniteEntryAdmission<'account> {
    pub(crate) const fn new(account: &'account PhysicalStorageAccount) -> Self {
        Self {
            account,
            pending: None,
            identity: None,
        }
    }

    pub(crate) const fn account(&self) -> &'account PhysicalStorageAccount {
        self.account
    }
}

/// This token owns no admission, control owner, numerical state or account share.
pub(crate) struct FiniteDeferredUnattachedBinding {
    // An opaque address comparison only; this token never dereferences or reconstructs a pointer.
    identity: usize,
}

#[expect(
    clippy::large_enum_variant,
    reason = "normal rejection transfers its actual finite owner inline without a failure-path allocation"
)]
pub(crate) enum FiniteRootUnattachedFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    Rejected(FiniteDeferredUnattachedBinding),
    Owned(invocation::FiniteUnattachedFailure<O, P, PP, U, D>),
}

type OptionalOperationLoan<'inv, O, P, PP, U, D, F, CB, EB> =
    Option<RetainedOperationLoan<'inv, O, P, PP, U, D, F, CB, EB>>;
type UnattachedResult<O, P, PP, U, D> =
    Result<invocation::FiniteUnattachedTerminal<P, D>, FiniteRootUnattachedFailure<O, P, PP, U, D>>;

/// All fields are original root references; no field points into another owned field.
pub(crate) struct FiniteInvocationRootFields<'inv, O, P, PP, U, D, F, CB, EB>
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
    terminal: &'inv mut Option<super::super::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: &'inv mut super::super::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: &'inv mut Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    common: &'inv mut Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    started: &'inv mut bool,
    contained: &'inv mut bool,
}

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
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
    pub(crate) const fn new() -> Self {
        Self {
            terminal: None,
            assembly: None,
            retained: None,
            common: None,
            started: false,
            contained: false,
        }
    }

    pub(crate) fn fields(&mut self) -> FiniteInvocationRootFields<'_, O, P, PP, U, D, F, CB, EB> {
        FiniteInvocationRootFields {
            terminal: &mut self.terminal,
            assembly: &mut self.assembly,
            retained: &mut self.retained,
            common: &mut self.common,
            started: &mut self.started,
            contained: &mut self.contained,
        }
    }

    pub(crate) fn loan(&mut self) -> OptionalOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(&mut self.assembly),
            terminal: TerminalPhysicalSlot::Finite(&mut self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    pub(crate) fn has_started(&self) -> bool {
        self.started
    }

    pub(crate) fn must_retain_domain(&self) -> bool {
        self.contained
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        std::ptr::from_ref(&self.common).cast()
    }

    pub(crate) fn terminal_catalog(&self) -> Option<crate::ledger::LedgerReadView<'_, O, P, D>> {
        self.common
            .as_ref()
            .map(|common| common.coordinator.retained_ledger().as_read())
    }

    pub(crate) fn record_payload_failure(&mut self, code: u32) {
        if let Some(common) = self.common.as_mut() {
            reject_retained_attachment(
                &mut common.view(),
                PreparationFailureCode::product_graph(code),
            );
        }
    }

    pub(crate) fn cleanup_owned_state(&mut self) {
        let retain = remainder::cleanup_finite_physical_state(
            &mut self.terminal,
            &mut self.assembly,
            &mut self.retained,
            Some(&mut self.common),
            &mut self.contained,
        );
        if retain {
            std::mem::forget(self.common.take());
        } else {
            let unresolved = InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            }
            .close_abandoned_common();
            if unresolved {
                std::mem::forget(self.common.take());
            }
        }
    }

    /// Runs after body captures are disposed and consumes this entry's original admission.
    pub(crate) fn finish_unattached(
        &mut self,
        binding: FiniteDeferredUnattachedBinding,
        entry: &mut FiniteEntryAdmission<'_>,
        final_progress: FinalProgress<PP, U>,
    ) -> UnattachedResult<O, P, PP, U, D> {
        if binding.identity != self.invocation_identity().addr()
            || entry.identity.map(<*const ()>::addr) != Some(binding.identity)
            || self.retained.is_some()
            || self.assembly.is_some()
            || self.terminal.is_some()
            || self.common.is_none()
            || entry.pending.is_none()
        {
            return Err(FiniteRootUnattachedFailure::Rejected(binding));
        }
        let admission = entry
            .pending
            .take()
            .expect("the original entry admission was checked");
        // Preserve outer domain duty if fixed capture or control disposal unwinds.
        let already_contained = self.contained;
        self.contained = true;
        let common = self
            .common
            .take()
            .expect("unattached completion takes the actual finite owner");
        match common.prepare_and_finish_unattached(admission, final_progress) {
            Ok(prepared) => {
                self.contained = already_contained;
                Ok(prepared)
            }
            Err(error) => Err(FiniteRootUnattachedFailure::Owned(error)),
        }
    }
}

impl<O, P, PP, U, D, F, CB, EB> Drop for FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
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
    fn drop(&mut self) {
        self.cleanup_owned_state();
    }
}

impl<'inv, O, P, PP, U, D, F, CB, EB> FiniteInvocationRootFields<'inv, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn loan(&mut self) -> OptionalOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(self.assembly),
            terminal: TerminalPhysicalSlot::Finite(self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(self.common),
                contained: self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    pub(crate) fn into_loan(self) -> OptionalOperationLoan<'inv, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(self.assembly),
            terminal: TerminalPhysicalSlot::Finite(self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(self.common),
                contained: self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    /// Transfers original owners to a normal error; the source shell stays irreversibly spent.
    pub(crate) fn into_owned(self) -> FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB> {
        let contained = *self.contained;
        *self.started = true;
        *self.contained = false;
        FiniteInvocationRoot {
            terminal: self.terminal.take(),
            assembly: self.assembly.take(),
            retained: self.retained.take(),
            common: self.common.take(),
            started: true,
            contained,
        }
    }
}

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRootFields<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    fn deferred_fields_are_available(&mut self) -> bool {
        if *self.contained || self.terminal.is_some() {
            return false;
        }
        let Some(operation) = self.loan() else {
            return false;
        };
        operation.retained.has_deferred_storage()
            && !operation.retained.metadata.failure_work_permit_outstanding
            && !operation.retained.metadata.ordinary_work_permit_outstanding
            && !operation.common.terminal_is_outstanding()
            && operation
                .retained
                .baseline
                .first_account()
                .is_some_and(|account| !account.snapshot().poisoned())
    }

    fn defer_assembly_stage(
        &mut self,
        stage: RetainedDeferredStage<CB, EB>,
    ) -> Result<(), RetainedDeferredStage<CB, EB>> {
        if !self.deferred_fields_are_available() {
            return Err(stage);
        }
        let Some(raw) = self.assembly_loan() else {
            return Err(stage);
        };
        let mut assembly = super::super::RetainedPublicationAssemblyBorrow::from_loan(raw);
        if !assembly.loan().has_exact_storage_graph() {
            return Err(stage);
        }
        if let RetainedDeferredStage::Success { proof } = &stage
            && !assembly.loan().success_proof_is_current(proof)
        {
            return Err(stage);
        }
        match assembly.loan_mut().prepare_body_deferral() {
            Some(Ok(())) => {}
            Some(Err(_)) => {
                if matches!(&stage, RetainedDeferredStage::Success { .. })
                    && let Some(operation) = assembly.loan_mut().operation.as_mut()
                {
                    operation.record_ledger_failure();
                }
                return Err(stage);
            }
            None => return Err(stage),
        }
        assembly.loan_mut().stage_prepared_body_deferral(stage);
        Ok(())
    }

    pub(crate) fn defer_operation_until_body_cleanup(&mut self) -> bool {
        if self.assembly.is_some() || !self.deferred_fields_are_available() {
            return false;
        }
        self.loan()
            .is_some_and(|operation| operation.defer_until_body_cleanup().is_ok())
    }

    pub(crate) fn defer_assembly_until_body_cleanup(&mut self) -> bool {
        self.defer_assembly_stage(RetainedDeferredStage::Assembly)
            .is_ok()
    }

    pub(crate) fn defer_failure_until_body_cleanup(
        &mut self,
        fallback_cause: PreparationFailureCode,
    ) -> bool {
        self.defer_assembly_stage(RetainedDeferredStage::Failure { fallback_cause })
            .is_ok()
    }

    pub(crate) fn defer_success_until_body_cleanup(
        &mut self,
        proof: super::super::RetainedSuccessClosureProof<CB, EB>,
    ) -> Result<(), super::super::RetainedSuccessClosureProof<CB, EB>> {
        match self.defer_assembly_stage(RetainedDeferredStage::Success { proof }) {
            Ok(()) => Ok(()),
            Err(RetainedDeferredStage::Success { proof }) => Err(proof),
            Err(_) => unreachable!("success staging returns its original uninstalled proof"),
        }
    }

    pub(crate) fn defer_fallback_until_body_cleanup(
        &mut self,
        cause: PreparationFailureCode,
        activation_containment_required: bool,
    ) -> bool {
        self.defer_assembly_stage(RetainedDeferredStage::Fallback {
            cause,
            activation_containment_required,
        })
        .is_ok()
    }

    fn deferred_assembly_is_available(&mut self) -> bool {
        if !self.deferred_fields_are_available() {
            return false;
        }
        let Some(raw) = self.assembly_loan() else {
            return false;
        };
        let assembly = super::super::RetainedPublicationAssemblyBorrow::from_loan(raw);
        let loan = assembly.loan();
        let Some(operation) = loan.operation.as_ref() else {
            return false;
        };
        loan.fields.has_deferred_storage()
            && loan.has_exact_storage_graph()
            && operation.common.as_read().ordinary_work_is_sealed()
            && loan
                .fields
                .account
                .as_ref()
                .zip(operation.retained.baseline.first_account())
                .is_some_and(|(account, baseline)| account.is_same_account(baseline))
            && match operation.retained.metadata.deferred_stage.as_ref() {
                Some(RetainedDeferredStage::Success { proof }) => {
                    loan.success_proof_is_current(proof)
                }
                Some(_) => true,
                None => false,
            }
    }

    pub(crate) fn resume_deferred_operation(&mut self) -> bool {
        if self.assembly.is_some() || !self.deferred_fields_are_available() {
            return false;
        }
        let Some(operation) = self.loan() else {
            return false;
        };
        if !operation.common.as_read().ordinary_work_is_sealed()
            || !matches!(
                operation.retained.metadata.deferred_stage,
                Some(RetainedDeferredStage::Operation)
            )
        {
            return false;
        }
        operation.retained.metadata.deferred_stage = Some(RetainedDeferredStage::TerminalOperation);
        true
    }

    pub(crate) fn resume_deferred_assembly(&mut self) -> bool {
        if !self.deferred_assembly_is_available() {
            return false;
        }
        let Some(retained) = self.retained.as_mut() else {
            return false;
        };
        if !matches!(
            retained.metadata.deferred_stage,
            Some(RetainedDeferredStage::Assembly)
        ) {
            return false;
        }
        retained.metadata.deferred_stage = None;
        true
    }

    pub(crate) fn resume_deferred_failure(&mut self) -> Option<PreparationFailureCode> {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let Some(RetainedDeferredStage::Failure { fallback_cause }) =
            retained.metadata.deferred_stage.as_ref()
        else {
            return None;
        };
        let fallback_cause = *fallback_cause;
        retained.metadata.deferred_stage = None;
        Some(fallback_cause)
    }

    pub(crate) fn resume_deferred_success(
        &mut self,
    ) -> Option<super::super::RetainedSuccessClosureProof<CB, EB>> {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        if !matches!(
            retained.metadata.deferred_stage,
            Some(RetainedDeferredStage::Success { .. })
        ) {
            return None;
        }
        match retained.metadata.deferred_stage.take()? {
            RetainedDeferredStage::Success { proof } => Some(proof),
            _ => unreachable!("the checked success stage remains installed until restoration"),
        }
    }

    pub(crate) fn resume_deferred_fallback(&mut self) -> Option<(PreparationFailureCode, bool)> {
        if !self.deferred_assembly_is_available() {
            return None;
        }
        let retained = self.retained.as_mut()?;
        let Some(RetainedDeferredStage::Fallback {
            cause,
            activation_containment_required,
        }) = retained.metadata.deferred_stage.as_ref()
        else {
            return None;
        };
        let fallback = (*cause, *activation_containment_required);
        retained.metadata.deferred_stage = None;
        Some(fallback)
    }

    pub(crate) fn contain_deferred_fault(&mut self) -> Option<FinalizationContainmentError> {
        if let Some(raw) = self.assembly_loan() {
            let mut assembly = super::super::RetainedPublicationAssemblyBorrow::from_loan(raw);
            if assembly.loan().first_containment_account().is_some() {
                return Some(assembly.loan_mut().contain_activation_fault());
            }
        }
        if let Some(operation) = self
            .loan()
            .filter(|operation| operation.retained.baseline.first_account().is_some())
        {
            return Some(operation.contain_activation_fault());
        }
        let account = self.terminal.as_ref()?.graph.account.as_ref()?;
        let reason = account
            .snapshot()
            .first_poison_evidence()
            .map_or(AccountPoisonReason::OwnershipGraphUnverified, |evidence| {
                evidence.reason()
            });
        let poisoned = account.record_poison(reason);
        *self.contained = true;
        Some(FinalizationContainmentError::new(poisoned, reason))
    }
}

pub(crate) struct FinitePendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CB, EB>
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
    root: &'inv mut FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>,
    entry: &'inv mut FiniteEntryAdmission<'account>,
    plan: FiniteRetainedOperationStartPlan<P, D>,
    failure_terminal_work: QualifiedFailureTerminalWork<D>,
    local: PhantomData<Rc<()>>,
}

/// Startup rejection keeps the actual owner from the stage that failed.
pub(crate) struct FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) cause: RetainedOperationStartError<D>,
    residue: FiniteAttachmentResidue<O, P, D, F>,
}

#[expect(
    clippy::large_enum_variant,
    reason = "successive startup stages return their original physical owner without allocating on rejection"
)]
enum FiniteAttachmentResidue<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    None,
    CrossPlane {
        _owner: FiniteStartupCrossPlaneContainment<O, P, D>,
    },
    Retained {
        _owner: FiniteStartupRetainedResidue<O, P, D, F>,
    },
}

impl<O, P, D, F> fmt::Debug for FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteAttachmentFailure")
            .field("cause", &self.cause)
            .field(
                "retains_cross_plane_owners",
                &matches!(&self.residue, FiniteAttachmentResidue::CrossPlane { .. }),
            )
            .field(
                "retains_construction_owners",
                &matches!(&self.residue, FiniteAttachmentResidue::Retained { .. }),
            )
            .finish()
    }
}

impl<O, P, D, F> FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn clean(cause: RetainedOperationStartError<D>) -> Self {
        Self {
            cause,
            residue: FiniteAttachmentResidue::None,
        }
    }

    pub(crate) fn has_residue(&self) -> bool {
        !matches!(&self.residue, FiniteAttachmentResidue::None)
    }
}
