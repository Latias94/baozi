//! Terminal publication ownership, attribution, and freeze protocol.

use super::*;

#[cfg(test)]
thread_local! {
    static PANIC_NEXT_EVIDENCE_ONLY_PRIMARY_BIND: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
pub(crate) fn panic_next_evidence_only_primary_bind() {
    PANIC_NEXT_EVIDENCE_ONLY_PRIMARY_BIND.with(|panic| panic.set(true));
}

#[cfg(test)]
fn maybe_panic_evidence_only_primary_bind(publish_canonical: bool) {
    if !publish_canonical
        && PANIC_NEXT_EVIDENCE_ONLY_PRIMARY_BIND.with(|panic| panic.replace(false))
    {
        panic!("injected evidence-only primary bind panic");
    }
}

pub(super) fn cause_after_primary_ledger_rejection(
    cause: PreparationFailureCode,
) -> PreparationFailureCode {
    match cause.class() {
        PreparationFailureClass::Requested => PreparationFailureCode::ledger(),
        PreparationFailureClass::Control
        | PreparationFailureClass::ProductGraph
        | PreparationFailureClass::Ledger => cause,
    }
}

pub(super) struct TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: Option<DetachedRetainedPart<F>>,
    evidence: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    account: Option<PhysicalStorageAccount>,
}

struct ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: DetachedRetainedPart<F>,
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
}

type TerminalPhysicalParts<P, D, F, CanonicalBlock, EvidenceBlock> = (
    DetachedRetainedPart<F>,
    DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    Option<RetainedTableAllocation<CanonicalBlock>>,
    Option<RetainedTableAllocation<EvidenceBlock>>,
    Option<PhysicalStorageAccount>,
);

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn new(
        fallback: DetachedRetainedPart<F>,
        evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
        evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
        account: Option<PhysicalStorageAccount>,
    ) -> Self {
        Self {
            fallback: Some(fallback),
            evidence: Some(evidence),
            canonical_table,
            evidence_table,
            account,
        }
    }

    fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    fn evidence(&self) -> &DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        match self.evidence.as_ref() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        }
    }

    fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .as_ref()
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::account_anchor)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(DetachedRetainedPart::account_anchor)
            })
            .or_else(|| self.canonical_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.canonical_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| self.evidence_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.evidence_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
    }

    fn has_exact_fallback_shape(&self) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self
                .account
                .as_ref()
                .is_none_or(|account| account.is_same_account(evidence.account()))
    }

    fn record_forgotten_physical_edges(
        &self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let account = match self.first_account() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        if let Some(canonical) = self.canonical_table.as_ref() {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.evidence_table.as_ref() {
            record_forgotten_table_edges(evidence);
        }
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edge();
        }
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }

    fn record_new_fault(&self, reason: AccountPoisonReason) -> AccountPoisonEventWitness {
        let account = match self.account.as_ref().or_else(|| self.first_account()) {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        record_account_poison_event(account, reason)
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(canonical) = self.canonical_table.take() {
            std::mem::forget(canonical);
        }
        if let Some(evidence) = self.evidence_table.take() {
            std::mem::forget(evidence);
        }
        if let Some(account) = self.account.take() {
            drop(account);
        }
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
            drop(fallback);
        }
        if let Some(mut evidence) = self.evidence.take() {
            evidence.fail_closed_after_recorded_poison();
            drop(evidence);
        }
    }

    pub(super) fn contain_new_fault(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.record_new_fault(reason);
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    pub(super) fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    pub(super) fn contain_after_quarantine(
        self,
        cause: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let reason = cause.snapshot().first_poison_evidence().map_or(
            AccountPoisonReason::OwnershipGraphUnverified,
            AccountPoisonEvidence::reason,
        );
        self.contain_after_recorded(reason)
    }

    pub(super) fn into_parts(
        mut self,
    ) -> TerminalPhysicalParts<P, D, F, CanonicalBlock, EvidenceBlock> {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        (
            fallback,
            evidence,
            self.canonical_table.take(),
            self.evidence_table.take(),
            self.account.take(),
        )
    }

    fn qualifies_pre_discard_success(&self) -> bool {
        let (
            Some(fallback),
            Some(evidence),
            Some(canonical),
            Some(outcome_evidence),
            Some(account),
        ) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.canonical_table.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        )
        else {
            return false;
        };
        fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && canonical.belongs_to(account)
            && outcome_evidence.belongs_to(account)
    }

    pub(super) fn qualifies_pre_cleanup_rich(&self, expect_canonical: bool) -> bool {
        let (Some(fallback), Some(evidence), Some(evidence_table), Some(account)) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        self.canonical_table.is_some() == expect_canonical
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && evidence_table.belongs_to(account)
            && self
                .canonical_table
                .as_ref()
                .is_none_or(|table| table.belongs_to(account))
    }

    pub(super) fn qualifies_pre_cleanup_fallback(
        &self,
        expect_canonical: bool,
        expect_evidence: bool,
    ) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        let table_expected = expect_canonical || expect_evidence;
        self.canonical_table.is_some() == expect_canonical
            && self.evidence_table.is_some() == expect_evidence
            && self.account.is_some() == table_expected
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self.account.as_ref().is_none_or(|account| {
                fallback.account().is_same_account(account)
                    && self
                        .canonical_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
                    && self
                        .evidence_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
            })
    }

    fn finish_pre_discard_success(
        mut self,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    ) -> ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock> {
        ExactPreDiscardSuccess {
            fallback: self
                .fallback
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence: self
                .evidence
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_table: self
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence_table: self
                .evidence_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            account: self
                .account
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_root,
            evidence_root,
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        if self.first_account().is_none() {
            return;
        }
        self.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        let _ = self.record_forgotten_physical_edges(AccountPoisonReason::OwnershipGraphUnverified);
        self.fail_closed_after_recorded_poison();
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }

    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalOwnershipManifest::new(
            self.fallback,
            self.evidence,
            Some(self.canonical_table),
            Some(self.evidence_table),
            Some(self.account),
        )
    }
}

pub(super) struct TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

pub(super) enum FrozenSuccessPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Success(TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
}

pub(super) enum EvidenceOnlySuccessPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Success(TerminalEvidenceOnlySuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Fallback(
        TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        PreparationFailureCode,
    ),
}

struct QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
}

struct QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
}

struct PublicationReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<PublicationReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct PublicationReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: FreezeReadyTypedProof<O, P, D>,
    set_id: RetainedSetId,
}

struct DetachedReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: crate::ledger::DetachedTypedFreezeProof<O, P, D>,
    set_id: RetainedSetId,
}

struct VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: VerifiedSuccessPhysicalDiscardState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the pending variant is the single linear owner; boxing would add allocation and split the ownership proof"
)]
enum VerifiedSuccessPhysicalDiscardState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Pending {
        acknowledgement: SuccessDiscardAcknowledgement,
        physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
        set_id: RetainedSetId,
        discarded_account: PhysicalStorageAccount,
    },
    Acknowledged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SuccessDiscardAcknowledgement {
    NotStarted,
    InProgress,
}

pub(super) struct FreezeReadyDetachedTerminalEvidence<P, D> {
    owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    qualification: TerminalEvidenceFreezeQualification,
}

type EligibilityReadySuccessJointCommit<O, P, D, F, CanonicalBlock, EvidenceBlock> =
    AccountBoundJointCommit<
        EligibilityReadySuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
        O,
        P,
        D,
    >;

/// Linear success owner after every fallible step except final account eligibility.
#[must_use = "the success intent must be jointly committed or contained by dropping it"]
pub(crate) struct RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>,
}

struct EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibilityReadySuccessJointCommit<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
    #[cfg(test)]
    observation: SuccessJointCommitTestObservation<D>,
}

#[cfg(test)]
struct SuccessJointCommitTestObservation<D> {
    current_live_bytes: u64,
    active_reservations: u64,
    cover_matches_state: bool,
    first_dimension: Option<(D, DimensionSnapshot)>,
}

struct EligibilityReadySuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock> {
    canonical_table: RetainedTableAllocation<CanonicalBlock>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
    set_id: RetainedSetId,
    fallback_type: PhantomData<fn() -> F>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: ExactPreDiscardSuccess<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
        }
    }

    fn contain_unqualified(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let cause = physical.into_terminal().contain_new_fault(reason);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
    }

    pub(super) fn try_form_joint_commit_intent(
        mut self,
        set_id: RetainedSetId,
    ) -> Result<
        RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_evidence_discard() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        let ExactPreDiscardSuccess {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root,
            evidence_root,
        } = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());

        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_success_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => evidence,
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = ExactPreDiscardSuccess {
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                        canonical_root,
                        evidence_root,
                    };
                    let cause = physical
                        .into_terminal()
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };

        let qualified = QualifiedSuccessGraph {
            state: Some(QualifiedSuccessState {
                physical: QualifiedSuccessPhysical {
                    fallback,
                    evidence: qualified_evidence.into_freeze_ready(),
                    canonical_table,
                    evidence_table,
                    account,
                    canonical_root,
                    evidence_root,
                },
                proof,
            }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_success_guard(SuccessGuardDropStage::Qualified, qualified);
        qualified.try_form_joint_commit_intent(set_id)
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(&mut self) -> QualifiedSuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let QualifiedSuccessState { physical, proof } = self.take_state();
        let cause = physical.into_terminal().contain_new_fault(reason);
        proof.retain_fail_closed();
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let QualifiedSuccessState { physical, proof } = self.take_state();
        let cause = physical.into_terminal().contain_after_recorded(reason);
        proof.retain_fail_closed();
        cause
    }

    fn contain_cleanup(self, cleanup: RetainedPartCleanupError) -> FinalizationContainmentError {
        match cleanup {
            RetainedPartCleanupError::MissingFaultAnchor
            | RetainedPartCleanupError::DeferredOwnershipFault => {
                self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::NewFaultAlreadyRecorded(_)
            | RetainedPartCleanupError::StillSharedOwnershipFault(_) => {
                self.contain_after_recorded(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::StorageReleaseUnverified(_) => {
                self.contain_after_recorded(AccountPoisonReason::RetainedStorageReleaseUnverified)
            }
            RetainedPartCleanupError::ChargeReleaseUnverified(witness) => {
                self.contain_after_recorded(witness.reason)
            }
        }
    }

    fn try_form_joint_commit_intent(
        mut self,
        set_id: RetainedSetId,
    ) -> Result<
        RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let discarded_account = match self
            .state
            .as_mut()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .physical
            .fallback
            .try_discard_allocation()
        {
            Ok(account) => account,
            Err(cleanup) => return Err(self.contain_cleanup(cleanup)),
        };
        let QualifiedSuccessState { physical, proof } = self.take_state();
        let verified =
            VerifiedSuccessPhysicalDiscard::new(physical, proof, set_id, discarded_account);
        #[cfg(test)]
        let verified = maybe_drop_success_guard(SuccessGuardDropStage::Verified, verified);
        let publication_ready = verified.acknowledge();
        #[cfg(test)]
        let publication_ready =
            maybe_drop_success_guard(SuccessGuardDropStage::PublicationReady, publication_ready);
        let eligibility_ready = publication_ready.try_detach_and_freeze()?;
        #[cfg(test)]
        let eligibility_ready =
            maybe_drop_success_guard(SuccessGuardDropStage::EligibilityReady, eligibility_ready);
        Ok(RetainedSuccessJointCommitIntent {
            state: eligibility_ready,
        })
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedSuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(QualifiedSuccessState { physical, proof }) = self.state.take() {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        let Self {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root: _,
            evidence_root: _,
        } = self;
        TerminalPhysicalOwnershipManifest::new(
            fallback,
            evidence.into_unfrozen_owner(),
            Some(canonical_table),
            Some(evidence_table),
            Some(account),
        )
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    PublicationReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(
        &mut self,
    ) -> PublicationReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn try_detach_and_freeze(
        mut self,
    ) -> Result<
        EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let PublicationReadySuccessState {
            physical,
            proof,
            set_id,
        } = self.take_state();
        let proof = match proof.try_into_detached() {
            Ok(proof) => proof,
            Err(error) => {
                let cause = physical
                    .into_terminal()
                    .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                error.retain_fail_closed();
                return Err(cause);
            }
        };
        let state = DetachedReadySuccessState {
            physical,
            proof,
            set_id,
        };
        let mut state = scopeguard::guard(
            state,
            contain_detached_ready_success_state::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        #[cfg(test)]
        maybe_panic_success_guard_stage(SuccessGuardDropStage::LedgerDetached);
        let DetachedReadySuccessState {
            physical,
            proof,
            set_id: _,
        } = &mut *state;
        physical
            .evidence
            .freeze_in_place(TerminalEvidenceCause::Success, proof.terminal_projection());
        drop(std::mem::replace(
            &mut state.physical.fallback,
            DetachedRetainedPart::empty(),
        ));
        let state = scopeguard::ScopeGuard::into_inner(state);
        let DetachedReadySuccessState {
            physical,
            proof,
            set_id,
        } = state;
        let QualifiedSuccessPhysical {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root,
            evidence_root,
        } = physical;
        debug_assert!(fallback.is_empty());
        #[cfg(test)]
        let observation = SuccessJointCommitTestObservation {
            current_live_bytes: proof.test_current_live_bytes(),
            active_reservations: proof.test_active_reservations(),
            cover_matches_state: proof.test_cover_matches_state(),
            first_dimension: proof.terminal_projection().dimensions().next().and_then(
                |(dimension, _)| {
                    proof
                        .test_dimension_evidence(dimension)
                        .map(|snapshot| (dimension, snapshot))
                },
            ),
        };
        Ok(EligibilityReadySuccessGraph {
            state: Some(AccountBoundJointCommit::new(
                EligibilityReadySuccessPhysical {
                    canonical_table,
                    evidence_table,
                    account,
                    evidence: evidence.into_frozen_owner(),
                    canonical_root,
                    evidence_root,
                    set_id,
                    fallback_type: PhantomData,
                },
                proof,
            )),
            #[cfg(test)]
            observation,
        })
    }
}

fn contain_detached_ready_success_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: DetachedReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let DetachedReadySuccessState {
        physical,
        proof,
        set_id: _,
    } = state;
    let _ = physical
        .into_terminal()
        .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
}

fn contain_publication_ready_success_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: PublicationReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let PublicationReadySuccessState {
        physical,
        proof,
        set_id: _,
    } = state;
    let _ = physical
        .into_terminal()
        .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for PublicationReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_publication_ready_success_state(state);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn new(
        physical: QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
        set_id: RetainedSetId,
        discarded_account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            state: VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement: SuccessDiscardAcknowledgement::NotStarted,
                physical,
                proof,
                set_id,
                discarded_account,
            },
        }
    }

    fn acknowledge(
        mut self,
    ) -> PublicationReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let freeze_ready = match &mut self.state {
            VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement,
                proof,
                ..
            } => {
                *acknowledgement = SuccessDiscardAcknowledgement::InProgress;
                #[cfg(test)]
                maybe_panic_success_guard_stage(SuccessGuardDropStage::VerifiedInProgress);
                proof.release_discard_after_physical_cleanup_in_place()
            }
            VerifiedSuccessPhysicalDiscardState::Acknowledged => unreachable_final_owner_state(),
        };
        let state = std::mem::replace(
            &mut self.state,
            VerifiedSuccessPhysicalDiscardState::Acknowledged,
        );
        match state {
            VerifiedSuccessPhysicalDiscardState::Pending {
                acknowledgement: _,
                physical,
                proof: _,
                set_id,
                discarded_account,
            } => {
                drop(discarded_account);
                PublicationReadySuccessGraph {
                    state: Some(PublicationReadySuccessState {
                        physical,
                        proof: freeze_ready,
                        set_id,
                    }),
                }
            }
            VerifiedSuccessPhysicalDiscardState::Acknowledged => unreachable_final_owner_state(),
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for VerifiedSuccessPhysicalDiscard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        let state = std::mem::replace(
            &mut self.state,
            VerifiedSuccessPhysicalDiscardState::Acknowledged,
        );
        if let VerifiedSuccessPhysicalDiscardState::Pending {
            acknowledgement: _,
            physical,
            proof,
            set_id: _,
            discarded_account,
        } = state
        {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            drop(discarded_account);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(crate) fn qualify_and_commit(
        self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        self.state.try_into_prepared()
    }

    pub(crate) fn qualify_and_commit_frozen_exact(
        self,
    ) -> Result<
        PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>,
        FinalizationContainmentError,
    > {
        self.state.try_into_prepared_success()
    }

    #[cfg(test)]
    pub(super) fn test_current_live_bytes(&self) -> u64 {
        self.state.observation.current_live_bytes
    }

    #[cfg(test)]
    pub(super) fn test_active_reservations(&self) -> u64 {
        self.state.observation.active_reservations
    }

    #[cfg(test)]
    pub(super) fn test_cover_matches_state(&self) -> bool {
        self.state.observation.cover_matches_state
    }

    #[cfg(test)]
    pub(super) fn test_dimension_evidence(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.state
            .observation
            .first_dimension
            .filter(|(key, _)| *key == dimension)
            .map(|(_, snapshot)| snapshot)
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let Some(state) = self.state.take() else {
            unreachable_final_owner_state()
        };
        Ok(state.try_commit()?.into_prepared())
    }

    fn try_into_prepared_success(
        mut self,
    ) -> Result<
        PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>,
        FinalizationContainmentError,
    > {
        let Some(state) = self.state.take() else {
            unreachable_final_owner_state()
        };
        Ok(state.try_commit()?.into_prepared_success())
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    EligibilityReadySuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn into_terminal(
        self,
    ) -> TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        let Self {
            canonical_table,
            evidence_table,
            account,
            evidence,
            canonical_root: _,
            evidence_root: _,
            set_id: _,
            fallback_type: _,
        } = self;
        TerminalPhysicalOwnershipManifest::new(
            DetachedRetainedPart::<F>::empty(),
            evidence,
            Some(canonical_table),
            Some(evidence_table),
            Some(account),
        )
    }

    fn into_prepared(self) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let Self {
            canonical_table,
            evidence_table,
            account,
            evidence,
            canonical_root,
            evidence_root,
            set_id,
            fallback_type: _,
        } = self;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::Success {
                canonical: PublishedCanonicalAggregate::from_table(canonical_table, set_id),
                outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(
                    evidence_table,
                    set_id,
                ),
                canonical_root,
                outcome_evidence_root: evidence_root,
                account,
            },
            evidence: PublishedPart::from_exact_owner(evidence),
        }
    }

    fn into_prepared_success(self) -> PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D> {
        let Self {
            canonical_table,
            evidence_table,
            account,
            evidence,
            canonical_root,
            evidence_root,
            set_id,
            fallback_type: _,
        } = self;
        PreparedRetainedSuccess {
            canonical: PublishedCanonicalAggregate::from_table(canonical_table, set_id),
            outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(evidence_table, set_id),
            canonical_root,
            outcome_evidence_root: evidence_root,
            account,
            evidence: PublishedPart::from_exact_owner(evidence),
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock> AccountBoundPublicationManifest
    for EligibilityReadySuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn publication_account(&self) -> &PhysicalStorageAccount {
        &self.account
    }

    fn contain_uncommitted(self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        self.into_terminal().contain_new_fault(reason)
    }

    fn contain_after_quarantine(self, quarantine: AccountPoisoned) -> FinalizationContainmentError {
        self.into_terminal().contain_after_quarantine(quarantine)
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            drop(state);
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}

pub(super) struct TerminalEvidenceOnlySuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct QualifiedEvidenceOnlySuccessPhysical<P, D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
}

struct QualifiedEvidenceOnlySuccessState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedEvidenceOnlySuccessPhysical<P, D, F>,
    proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
}

struct QualifiedEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedEvidenceOnlySuccessState<O, P, D, F>>,
}

enum VerifiedEvidenceOnlySuccessDiscardState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Pending {
        acknowledgement: SuccessDiscardAcknowledgement,
        physical: QualifiedEvidenceOnlySuccessPhysical<P, D, F>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
        discarded_account: PhysicalStorageAccount,
    },
    Acknowledged,
}

struct VerifiedEvidenceOnlySuccessDiscard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: VerifiedEvidenceOnlySuccessDiscardState<O, P, D, F>,
}

struct FreezeReadyEvidenceOnlySuccessState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedEvidenceOnlySuccessPhysical<P, D, F>,
    proof: FreezeReadyTypedProof<O, P, D>,
}

struct FreezeReadyEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyEvidenceOnlySuccessState<O, P, D, F>>,
}

struct DetachedReadyEvidenceOnlySuccessState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    physical: QualifiedEvidenceOnlySuccessPhysical<P, D, F>,
    proof: crate::ledger::DetachedTypedFreezeProof<O, P, D>,
}

struct EligibilityReadyEvidenceOnlySuccessPhysical<P, D> {
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
}

type EligibilityReadyEvidenceOnlySuccessJointCommit<O, P, D> =
    AccountBoundJointCommit<EligibilityReadyEvidenceOnlySuccessPhysical<P, D>, O, P, D>;

struct EligibilityReadyEvidenceOnlySuccessGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibilityReadyEvidenceOnlySuccessJointCommit<O, P, D>>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalEvidenceOnlySuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
        }
    }

    fn contain_unqualified(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let cause = physical.contain_new_fault(reason);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
    }

    pub(super) fn into_prepared(
        mut self,
    ) -> Result<PreparedRetainedEvidenceOnlySuccess<P, D>, FinalizationContainmentError> {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_evidence_discard() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .has_exact_fallback_shape()
        {
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let (fallback, evidence, canonical_table, evidence_table, account) = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .into_parts();
        if canonical_table.is_some() || evidence_table.is_some() || account.is_some() {
            let physical = TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence,
                canonical_table,
                evidence_table,
                account,
            );
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let evidence =
            match QualifiedDetachedTerminalEvidence::try_from_success_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => {
                    evidence.into_freeze_ready()
                }
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), ()>::new(
                        fallback, evidence, None, None, None,
                    );
                    let cause = physical
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };
        QualifiedEvidenceOnlySuccessGraph {
            state: Some(QualifiedEvidenceOnlySuccessState {
                physical: QualifiedEvidenceOnlySuccessPhysical { fallback, evidence },
                proof,
            }),
        }
        .into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalEvidenceOnlySuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
    }
}

impl<P, D, F> QualifiedEvidenceOnlySuccessPhysical<P, D, F> {
    fn into_terminal(self) -> TerminalPhysicalOwnershipManifest<P, D, F, (), ()> {
        TerminalPhysicalOwnershipManifest::new(
            self.fallback,
            self.evidence.into_unfrozen_owner(),
            None,
            None,
            None,
        )
    }
}

impl<O, P, D, F> QualifiedEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(&mut self) -> QualifiedEvidenceOnlySuccessState<O, P, D, F> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let state = self.take_state();
        let cause = state.physical.into_terminal().contain_new_fault(reason);
        state.proof.retain_fail_closed();
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let state = self.take_state();
        let cause = state
            .physical
            .into_terminal()
            .contain_after_recorded(reason);
        state.proof.retain_fail_closed();
        cause
    }

    fn contain_cleanup(self, cleanup: RetainedPartCleanupError) -> FinalizationContainmentError {
        match cleanup {
            RetainedPartCleanupError::MissingFaultAnchor
            | RetainedPartCleanupError::DeferredOwnershipFault => {
                self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::NewFaultAlreadyRecorded(_)
            | RetainedPartCleanupError::StillSharedOwnershipFault(_) => {
                self.contain_after_recorded(AccountPoisonReason::OwnershipGraphUnverified)
            }
            RetainedPartCleanupError::StorageReleaseUnverified(_) => {
                self.contain_after_recorded(AccountPoisonReason::RetainedStorageReleaseUnverified)
            }
            RetainedPartCleanupError::ChargeReleaseUnverified(witness) => {
                self.contain_after_recorded(witness.reason)
            }
        }
    }

    fn into_prepared(
        mut self,
    ) -> Result<PreparedRetainedEvidenceOnlySuccess<P, D>, FinalizationContainmentError> {
        let discarded_account = match self
            .state
            .as_mut()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .physical
            .fallback
            .try_discard_allocation()
        {
            Ok(account) => account,
            Err(cleanup) => return Err(self.contain_cleanup(cleanup)),
        };
        let state = self.take_state();
        VerifiedEvidenceOnlySuccessDiscard {
            state: VerifiedEvidenceOnlySuccessDiscardState::Pending {
                acknowledgement: SuccessDiscardAcknowledgement::NotStarted,
                physical: state.physical,
                proof: state.proof,
                discarded_account,
            },
        }
        .acknowledge()
        .try_detach_and_freeze()
    }
}

impl<O, P, D, F> Drop for QualifiedEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = state
                .physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            state.proof.retain_fail_closed();
        }
    }
}

impl<O, P, D, F> VerifiedEvidenceOnlySuccessDiscard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn acknowledge(mut self) -> FreezeReadyEvidenceOnlySuccessGraph<O, P, D, F> {
        let proof = match &mut self.state {
            VerifiedEvidenceOnlySuccessDiscardState::Pending {
                acknowledgement,
                proof,
                ..
            } => {
                *acknowledgement = SuccessDiscardAcknowledgement::InProgress;
                proof.release_discard_after_physical_cleanup_in_place()
            }
            VerifiedEvidenceOnlySuccessDiscardState::Acknowledged => {
                unreachable_final_owner_state()
            }
        };
        let state = std::mem::replace(
            &mut self.state,
            VerifiedEvidenceOnlySuccessDiscardState::Acknowledged,
        );
        match state {
            VerifiedEvidenceOnlySuccessDiscardState::Pending {
                acknowledgement: _,
                physical,
                proof: _,
                discarded_account,
            } => {
                drop(discarded_account);
                FreezeReadyEvidenceOnlySuccessGraph {
                    state: Some(FreezeReadyEvidenceOnlySuccessState { physical, proof }),
                }
            }
            VerifiedEvidenceOnlySuccessDiscardState::Acknowledged => {
                unreachable_final_owner_state()
            }
        }
    }
}

impl<O, P, D, F> Drop for VerifiedEvidenceOnlySuccessDiscard<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        let state = std::mem::replace(
            &mut self.state,
            VerifiedEvidenceOnlySuccessDiscardState::Acknowledged,
        );
        if let VerifiedEvidenceOnlySuccessDiscardState::Pending {
            acknowledgement: _,
            physical,
            proof,
            discarded_account,
        } = state
        {
            let _ = physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            drop(discarded_account);
        }
    }
}

impl<O, P, D, F> FreezeReadyEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_detach_and_freeze(
        mut self,
    ) -> Result<PreparedRetainedEvidenceOnlySuccess<P, D>, FinalizationContainmentError> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match state.proof.try_into_detached() {
            Ok(proof) => proof,
            Err(error) => {
                let cause = state
                    .physical
                    .into_terminal()
                    .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                error.retain_fail_closed();
                return Err(cause);
            }
        };
        let state = DetachedReadyEvidenceOnlySuccessState {
            physical: state.physical,
            proof,
        };
        let mut state =
            scopeguard::guard(state, contain_detached_ready_evidence_only_success_state);
        let DetachedReadyEvidenceOnlySuccessState { physical, proof } = &mut *state;
        physical
            .evidence
            .freeze_in_place(TerminalEvidenceCause::Success, proof.terminal_projection());
        drop(std::mem::replace(
            &mut state.physical.fallback,
            DetachedRetainedPart::empty(),
        ));
        let state = scopeguard::ScopeGuard::into_inner(state);
        let DetachedReadyEvidenceOnlySuccessState { physical, proof } = state;
        let QualifiedEvidenceOnlySuccessPhysical { fallback, evidence } = physical;
        debug_assert!(fallback.is_empty());
        EligibilityReadyEvidenceOnlySuccessGraph {
            state: Some(AccountBoundJointCommit::new(
                EligibilityReadyEvidenceOnlySuccessPhysical {
                    evidence: evidence.into_frozen_owner(),
                },
                proof,
            )),
        }
        .try_into_prepared()
    }
}

impl<O, P, D, F> Drop for FreezeReadyEvidenceOnlySuccessGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = state
                .physical
                .into_terminal()
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            state.proof.retain_fail_closed();
        }
    }
}

fn contain_detached_ready_evidence_only_success_state<O, P, D, F>(
    state: DetachedReadyEvidenceOnlySuccessState<O, P, D, F>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let _ = state
        .physical
        .into_terminal()
        .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    state.proof.retain_fail_closed();
}

impl<O, P, D> EligibilityReadyEvidenceOnlySuccessGraph<O, P, D>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_into_prepared(
        mut self,
    ) -> Result<PreparedRetainedEvidenceOnlySuccess<P, D>, FinalizationContainmentError> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        Ok(state.try_commit()?.into_prepared())
    }
}

impl<O, P, D> Drop for EligibilityReadyEvidenceOnlySuccessGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            drop(state);
        }
    }
}

impl<P, D> EligibilityReadyEvidenceOnlySuccessPhysical<P, D> {
    fn into_terminal(self) -> TerminalPhysicalOwnershipManifest<P, D, (), (), ()> {
        TerminalPhysicalOwnershipManifest::new(
            DetachedRetainedPart::empty(),
            self.evidence,
            None,
            None,
            None,
        )
    }

    fn into_prepared(self) -> PreparedRetainedEvidenceOnlySuccess<P, D> {
        PreparedRetainedEvidenceOnlySuccess {
            evidence: PublishedPart::from_exact_owner(self.evidence),
        }
    }
}

impl<P, D> AccountBoundPublicationManifest for EligibilityReadyEvidenceOnlySuccessPhysical<P, D> {
    fn publication_account(&self) -> &PhysicalStorageAccount {
        self.evidence.account()
    }

    fn contain_uncommitted(self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        self.into_terminal().contain_new_fault(reason)
    }

    fn contain_after_quarantine(self, quarantine: AccountPoisoned) -> FinalizationContainmentError {
        self.into_terminal().contain_after_quarantine(quarantine)
    }
}

pub(super) struct PrimaryPublicationBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

impl<O, P, D> PrimaryPublicationBatches<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn new(
        canonical: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    ) -> Self {
        Self {
            canonical,
            evidence,
        }
    }
}

struct TerminalPrimaryAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    baseline: FallbackBatchBaseline<O, P, D>,
    batches: PrimaryPublicationBatches<O, P, D>,
}

impl<O, P, D> TerminalPrimaryAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn try_bind(&mut self, publish_canonical: bool) -> Result<(), LedgerCause<D>> {
        self.ledger.try_bind_primary(
            &mut self.baseline,
            self.batches.canonical.as_mut(),
            self.batches.evidence.as_mut(),
            publish_canonical,
        )
    }

    fn finish(self, publish_canonical: bool) -> crate::ledger::TypedFreezeProof<O, P, D> {
        self.ledger.finish_bound_primary(
            self.baseline,
            self.batches.canonical,
            self.batches.evidence,
            publish_canonical,
        )
    }

    fn into_fallback(self) -> TerminalFallbackAttribution<O, P, D> {
        TerminalFallbackAttribution {
            ledger: self.ledger,
            baseline: self.baseline,
            discard: self.batches,
        }
    }
}

pub(super) struct TerminalFallbackAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    baseline: FallbackBatchBaseline<O, P, D>,
    discard: PrimaryPublicationBatches<O, P, D>,
}

impl<O, P, D> TerminalFallbackAttribution<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) fn new(
        ledger: Ledger<O, P, D>,
        baseline: FallbackBatchBaseline<O, P, D>,
        discard: PrimaryPublicationBatches<O, P, D>,
    ) -> Self {
        Self {
            ledger,
            baseline,
            discard,
        }
    }

    fn try_prepare_sources(&mut self) -> Result<(), LedgerCause<D>> {
        self.ledger.try_bind_fallback_discards(
            &mut self.baseline,
            self.discard.canonical.as_mut(),
            self.discard.evidence.as_mut(),
        )
    }

    fn finish(self) -> crate::ledger::TypedFreezeProof<O, P, D> {
        self.baseline.finish_prepared_fallback_proof(
            self.ledger,
            self.discard.canonical,
            self.discard.evidence,
        )
    }
}

pub(super) enum TerminalPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Success(TerminalSuccessFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Rich(TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>),
    Fallback(
        TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        PreparationFailureCode,
    ),
}

pub(super) struct TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalPrimaryAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        ledger: Ledger<O, P, D>,
        baseline: FallbackBatchBaseline<O, P, D>,
        batches: PrimaryPublicationBatches<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(TerminalPrimaryAttribution {
                ledger,
                baseline,
                batches,
            }),
            physical: Some(physical),
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalPrimaryAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical(
        &self,
    ) -> &TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock> {
        match self.physical.as_ref() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_new_fault(reason);
        self.retain_attribution_after_physical_containment();
        cause
    }

    fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_after_recorded(reason);
        self.retain_attribution_after_physical_containment();
        cause
    }

    fn into_fallback(mut self) -> TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let attribution = match self.attribution.take() {
            Some(attribution) => attribution.into_fallback(),
            None => unreachable_final_owner_state(),
        };
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        TerminalFallbackGuard::new(attribution, physical)
    }

    fn try_bind_once(
        &mut self,
        publish_canonical: bool,
    ) -> Result<Result<(), LedgerCause<D>>, Box<dyn std::any::Any + Send>> {
        catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            maybe_panic_evidence_only_primary_bind(publish_canonical);
            self.attribution_mut().try_bind(publish_canonical)
        }))
    }

    fn contain_primary_binding_panic(
        self,
        payload: Box<dyn std::any::Any + Send>,
    ) -> RetainedPublicationPrepareError {
        discard_untrusted_panic_payload(payload);
        self.physical()
            .record_new_fault(AccountPoisonReason::PrimaryPublicationBindingPanicked);
        let containment =
            self.contain_after_recorded(AccountPoisonReason::PrimaryPublicationBindingPanicked);
        RetainedPublicationPrepareError::FinalizationContainment(containment)
    }

    pub(super) fn try_advance_frozen_success(
        mut self,
        terminal: RetainedTerminalMode,
    ) -> Result<
        FrozenSuccessPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        let RetainedTerminalMode::Success {
            canonical_root,
            evidence_root,
        } = terminal
        else {
            let containment = self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                containment,
            ));
        };
        match self.try_bind_once(true) {
            Err(payload) => return Err(self.contain_primary_binding_panic(payload)),
            Ok(Err(_cause)) => {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            Ok(Ok(())) => {}
        }
        if !self.physical().qualifies_pre_discard_success() {
            let containment = self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                containment,
            ));
        }
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .finish_pre_discard_success(canonical_root, evidence_root);
        let proof = self
            .attribution
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .finish(true);
        Ok(FrozenSuccessPrimaryTransition::Success(
            TerminalSuccessFreezeGuard::new(proof, physical),
        ))
    }

    pub(super) fn try_advance_evidence_only_success(
        mut self,
    ) -> Result<
        EvidenceOnlySuccessPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        match self.try_bind_once(false) {
            Err(payload) => return Err(self.contain_primary_binding_panic(payload)),
            Ok(Err(_cause)) => {
                return Ok(EvidenceOnlySuccessPrimaryTransition::Fallback(
                    self.into_fallback(),
                    PreparationFailureCode::ledger(),
                ));
            }
            Ok(Ok(())) => {}
        }
        if !self.physical().has_exact_fallback_shape() {
            let containment = self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                containment,
            ));
        }
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = self
            .attribution
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .finish(false);
        Ok(EvidenceOnlySuccessPrimaryTransition::Success(
            TerminalEvidenceOnlySuccessFreezeGuard::new(proof, physical),
        ))
    }

    pub(super) fn try_advance(
        mut self,
        mut terminal: RetainedTerminalMode,
    ) -> Result<
        TerminalPrimaryTransition<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        let publish_canonical = matches!(terminal, RetainedTerminalMode::Success { .. });
        match self.try_bind_once(publish_canonical) {
            Err(payload) => {
                return Err(self.contain_primary_binding_panic(payload));
            }
            Ok(Err(_cause)) => {
                let evidence_root = match terminal {
                    RetainedTerminalMode::Success { evidence_root, .. } => evidence_root,
                    RetainedTerminalMode::RichFailure { cause, .. }
                    | RetainedTerminalMode::MinimalFallback { cause } => {
                        return Ok(TerminalPrimaryTransition::Fallback(
                            self.into_fallback(),
                            cause_after_primary_ledger_rejection(cause),
                        ));
                    }
                };
                match self.try_bind_once(false) {
                    Err(payload) => {
                        return Err(self.contain_primary_binding_panic(payload));
                    }
                    Ok(Err(_cause)) => {
                        return Ok(TerminalPrimaryTransition::Fallback(
                            self.into_fallback(),
                            PreparationFailureCode::ledger(),
                        ));
                    }
                    Ok(Ok(())) => {
                        terminal = RetainedTerminalMode::RichFailure {
                            evidence_root,
                            cause: PreparationFailureCode::ledger(),
                        };
                    }
                }
            }
            Ok(Ok(())) => {}
        }

        if let RetainedTerminalMode::Success {
            canonical_root,
            evidence_root,
        } = terminal
        {
            if !self.physical().qualifies_pre_discard_success() {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state())
                .finish_pre_discard_success(canonical_root, evidence_root);
            let proof = self
                .attribution
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state())
                .finish(true);
            return Ok(TerminalPrimaryTransition::Success(
                TerminalSuccessFreezeGuard::new(proof, physical),
            ));
        }

        let (evidence_root, cause) = match terminal {
            RetainedTerminalMode::RichFailure {
                evidence_root,
                cause,
            } => (evidence_root, cause),
            RetainedTerminalMode::Success { .. } | RetainedTerminalMode::MinimalFallback { .. } => {
                unreachable_final_owner_state()
            }
        };
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(false),
            None => unreachable_final_owner_state(),
        };
        Ok(TerminalPrimaryTransition::Rich(
            TerminalRichFreezeGuard::new(proof, physical, evidence_root, cause),
        ))
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPrimaryGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }
}

pub(super) struct TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    attribution: Option<TerminalFallbackAttribution<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        attribution: TerminalFallbackAttribution<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Self {
        Self {
            attribution: Some(attribution),
            physical: Some(physical),
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalFallbackAttribution<O, P, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }

    fn contain_new_fault(mut self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let cause = physical.contain_new_fault(reason);
        self.retain_attribution_after_physical_containment();
        cause
    }

    pub(super) fn try_prepare(
        mut self,
        cause: PreparationFailureCode,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        RetainedPublicationPrepareError,
    > {
        match catch_unwind(AssertUnwindSafe(|| {
            self.attribution_mut().try_prepare_sources()
        })) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
            Err(payload) => {
                let containment =
                    self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                discard_untrusted_panic_payload(payload);
                return Err(RetainedPublicationPrepareError::FinalizationContainment(
                    containment,
                ));
            }
        }

        let physical = match self.physical.take() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        };
        let proof = match self.attribution.take() {
            Some(attribution) => attribution.finish(),
            None => unreachable_final_owner_state(),
        };
        TerminalMinimalFreezeGuard::new(proof, physical, cause)
            .into_prepared()
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalFallbackGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            std::mem::forget(attribution);
        }
    }
}

pub(super) fn prepare_terminal_control<O, P, PP, U, D>(
    coordinator: &mut OperationCoordinator<O, P, PP, U, D>,
    final_progress: FinalProgress<PP, U>,
) -> Option<ControlCause>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    coordinator.try_prepare_typed_control(final_progress).err()
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl PreparationFailureCode {
    pub(crate) fn from_control(cause: &ControlCause) -> Self {
        let code = match cause {
            ControlCause::Cancelled { .. } => 1,
            ControlCause::DeadlineExceeded => 2,
            ControlCause::ObserverPanicked => 3,
            ControlCause::CancellationSignalPanicked => 4,
            ControlCause::InvalidProgress { .. } => 5,
            ControlCause::ProgressStorage { .. } => 6,
        };
        Self::control(code)
    }

    pub(super) const fn from_closure_validation(cause: RetainedClosureValidationError) -> Self {
        let code = match cause {
            RetainedClosureValidationError::Poisoned => 1,
            RetainedClosureValidationError::WrongOutcomeMode => 2,
            RetainedClosureValidationError::InvalidRoot => 3,
            RetainedClosureValidationError::Rejected => 4,
            RetainedClosureValidationError::Panicked => 5,
        };
        Self::product_graph(code)
    }
}

/// Failure to establish a safe terminal publication; all uncertain accounting is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RetainedPublicationPrepareError {
    /// Final eligibility failed closed without producing any prepared or fallback owner.
    FinalizationContainment(FinalizationContainmentError),
}

impl fmt::Display for RetainedPublicationPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalizationContainment(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for RetainedPublicationPrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FinalizationContainment(cause) => Some(cause),
        }
    }
}

pub(super) fn drop_retained_table_with_account_anchor<Block>(
    table: RetainedTableAllocation<Block>,
    account: &PhysicalStorageAccount,
) -> RetainedStorageDropOutcome {
    let ownership = if table.belongs_to(account) {
        RetainedStorageDropOutcome::Clean
    } else {
        RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
            account,
            AccountPoisonReason::OwnershipGraphUnverified,
        ))
    };
    ownership.join(drop_retained_table_before_attribution(table, account))
}

fn drop_retained_table_before_attribution<Block>(
    table: RetainedTableAllocation<Block>,
    account: &PhysicalStorageAccount,
) -> RetainedStorageDropOutcome {
    if !table.belongs_to(account) {
        record_forgotten_table_edges(&table);
        std::mem::forget(table);
        return RetainedStorageDropOutcome::MissingFaultAnchor;
    }
    drop_retained_allocation_before_attribution(table, account)
}

pub(super) fn terminal_evidence_catalog_is_qualified<O, P, D>(
    ledger: &Ledger<O, P, D>,
    expected_dimensions: usize,
    expected_phases: usize,
) -> bool
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    #[cfg(test)]
    if take_injected_terminal_evidence_catalog_failure() {
        return false;
    }

    ledger.catalog_shape_is_strictly_ordered(expected_dimensions, expected_phases)
}

pub(super) struct QualifiedDetachedTerminalEvidence<P, D> {
    owner: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    qualification: Option<TerminalEvidenceFreezeQualification>,
}

pub(super) enum DetachedTerminalEvidenceQualification<P, D> {
    Qualified(QualifiedDetachedTerminalEvidence<P, D>),
    Rejected(DetachedRetainedPart<TerminalLedgerEvidence<P, D>>),
    Panicked(DetachedRetainedPart<TerminalLedgerEvidence<P, D>>),
}

impl<P, D> QualifiedDetachedTerminalEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(super) fn try_from_success_proof<O>(
        owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        proof: &QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    ) -> DetachedTerminalEvidenceQualification<P, D>
    where
        O: OperationOwner,
    {
        let qualification = catch_unwind(AssertUnwindSafe(|| {
            owner.allocation().as_ref().try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        }));
        match qualification {
            Ok(Ok(qualification)) => DetachedTerminalEvidenceQualification::Qualified(Self {
                owner: Some(owner),
                qualification: Some(qualification),
            }),
            Ok(Err(_)) => DetachedTerminalEvidenceQualification::Rejected(owner),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                DetachedTerminalEvidenceQualification::Panicked(owner)
            }
        }
    }

    pub(super) fn try_from_discard_proof<O>(
        owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        proof: &QualifiedTypedDiscardFreezeProof<O, P, D>,
    ) -> DetachedTerminalEvidenceQualification<P, D>
    where
        O: OperationOwner,
    {
        let qualification = catch_unwind(AssertUnwindSafe(|| {
            owner.allocation().as_ref().try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        }));
        match qualification {
            Ok(Ok(qualification)) => DetachedTerminalEvidenceQualification::Qualified(Self {
                owner: Some(owner),
                qualification: Some(qualification),
            }),
            Ok(Err(_)) => DetachedTerminalEvidenceQualification::Rejected(owner),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                DetachedTerminalEvidenceQualification::Panicked(owner)
            }
        }
    }
}

impl<P, D> QualifiedDetachedTerminalEvidence<P, D> {
    pub(super) fn into_freeze_ready(mut self) -> FreezeReadyDetachedTerminalEvidence<P, D> {
        let qualification = self
            .qualification
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let owner = self
            .owner
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        FreezeReadyDetachedTerminalEvidence {
            owner,
            qualification,
        }
    }

    pub(super) fn into_unfrozen_owner(
        mut self,
    ) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }
}

impl<P, D> FreezeReadyDetachedTerminalEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(super) fn freeze_in_place(
        &mut self,
        terminal_cause: TerminalEvidenceCause,
        snapshot: &crate::ledger::LedgerSnapshot<P, D>,
    ) {
        self.owner
            .allocation()
            .as_ref()
            .freeze_from(&self.qualification, terminal_cause, snapshot);
    }
}

impl<P, D> FreezeReadyDetachedTerminalEvidence<P, D> {
    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        self.owner.account()
    }

    pub(super) fn into_frozen_owner(self) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
    }

    pub(super) fn into_unfrozen_owner(self) -> DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        self.owner
    }
}

impl<P, D> Drop for QualifiedDetachedTerminalEvidence<P, D> {
    fn drop(&mut self) {
        if let Some(mut owner) = self.owner.take() {
            owner.record_forgotten_physical_edge();
            owner.fail_closed_after_recorded_poison();
        }
    }
}
