//! A terminal decision stays beside the original invocation until concrete extraction.

use super::*;
use crate::ledger::{TypedTerminalLayout, TypedTerminalRoles};
use crate::retained::terminal_physical::TerminalPhysicalSlots;
use crate::retained::{
    DeferredAttachedTerminal, DynamicTerminalCapsule, RetainedJointCommitFormationWithControl,
    RetainedPublicationAssembly, RetainedPublicationAssemblyLoan,
    RetainedSuccessJointCommitFormation, RetainedTerminalRequest, TerminalDisposition,
    TerminalPhysicalState,
};

pub(crate) struct AttachedTerminalDecision<P> {
    pub(crate) last_completed_phase: Option<P>,
    pub(crate) layout: TypedTerminalLayout,
    pub(crate) roles: TypedTerminalRoles,
}

/// Both concrete roots select the same original batch roles after borrowed cleanup has ended.
pub(super) fn select_terminal_batches<'loan, O, P, D, CB, EB>(
    decision: &AttachedTerminalDecision<P>,
    baseline: crate::ledger::FallbackBatchView<'loan, O, P, D>,
    primary: Option<&'loan mut super::super::super::PublicationAssemblyFields<O, P, D, CB, EB>>,
) -> crate::ledger::TypedTerminalBatches<'loan, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (canonical_source, evidence_source) = match primary {
        Some(primary) => (
            primary.canonical_batch.as_mut(),
            primary.evidence_batch.as_mut(),
        ),
        None => (None, None),
    };
    let deferred_canonical_storage = if decision.roles.deferred_canonical {
        Some(canonical_source.expect("the ready decision retains its original canonical source"))
    } else {
        None
    };
    let deferred_evidence_storage_first = if decision.roles.deferred_evidence_first {
        Some(
            baseline
                .numerical_evidence
                .expect("qualification retains its original numerical evidence source"),
        )
    } else {
        None
    };
    let deferred_evidence_storage_second = if decision.roles.deferred_evidence_second {
        Some(match decision.layout {
            TypedTerminalLayout::Fallback => baseline.fallback,
            TypedTerminalLayout::Primary => {
                evidence_source.expect("the ready decision retains its original outcome source")
            }
        })
    } else {
        None
    };
    crate::ledger::TypedTerminalBatches {
        canonical: baseline.canonical_assembly,
        evidence: baseline.evidence_assembly,
        deferred_canonical_storage,
        deferred_evidence_storage_first,
        deferred_evidence_storage_second,
        discard_canonical: None,
        discard_evidence: None,
    }
}

/// A once-only ticket for the decision held by one actual invocation.
pub(crate) struct AttachedTerminalStamp {
    identity: crate::ledger::LedgerIdentity,
}

/// Exclusive original fields survive a shorter qualification loan without moving an owner.
pub(in crate::retained) struct DynamicTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    baseline: &'inv mut FallbackPublicationBaseline<O, P, D, F>,
    metadata: &'inv mut RetainedOperationMetadata<P, D, CB, EB>,
    terminal: &'inv mut Option<TerminalPhysicalState<P, D, F, CB, EB>>,
    assembly: Option<&'inv mut super::super::super::PublicationAssemblyFields<O, P, D, CB, EB>>,
    common: &'inv mut Option<OperationInvocation<O, P, PP, U, D>>,
    contained: &'inv mut bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

type TerminalFieldsFromAssembly<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    DynamicTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>,
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
>;

type OptionalTerminalFields<'inv, O, P, PP, U, D, F, CB, EB> =
    Option<DynamicTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>>;

impl<'inv, O, P, PP, U, D, F, CB, EB> DynamicTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>
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
    pub(in crate::retained) fn from_operation(
        operation: RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>,
    ) -> Self {
        let RetainedOperationLoan {
            retained,
            terminal,
            assembly,
            common,
        } = operation.into_loan();
        let RetainedOperationFieldsLoan { baseline, metadata } = retained;
        let FallbackPublicationBaselineLoan::Dynamic(baseline) = baseline else {
            panic!("dynamic finalization requires its original dynamic baseline");
        };
        let TerminalPhysicalSlot::Dynamic(terminal) = terminal else {
            panic!("dynamic finalization requires its original dynamic terminal slot");
        };
        let InvocationLoan {
            common, contained, ..
        } = common;
        let InvocationSlot::Dynamic(common) = common else {
            panic!("dynamic finalization requires the original dynamic invocation slot");
        };
        Self {
            baseline,
            metadata,
            terminal,
            assembly: assembly.and_then(Option::as_mut),
            common,
            contained,
            not_send_or_sync: PhantomData,
        }
    }

    pub(in crate::retained) fn from_assembly(
        assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
    ) -> TerminalFieldsFromAssembly<'inv, O, P, PP, U, D, F, CB, EB> {
        let mut assembly = assembly.into_loan();
        if assembly.fields.fields.is_none()
            || assembly.operation.as_ref().is_none_or(|operation| {
                !matches!(&operation.common.common, InvocationSlot::Dynamic(_))
                    || operation.retained.baseline.is_finite()
                    || operation.terminal.is_finite()
            })
        {
            return Err(RetainedPublicationAssembly::from_loan(assembly));
        }
        let RetainedOperationLoan {
            retained,
            terminal,
            common,
            ..
        } = assembly
            .operation
            .take()
            .expect("the actual operation was checked before taking either field loan");
        let RetainedOperationFieldsLoan { baseline, metadata } = retained;
        let FallbackPublicationBaselineLoan::Dynamic(baseline) = baseline else {
            panic!("dynamic finalization requires its original dynamic baseline");
        };
        let TerminalPhysicalSlot::Dynamic(terminal) = terminal else {
            panic!("dynamic finalization requires its original dynamic terminal slot");
        };
        let InvocationLoan {
            common, contained, ..
        } = common;
        let InvocationSlot::Dynamic(common) = common else {
            unreachable!("the original backend was checked before ending either loan");
        };
        let fields = assembly.fields.take();
        Ok(Self {
            baseline,
            metadata,
            terminal,
            assembly: Some(fields),
            common,
            contained,
            not_send_or_sync: PhantomData,
        })
    }

    fn borrow_operation(&mut self) -> RetainedOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        RetainedOperationLoan {
            retained: RetainedOperationFieldsLoan {
                baseline: FallbackPublicationBaselineLoan::Dynamic(self.baseline),
                metadata: self.metadata,
            },
            terminal: TerminalPhysicalSlot::Dynamic(self.terminal),
            assembly: None,
            common: InvocationLoan {
                common: InvocationSlot::Dynamic(self.common),
                contained: self.contained,
                not_send_or_sync: PhantomData,
            },
        }
    }

    fn borrow_assembly(
        &mut self,
    ) -> RetainedPublicationAssemblyLoan<'_, O, P, PP, U, D, F, CB, EB> {
        let fields = self
            .assembly
            .as_deref_mut()
            .expect("assembly preparation retains the original fields");
        RetainedPublicationAssemblyLoan {
            fields: fields.into(),
            operation: Some(RetainedOperationLoan {
                retained: RetainedOperationFieldsLoan {
                    baseline: FallbackPublicationBaselineLoan::Dynamic(self.baseline),
                    metadata: self.metadata,
                },
                terminal: TerminalPhysicalSlot::Dynamic(self.terminal),
                assembly: None,
                common: InvocationLoan {
                    common: InvocationSlot::Dynamic(self.common),
                    contained: self.contained,
                    not_send_or_sync: PhantomData,
                },
            }),
        }
    }
}

impl<P> InvocationLifecycle<P> {
    fn try_seal_attached(
        &mut self,
        decision: AttachedTerminalDecision<P>,
        identity: crate::ledger::LedgerIdentity,
    ) -> Result<AttachedTerminalStamp, AttachedTerminalDecision<P>> {
        if !self.terminal_outstanding || self.terminal_seal.is_some() {
            return Err(decision);
        }
        self.terminal_seal = Some(InvocationTerminalSeal::AttachedReady(decision));
        Ok(AttachedTerminalStamp { identity })
    }

    pub(super) fn has_attached_stamp(
        &self,
        stamp: &AttachedTerminalStamp,
        identity: crate::ledger::LedgerIdentity,
    ) -> bool {
        self.terminal_outstanding
            && stamp.identity == identity
            && matches!(
                self.terminal_seal,
                Some(InvocationTerminalSeal::AttachedReady(_))
            )
    }

    pub(super) fn take_attached_decision(
        &mut self,
        stamp: AttachedTerminalStamp,
        identity: crate::ledger::LedgerIdentity,
    ) -> Option<AttachedTerminalDecision<P>> {
        if !self.has_attached_stamp(&stamp, identity) {
            return None;
        }
        match self.terminal_seal.take() {
            Some(InvocationTerminalSeal::AttachedReady(decision)) => Some(decision),
            _ => unreachable!("the original attached decision was just checked"),
        }
    }

    pub(super) fn attached_decision(&self) -> Option<&AttachedTerminalDecision<P>> {
        match self.terminal_seal.as_ref() {
            Some(InvocationTerminalSeal::AttachedReady(decision)) => Some(decision),
            _ => None,
        }
    }
}

impl<O, P, PP, U, D> TerminalInvocationLoan<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn try_seal_attached(
        &mut self,
        decision: AttachedTerminalDecision<P>,
    ) -> Result<AttachedTerminalStamp, AttachedTerminalDecision<P>> {
        let identity = self.common.as_read().identity();
        self.common
            .lifecycle_mut()
            .try_seal_attached(decision, identity)
    }
}

type MinimalTerminalResult<P, D, F, CB, EB> =
    Result<PreparedRetainedPublication<CB, EB, F, P, D>, FinalizationContainmentError>;

type DetachedRootResult<O, P, D, F, CB, EB> =
    Result<DynamicTerminalCapsule<O, P, D, F, CB, EB>, FinalizationContainmentError>;

type PublicationTerminalResult<O, P, D, F, CB, EB> = Result<
    Result<RetainedSuccessJointCommitFormation<O, P, D, F, CB, EB>, FinalizationContainmentError>,
    AttachedTerminalStamp,
>;

impl<'inv, O, P, PP, U, D, F, CB, EB> DynamicTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>
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
    fn install_terminal_preparation(
        &mut self,
        prepared: DeferredAttachedTerminal,
    ) -> AttachedTerminalStamp {
        let common = self
            .common
            .as_ref()
            .expect("prepared state stays in its actual root");
        assert!(common.lifecycle.has_attached_stamp(
            &prepared.stamp,
            common.coordinator.retained_ledger().as_read().identity(),
        ));
        let crate::retained::terminal_loan::TerminalEvidenceQualification::Dynamic(qualification) =
            prepared.qualification
        else {
            panic!("the dynamic root only installs its original dynamic evidence qualification");
        };
        let physical = self
            .terminal
            .as_mut()
            .expect("prepared physical owners stay in the root");
        assert!(physical.qualification.is_none() && physical.disposition.is_none());
        physical.qualification = Some(qualification);
        physical.disposition = Some(prepared.disposition);
        prepared.stamp
    }

    pub(in crate::retained) fn finish_minimal(
        mut self,
        cause: PreparationFailureCode,
    ) -> Result<PreparedRetainedPublication<CB, EB, F, P, D>, RetainedPublicationPrepareError> {
        let prepared = {
            self.borrow_operation()
                .prepare_minimal_fallback_deferred(cause)?
        };
        let stamp = self.install_terminal_preparation(prepared);
        self.try_finish_minimal_terminal(stamp)
            .unwrap_or_else(|_| unreachable_final_owner_state())
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)
    }

    pub(in crate::retained) fn finish_evidence_only(
        mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedRetainedEvidenceOnlyOutcome<F, P, D>, RetainedPublicationPrepareError> {
        let prepared = {
            self.borrow_operation()
                .prepare_evidence_only_deferred(final_progress)?
        };
        let stamp = self.install_terminal_preparation(prepared);
        self.try_finish_evidence_only_terminal(stamp)
            .unwrap_or_else(|_| unreachable_final_owner_state())
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)
    }

    pub(in crate::retained) fn finish_publication(
        mut self,
        request: RetainedTerminalRequest<CB, EB, PP, U>,
    ) -> Result<
        RetainedJointCommitFormationWithControl<O, P, D, F, CB, EB>,
        RetainedPublicationPrepareError,
    > {
        let frozen_success = matches!(&request, RetainedTerminalRequest::FrozenSuccess { .. });
        let (prepared, control_cause) = {
            let classification = self
                .borrow_assembly()
                .try_classify_terminal_preparation(request)?;
            // A frozen joint success cannot publish a late failure and reject it afterwards.
            let classification = if frozen_success {
                classification.require_success()?
            } else {
                classification
            };
            classification.finish_deferred()?
        };
        let stamp = self.install_terminal_preparation(prepared);
        let formation = self
            .try_finish_publication_terminal(stamp)
            .unwrap_or_else(|_| unreachable_final_owner_state())
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)?;
        Ok((formation, control_cause))
    }

    fn contain_extraction_fault(&mut self) -> Option<FinalizationContainmentError> {
        *self.contained = true;
        let reason = AccountPoisonReason::OwnershipGraphUnverified;
        let cause = self
            .terminal
            .take()
            .map(|physical| physical.contain_new_fault(reason));
        // All physical owners are protected before any surviving accounting slots are retained.
        if let Some(assembly) = self.assembly.as_deref_mut() {
            let mut physical = TerminalPhysicalSlots::<P, D, F, CB, EB> {
                fallback: None,
                evidence: None,
                canonical_table: Some(&mut assembly.canonical_table),
                evidence_table: Some(&mut assembly.evidence_table),
                account: Some(&mut assembly.account),
            };
            physical.quarantine_unresolved();
        }
        if let Some(account) = self.baseline.first_account() {
            let _ = account.record_poison(reason);
        }
        self.baseline.record_forgotten_physical_edges();
        self.baseline.fail_closed_after_recorded_poison();
        if let Some(assembly) = self.assembly.as_deref_mut() {
            assembly.retain_batches_fail_closed();
        }
        self.metadata.deferred_stage = None;
        std::mem::forget(self.common.take());
        cause
    }

    /// Rejects another root's stamp before taking any actual owner or replaying preparation.
    fn try_finish_minimal_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> Result<MinimalTerminalResult<P, D, F, CB, EB>, AttachedTerminalStamp> {
        if !self.terminal.as_ref().is_some_and(|physical| {
            matches!(
                physical.disposition,
                Some(TerminalDisposition::MinimalFallback { .. }),
            )
        }) {
            return Err(stamp);
        }
        self.try_detach_terminal(stamp)
            .map(|result| result.and_then(DynamicTerminalCapsule::finish_minimal))
    }

    fn try_finish_evidence_only_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> Result<
        Result<PreparedRetainedEvidenceOnlyOutcome<F, P, D>, FinalizationContainmentError>,
        AttachedTerminalStamp,
    > {
        if !self.terminal.as_ref().is_some_and(|physical| {
            matches!(
                physical.disposition,
                Some(
                    TerminalDisposition::EvidenceOnlySuccess
                        | TerminalDisposition::MinimalFallback { .. }
                ),
            )
        }) {
            return Err(stamp);
        }
        self.try_detach_terminal(stamp).map(|result| {
            result.and_then(|capsule| match capsule.disposition() {
                TerminalDisposition::EvidenceOnlySuccess => capsule
                    .finish_evidence_only()
                    .map(PreparedRetainedEvidenceOnlyOutcome::Success),
                TerminalDisposition::MinimalFallback { .. } => {
                    capsule
                        .finish_minimal()
                        .map(|prepared| match prepared.into_outcome() {
                            PreparedRetainedPublicationOutcome::MinimalFallback(fallback) => {
                                PreparedRetainedEvidenceOnlyOutcome::MinimalFallback(fallback)
                            }
                            _ => unreachable_final_owner_state(),
                        })
                }
                _ => unreachable_final_owner_state(),
            })
        })
    }

    fn try_finish_publication_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> PublicationTerminalResult<O, P, D, F, CB, EB> {
        if !self.terminal.as_ref().is_some_and(|physical| {
            matches!(
                physical.disposition,
                Some(
                    TerminalDisposition::Success { .. }
                        | TerminalDisposition::RichFailure { .. }
                        | TerminalDisposition::MinimalFallback { .. }
                ),
            )
        }) {
            return Err(stamp);
        }
        self.try_detach_terminal(stamp).map(|result| {
            result.and_then(|capsule| match capsule.disposition() {
                TerminalDisposition::Success { .. } => capsule
                    .finish_success()
                    .map(RetainedSuccessJointCommitFormation::Intent),
                TerminalDisposition::RichFailure { .. } => capsule
                    .finish_rich()
                    .map(RetainedSuccessJointCommitFormation::PreparedFailure),
                TerminalDisposition::MinimalFallback { .. } => capsule
                    .finish_minimal()
                    .map(RetainedSuccessJointCommitFormation::PreparedFailure),
                _ => unreachable_final_owner_state(),
            })
        })
    }

    fn try_detach_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> Result<DetachedRootResult<O, P, D, F, CB, EB>, AttachedTerminalStamp> {
        let Some(common) = self.common.as_ref() else {
            return Err(stamp);
        };
        let identity = common.coordinator.retained_ledger().as_read().identity();
        if !common.lifecycle.has_attached_stamp(&stamp, identity)
            || !self.terminal.as_ref().is_some_and(|physical| {
                physical.qualification.is_some() && physical.disposition.is_some()
            })
        {
            return Err(stamp);
        }
        let decision = common
            .lifecycle
            .attached_decision()
            .expect("the ready decision was checked");
        if decision.layout == TypedTerminalLayout::Fallback && decision.roles.deferred_canonical {
            return Err(stamp);
        }

        // The actual root keeps physical cleanup responsibility throughout partial numerical
        // extraction, including the interval before the owned capsule is installed.
        let mut root = scopeguard::guard_on_unwind(self, |root| {
            let _ = root.contain_extraction_fault();
        });
        *root.contained = true;
        let common = root.common.as_mut().expect("the actual state is installed");
        let decision = common
            .lifecycle
            .take_attached_decision(stamp, identity)
            .expect("the original stamp was checked without an intervening callback");
        let common = scopeguard::guard(
            root.common
                .take()
                .expect("concrete extraction takes the original owner once"),
            std::mem::forget,
        );
        let original: &mut Self = &mut root;
        let baseline = original
            .baseline
            .terminal_batches()
            .expect("qualification keeps the original batch owner");
        let batches =
            select_terminal_batches(&decision, baseline, original.assembly.as_deref_mut());
        let proof = match scopeguard::ScopeGuard::into_inner(common)
            .try_detach_typed(decision.last_completed_phase, batches)
        {
            Ok(proof) => proof,
            Err(error) => {
                let cause = root
                    .contain_extraction_fault()
                    .expect("the original physical graph is installed until detach succeeds");
                error.retain_fail_closed();
                return Ok(Err(cause));
            }
        };
        let physical = root
            .terminal
            .take()
            .expect("only successful detach transfers physical owners");
        root.metadata.deferred_stage = None;
        Ok(Ok(DynamicTerminalCapsule::new(physical, proof)))
    }
}

impl<O, P, PP, U, D, F, CB, EB> InvocationRoot<O, P, PP, U, D, F, CB, EB>
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
    fn terminal_fields(&mut self) -> OptionalTerminalFields<'_, O, P, PP, U, D, F, CB, EB> {
        let retained = self.retained.as_mut()?;
        Some(DynamicTerminalRootFields {
            baseline: &mut retained.baseline,
            metadata: &mut retained.metadata,
            terminal: &mut self.terminal,
            assembly: self.assembly.as_mut(),
            common: &mut self.common,
            contained: &mut self.contained,
            not_send_or_sync: PhantomData,
        })
    }

    pub(crate) fn install_terminal_preparation(
        &mut self,
        prepared: DeferredAttachedTerminal,
    ) -> AttachedTerminalStamp {
        self.terminal_fields()
            .expect("the prepared root retains its original fields")
            .install_terminal_preparation(prepared)
    }

    /// Completes borrowed fallback preparation before lending any concrete extraction.
    pub(crate) fn prepare_minimal_terminal(
        &mut self,
        cause: PreparationFailureCode,
    ) -> Result<Option<AttachedTerminalStamp>, RetainedPublicationPrepareError> {
        let Some(operation) = self.resume_deferred_operation() else {
            return Ok(None);
        };
        let prepared = operation
            .into_loan()
            .prepare_minimal_fallback_deferred(cause)?;
        Ok(Some(self.install_terminal_preparation(prepared)))
    }

    pub(crate) fn prepare_evidence_only_terminal(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<Option<AttachedTerminalStamp>, RetainedPublicationPrepareError> {
        let Some(operation) = self.resume_deferred_operation() else {
            return Ok(None);
        };
        let prepared = operation
            .into_loan()
            .prepare_evidence_only_deferred(final_progress)?;
        Ok(Some(self.install_terminal_preparation(prepared)))
    }

    pub(crate) fn try_finish_minimal_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> Result<MinimalTerminalResult<P, D, F, CB, EB>, AttachedTerminalStamp> {
        let Some(mut fields) = self.terminal_fields() else {
            return Err(stamp);
        };
        fields.try_finish_minimal_terminal(stamp)
    }

    pub(crate) fn try_finish_evidence_only_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> Result<
        Result<PreparedRetainedEvidenceOnlyOutcome<F, P, D>, FinalizationContainmentError>,
        AttachedTerminalStamp,
    > {
        let Some(mut fields) = self.terminal_fields() else {
            return Err(stamp);
        };
        fields.try_finish_evidence_only_terminal(stamp)
    }

    pub(crate) fn try_finish_publication_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> PublicationTerminalResult<O, P, D, F, CB, EB> {
        let Some(mut fields) = self.terminal_fields() else {
            return Err(stamp);
        };
        fields.try_finish_publication_terminal(stamp)
    }
}
