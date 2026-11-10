//! Terminal publication ownership, attribution, and freeze protocol.

use super::terminal_loan::{BorrowedTerminalEvidence, TerminalEvidenceQualification};
use super::*;

#[cfg(test)]
thread_local! {
    static PANIC_NEXT_PRIMARY_BIND: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn panic_next_evidence_only_primary_bind() {
    PANIC_NEXT_PRIMARY_BIND.with(|panic| panic.set(Some(false)));
}

#[cfg(test)]
pub(crate) fn panic_next_canonical_primary_bind() {
    PANIC_NEXT_PRIMARY_BIND.with(|panic| panic.set(Some(true)));
}

#[cfg(test)]
fn maybe_panic_evidence_only_primary_bind(publish_canonical: bool) {
    if PANIC_NEXT_PRIMARY_BIND.with(|panic| {
        if panic.get() == Some(publish_canonical) {
            panic.set(None);
            true
        } else {
            false
        }
    }) {
        panic!("injected terminal primary bind panic");
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

pub(super) struct TerminalSuccessFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    proof: Option<crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D>>,
    physical: Option<BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

pub(super) enum EvidenceOnlySuccessPrimaryTransition<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    Success(
        TerminalEvidenceOnlySuccessFreezeGuard<
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
    ),
    Fallback(
        TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        PreparationFailureCode,
    ),
}

struct QualifiedSuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    #[expect(
        clippy::type_complexity,
        reason = "the state preserves one invocation loan with its exact ledger and payload brands"
    )]
    state: Option<QualifiedSuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct QualifiedSuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    physical: BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
}

type QualifiedSuccessResult<'inv, O, P, PP, U, D, F, CB, EB> =
    Result<QualifiedSuccessGraph<'inv, O, P, PP, U, D, F, CB, EB>, FinalizationContainmentError>;

type PublicationReadySuccessResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    PublicationReadySuccessGraph<'inv, O, P, PP, U, D, F, CB, EB>,
    FinalizationContainmentError,
>;

struct BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock> {
    fallback: TerminalPartLoan<'inv, F>,
    evidence: BorrowedFreezeReadyDetachedTerminalEvidence<'inv, P, D>,
    canonical_table: TerminalTableLoan<'inv, CanonicalBlock>,
    evidence_table: TerminalTableLoan<'inv, EvidenceBlock>,
    account: TerminalAccountLoan<'inv>,
    canonical_root: RetainedSlotIdentity,
    evidence_root: RetainedSlotIdentity,
}

struct PublicationReadySuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    #[expect(
        clippy::type_complexity,
        reason = "the state preserves one invocation loan with its exact ledger and payload brands"
    )]
    state: Option<
        PublicationReadySuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
    >,
}

struct PublicationReadySuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    physical: BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    proof: FreezeReadyTypedProof<'inv, O, P, PP, U, D>,
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

struct VerifiedSuccessPhysicalDiscard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    state:
        VerifiedSuccessPhysicalDiscardState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the pending variant is the single linear owner; boxing would add allocation and split the ownership proof"
)]
enum VerifiedSuccessPhysicalDiscardState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    Pending {
        acknowledgement: SuccessDiscardAcknowledgement,
        physical: BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
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

pub(super) struct BorrowedFreezeReadyDetachedTerminalEvidence<'inv, P, D> {
    owner: BorrowedTerminalEvidence<'inv, P, D>,
    qualification: TerminalEvidenceQualification,
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    TerminalSuccessFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D>,
        physical: BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
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

    pub(super) fn into_deferred(
        self,
        set_id: RetainedSetId,
    ) -> Result<DeferredAttachedTerminal, FinalizationContainmentError> {
        self.into_qualified()?
            .into_publication_ready(set_id)?
            .defer_to_root()
    }

    fn into_qualified(
        mut self,
    ) -> QualifiedSuccessResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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
        let BorrowedExactPreDiscardSuccess {
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
            match BorrowedQualifiedDetachedTerminalEvidence::try_from_success_proof(
                evidence, &proof,
            ) {
                BorrowedDetachedTerminalEvidenceQualification::Qualified(evidence) => evidence,
                BorrowedDetachedTerminalEvidenceQualification::Rejected(evidence)
                | BorrowedDetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = BorrowedExactPreDiscardSuccess {
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
                physical: BorrowedQualifiedSuccessPhysical {
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
        Ok(qualified)
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(
        &mut self,
    ) -> QualifiedSuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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

    fn into_publication_ready(
        mut self,
        set_id: RetainedSetId,
    ) -> PublicationReadySuccessResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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
        Ok(publication_ready)
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedSuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn terminal_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .owner
            .as_ref()
            .or_else(|| self.evidence.account())
    }

    fn into_terminal(
        self,
    ) -> BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    {
        let Self {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root: _,
            evidence_root: _,
        } = self;
        BorrowedTerminalPhysicalOwnershipManifest::new(
            fallback,
            evidence.into_unfrozen_owner(),
            Some(canonical_table),
            Some(evidence_table),
            Some(account),
        )
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    PublicationReadySuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(
        &mut self,
    ) -> PublicationReadySuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        self.state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn defer_to_root(mut self) -> Result<DeferredAttachedTerminal, FinalizationContainmentError> {
        let PublicationReadySuccessState {
            physical,
            proof,
            set_id,
        } = self.take_state();
        let seal = if physical.evidence.matches_backend(proof.is_finite())
            && physical.terminal_account().is_some()
        {
            proof.seal_to_root()
        } else {
            Err(proof)
        };
        let stamp = match seal {
            Ok(stamp) => stamp,
            Err(proof) => {
                let cause = physical
                    .into_terminal()
                    .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                proof.retain_fail_closed();
                return Err(cause);
            }
        };
        let BorrowedQualifiedSuccessPhysical {
            fallback: _,
            evidence,
            canonical_table: _,
            evidence_table: _,
            account: _,
            canonical_root,
            evidence_root,
        } = physical;
        let (_, qualification) = evidence.into_deferred_parts();
        Ok(DeferredAttachedTerminal {
            stamp,
            qualification,
            disposition: TerminalDisposition::Success {
                canonical_root,
                evidence_root,
                set_id,
            },
        })
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    DetachedReadySuccessState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze(self) -> EligibilityReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let mut state = scopeguard::guard(
            self,
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
                        .test_storage_dimension_evidence_at(0)
                        .map(|snapshot| (dimension, snapshot))
                },
            ),
        };
        EligibilityReadySuccessGraph {
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
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    DynamicTerminalCapsule<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(super) fn finish_success(
        mut self,
    ) -> Result<
        RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        FinalizationContainmentError,
    > {
        let ready = self.proof.is_some()
            && self.physical.as_ref().is_some_and(|physical| {
                matches!(
                    physical.disposition.as_ref(),
                    Some(TerminalDisposition::Success { .. })
                ) && physical.qualification.is_some()
                    && physical
                        .graph
                        .fallback
                        .as_ref()
                        .is_some_and(DetachedRetainedPart::is_empty)
                    && physical.evidence.is_some()
                    && physical.graph.canonical_table.is_some()
                    && physical.graph.evidence_table.is_some()
                    && physical.graph.account.is_some()
            });
        if !ready {
            return Err(self.contain_success_fault());
        }

        let mut physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let Some(TerminalDisposition::Success {
            canonical_root,
            evidence_root,
            set_id,
        }) = physical.disposition.take()
        else {
            unreachable_final_owner_state()
        };
        let qualification = physical
            .qualification
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let state = DetachedReadySuccessState {
            physical: QualifiedSuccessPhysical {
                fallback: physical
                    .graph
                    .fallback
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state()),
                evidence: FreezeReadyDetachedTerminalEvidence::from_qualified_parts(
                    physical
                        .evidence
                        .take()
                        .unwrap_or_else(|| unreachable_final_owner_state()),
                    qualification,
                ),
                canonical_table: physical
                    .graph
                    .canonical_table
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state()),
                evidence_table: physical
                    .graph
                    .evidence_table
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state()),
                account: physical
                    .graph
                    .account
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state()),
                canonical_root,
                evidence_root,
            },
            proof: self
                .proof
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            set_id,
        };
        Ok(state.freeze().into_joint_commit_intent())
    }

    pub(super) fn finish_evidence_only(
        mut self,
    ) -> Result<PreparedRetainedEvidenceOnlySuccess<P, D>, FinalizationContainmentError> {
        let ready = self.proof.is_some()
            && self.physical.as_ref().is_some_and(|physical| {
                matches!(
                    physical.disposition.as_ref(),
                    Some(TerminalDisposition::EvidenceOnlySuccess)
                ) && physical.qualification.is_some()
                    && physical
                        .graph
                        .fallback
                        .as_ref()
                        .is_some_and(DetachedRetainedPart::is_empty)
                    && physical.evidence.is_some()
                    && physical.graph.canonical_table.is_none()
                    && physical.graph.evidence_table.is_none()
                    && physical.graph.account.is_none()
            });
        if !ready {
            return Err(self.contain_success_fault());
        }

        let mut physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let qualification = physical
            .qualification
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let state = DetachedReadyEvidenceOnlySuccessState {
            physical: QualifiedEvidenceOnlySuccessPhysical {
                fallback: physical
                    .graph
                    .fallback
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state()),
                evidence: FreezeReadyDetachedTerminalEvidence::from_qualified_parts(
                    physical
                        .evidence
                        .take()
                        .unwrap_or_else(|| unreachable_final_owner_state()),
                    qualification,
                ),
            },
            proof: self
                .proof
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
        };
        state.freeze().try_into_prepared()
    }

    fn contain_success_fault(mut self) -> FinalizationContainmentError {
        let cause = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        if let Some(proof) = self.proof.take() {
            proof.retain_fail_closed();
        }
        cause
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

fn contain_publication_ready_success_state<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>(
    state: PublicationReadySuccessState<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
) where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for PublicationReadySuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_publication_ready_success_state(state);
        }
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    VerifiedSuccessPhysicalDiscard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn new(
        physical: BorrowedQualifiedSuccessPhysical<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
        proof: QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
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
    ) -> PublicationReadySuccessGraph<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for VerifiedSuccessPhysicalDiscard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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
    fn into_joint_commit_intent(
        self,
    ) -> RetainedSuccessJointCommitIntent<O, P, D, F, CanonicalBlock, EvidenceBlock> {
        let state = self;
        #[cfg(test)]
        let state = maybe_drop_success_guard(SuccessGuardDropStage::EligibilityReady, state);
        RetainedSuccessJointCommitIntent { state }
    }

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
    fn into_terminal(self) -> TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock> {
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
        TerminalPhysicalState::new(
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalSuccessFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

pub(super) struct TerminalEvidenceOnlySuccessFreezeGuard<
    'inv,
    O,
    P,
    PP,
    U,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    proof: Option<crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D>>,
    physical: Option<
        BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    >,
}

struct BorrowedQualifiedEvidenceOnlySuccessPhysical<'inv, P, D, F> {
    fallback: TerminalPartLoan<'inv, F>,
    evidence: BorrowedFreezeReadyDetachedTerminalEvidence<'inv, P, D>,
    account: Option<TerminalAccountLoan<'inv>>,
}

struct QualifiedEvidenceOnlySuccessState<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    physical: BorrowedQualifiedEvidenceOnlySuccessPhysical<'inv, P, D, F>,
    proof: QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
}

struct QualifiedEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    state: Option<QualifiedEvidenceOnlySuccessState<'inv, O, P, PP, U, D, F>>,
}

type QualifiedEvidenceOnlySuccessResult<'inv, O, P, PP, U, D, F> = Result<
    QualifiedEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>,
    FinalizationContainmentError,
>;

type FreezeReadyEvidenceOnlySuccessResult<'inv, O, P, PP, U, D, F> = Result<
    FreezeReadyEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>,
    FinalizationContainmentError,
>;

#[allow(
    clippy::large_enum_variant,
    reason = "the bounded discard proof stays inline so terminal acknowledgement cannot allocate"
)]
enum VerifiedEvidenceOnlySuccessDiscardState<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    Pending {
        acknowledgement: SuccessDiscardAcknowledgement,
        physical: BorrowedQualifiedEvidenceOnlySuccessPhysical<'inv, P, D, F>,
        proof: QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
        discarded_account: PhysicalStorageAccount,
    },
    Acknowledged,
}

struct VerifiedEvidenceOnlySuccessDiscard<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    state: VerifiedEvidenceOnlySuccessDiscardState<'inv, O, P, PP, U, D, F>,
}

struct FreezeReadyEvidenceOnlySuccessState<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    physical: BorrowedQualifiedEvidenceOnlySuccessPhysical<'inv, P, D, F>,
    proof: FreezeReadyTypedProof<'inv, O, P, PP, U, D>,
}

struct FreezeReadyEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    state: Option<FreezeReadyEvidenceOnlySuccessState<'inv, O, P, PP, U, D, F>>,
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

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    TerminalEvidenceOnlySuccessFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn new(
        proof: crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D>,
        physical: BorrowedTerminalPhysicalOwnershipManifest<
            'inv,
            P,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
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

    pub(super) fn into_deferred(
        self,
    ) -> Result<DeferredAttachedTerminal, FinalizationContainmentError> {
        self.into_qualified()?.into_freeze_ready()?.defer_to_root()
    }

    fn into_qualified(mut self) -> QualifiedEvidenceOnlySuccessResult<'inv, O, P, PP, U, D, F> {
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
        if canonical_table.is_some()
            || evidence_table.is_some()
            || account.is_some() != evidence.is_finite()
        {
            let physical = BorrowedTerminalPhysicalOwnershipManifest::new(
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
        let evidence = match BorrowedQualifiedDetachedTerminalEvidence::try_from_success_proof(
            evidence, &proof,
        ) {
            BorrowedDetachedTerminalEvidenceQualification::Qualified(evidence) => {
                evidence.into_freeze_ready()
            }
            BorrowedDetachedTerminalEvidenceQualification::Rejected(evidence)
            | BorrowedDetachedTerminalEvidenceQualification::Panicked(evidence) => {
                let physical = BorrowedTerminalPhysicalOwnershipManifest::<P, D, F, (), ()>::new(
                    fallback, evidence, None, None, account,
                );
                let cause = physical
                    .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                proof.retain_fail_closed();
                return Err(cause);
            }
        };
        Ok(QualifiedEvidenceOnlySuccessGraph {
            state: Some(QualifiedEvidenceOnlySuccessState {
                physical: BorrowedQualifiedEvidenceOnlySuccessPhysical {
                    fallback,
                    evidence,
                    account,
                },
                proof,
            }),
        })
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalEvidenceOnlySuccessFreezeGuard<
        'inv,
        O,
        P,
        PP,
        U,
        D,
        F,
        CanonicalBlock,
        EvidenceBlock,
    >
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

impl<'inv, P, D, F> BorrowedQualifiedEvidenceOnlySuccessPhysical<'inv, P, D, F> {
    fn terminal_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account.as_deref().or_else(|| self.evidence.account())
    }

    fn into_terminal(self) -> BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, (), ()> {
        BorrowedTerminalPhysicalOwnershipManifest::new(
            self.fallback,
            self.evidence.into_unfrozen_owner(),
            None,
            None,
            self.account,
        )
    }
}

impl<'inv, O, P, PP, U, D, F> QualifiedEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn take_state(&mut self) -> QualifiedEvidenceOnlySuccessState<'inv, O, P, PP, U, D, F> {
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

    fn into_freeze_ready(
        mut self,
    ) -> FreezeReadyEvidenceOnlySuccessResult<'inv, O, P, PP, U, D, F> {
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
        Ok(VerifiedEvidenceOnlySuccessDiscard {
            state: VerifiedEvidenceOnlySuccessDiscardState::Pending {
                acknowledgement: SuccessDiscardAcknowledgement::NotStarted,
                physical: state.physical,
                proof: state.proof,
                discarded_account,
            },
        }
        .acknowledge())
    }
}

impl<'inv, O, P, PP, U, D, F> Drop for QualifiedEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

impl<'inv, O, P, PP, U, D, F> VerifiedEvidenceOnlySuccessDiscard<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn acknowledge(mut self) -> FreezeReadyEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F> {
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

impl<'inv, O, P, PP, U, D, F> Drop for VerifiedEvidenceOnlySuccessDiscard<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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

impl<'inv, O, P, PP, U, D, F> FreezeReadyEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn defer_to_root(mut self) -> Result<DeferredAttachedTerminal, FinalizationContainmentError> {
        let FreezeReadyEvidenceOnlySuccessState { physical, proof } = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let seal = if physical.evidence.matches_backend(proof.is_finite())
            && physical.account.is_some() == physical.evidence.is_finite()
            && physical.terminal_account().is_some()
        {
            proof.seal_to_root()
        } else {
            Err(proof)
        };
        let stamp = match seal {
            Ok(stamp) => stamp,
            Err(proof) => {
                let cause = physical
                    .into_terminal()
                    .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                proof.retain_fail_closed();
                return Err(cause);
            }
        };
        let BorrowedQualifiedEvidenceOnlySuccessPhysical {
            fallback: _,
            evidence,
            account: _,
        } = physical;
        let (_, qualification) = evidence.into_deferred_parts();
        Ok(DeferredAttachedTerminal {
            stamp,
            qualification,
            disposition: TerminalDisposition::EvidenceOnlySuccess,
        })
    }
}

impl<O, P, D, F> DetachedReadyEvidenceOnlySuccessState<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze(self) -> EligibilityReadyEvidenceOnlySuccessGraph<O, P, D> {
        let mut state = scopeguard::guard(self, contain_detached_ready_evidence_only_success_state);
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
    }
}

impl<'inv, O, P, PP, U, D, F> Drop for FreezeReadyEvidenceOnlySuccessGraph<'inv, O, P, PP, U, D, F>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
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
    fn into_terminal(self) -> TerminalPhysicalState<P, D, (), (), ()> {
        TerminalPhysicalState::new(
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

pub(super) struct PrimaryPublicationBatches<'inv, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: Option<&'inv mut Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>>,
    evidence:
        Option<&'inv mut Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>>,
}

impl<'inv, O, P, D> PrimaryPublicationBatches<'inv, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn new(
        canonical: Option<&'inv mut Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>>,
        evidence: Option<
            &'inv mut Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        >,
    ) -> Self {
        Self {
            canonical,
            evidence,
        }
    }
}

struct TerminalPrimaryAttribution<'inv, O, P, PP, U, D>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    ledger: TerminalInvocationLoan<'inv, O, P, PP, U, D>,
    baseline: crate::ledger::FallbackBatchView<'inv, O, P, D>,
    batches: PrimaryPublicationBatches<'inv, O, P, D>,
}

impl<'inv, O, P, PP, U, D> TerminalPrimaryAttribution<'inv, O, P, PP, U, D>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn retain_fail_closed(self) {
        let Self {
            ledger,
            baseline: _,
            batches: _,
        } = self;
        ledger.retain_fail_closed();
    }

    fn try_bind(&mut self, publish_canonical: bool) -> Result<(), LedgerCause<D>> {
        self.ledger.as_read().try_bind_primary_rows(
            self.baseline.reborrow(),
            self.batches
                .canonical
                .as_deref_mut()
                .and_then(Option::as_mut),
            self.batches
                .evidence
                .as_deref_mut()
                .and_then(Option::as_mut),
            publish_canonical,
        )
    }

    fn finish(
        self,
        publish_canonical: bool,
    ) -> crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D> {
        self.ledger.finish_bound_primary(
            self.baseline,
            self.batches.canonical.and_then(Option::as_mut),
            self.batches.evidence.and_then(Option::as_mut),
            publish_canonical,
        )
    }

    fn into_fallback(self) -> TerminalFallbackAttribution<'inv, O, P, PP, U, D> {
        TerminalFallbackAttribution {
            ledger: self.ledger,
            baseline: self.baseline,
            discard: self.batches,
        }
    }
}

pub(super) struct TerminalFallbackAttribution<'inv, O, P, PP, U, D>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    ledger: TerminalInvocationLoan<'inv, O, P, PP, U, D>,
    baseline: crate::ledger::FallbackBatchView<'inv, O, P, D>,
    discard: PrimaryPublicationBatches<'inv, O, P, D>,
}

impl<'inv, O, P, PP, U, D> TerminalFallbackAttribution<'inv, O, P, PP, U, D>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn retain_fail_closed(self) {
        let Self {
            ledger,
            baseline: _,
            discard: _,
        } = self;
        ledger.retain_fail_closed();
    }

    pub(super) fn new(
        ledger: TerminalInvocationLoan<'inv, O, P, PP, U, D>,
        baseline: crate::ledger::FallbackBatchView<'inv, O, P, D>,
        discard: PrimaryPublicationBatches<'inv, O, P, D>,
    ) -> Self {
        Self {
            ledger,
            baseline,
            discard,
        }
    }

    fn try_prepare_sources(&mut self) -> Result<(), LedgerCause<D>> {
        self.ledger.as_read().try_bind_fallback_discard_rows(
            self.baseline.reborrow(),
            self.discard
                .canonical
                .as_deref_mut()
                .and_then(Option::as_mut),
            self.discard
                .evidence
                .as_deref_mut()
                .and_then(Option::as_mut),
        )
    }

    fn finish(self) -> crate::ledger::TypedFreezeProof<'inv, O, P, PP, U, D> {
        self.baseline.finish_prepared_fallback_proof(
            self.ledger,
            self.discard.canonical.and_then(Option::as_mut),
            self.discard.evidence.and_then(Option::as_mut),
        )
    }
}

#[must_use = "the selected unqualified terminal graph must be finished or contained"]
pub(super) enum TerminalPreFreezeTransition<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    Success {
        guard: TerminalPrimaryGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    },
    Rich {
        guard: TerminalPrimaryGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
    },
    Fallback {
        guard: TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        cause: PreparationFailureCode,
    },
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPreFreezeTransition<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn physical(
        &self,
    ) -> &BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    {
        match self {
            Self::Success { guard, .. } | Self::Rich { guard, .. } => guard.physical(),
            Self::Fallback { guard, .. } => guard
                .physical
                .as_ref()
                .unwrap_or_else(|| unreachable_final_owner_state()),
        }
    }

    fn contain_new_fault(self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        match self {
            Self::Success { guard, .. } | Self::Rich { guard, .. } => {
                guard.contain_new_fault(reason)
            }
            Self::Fallback { guard, .. } => guard.contain_new_fault(reason),
        }
    }

    fn contain_after_recorded(self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        match self {
            Self::Success { guard, .. } | Self::Rich { guard, .. } => {
                guard.contain_after_recorded(reason)
            }
            Self::Fallback { guard, .. } => guard.contain_after_recorded(reason),
        }
    }

    pub(super) fn check_payload_cleanup(self) -> Result<Self, FinalizationContainmentError> {
        let Some(account) = self.physical().terminal_account() else {
            return Err(self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified));
        };
        let preflight = account.try_begin_account_preflight().map(drop);
        match preflight {
            Ok(()) => Ok(self),
            Err(quarantine) => {
                Err(self.contain_after_recorded(quarantined_account_reason(quarantine)))
            }
        }
    }

    pub(super) fn contain_activation_fault(self) -> FinalizationContainmentError {
        match self.check_payload_cleanup() {
            Ok(state) => state.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified),
            Err(cause) => cause,
        }
    }

    pub(super) fn finish_deferred(
        self,
        set_id: RetainedSetId,
    ) -> Result<DeferredAttachedTerminal, RetainedPublicationPrepareError> {
        match self {
            Self::Success {
                guard,
                canonical_root,
                evidence_root,
            } => guard
                .into_success_freeze(canonical_root, evidence_root)
                .into_deferred(set_id)
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            Self::Rich {
                guard,
                evidence_root,
                cause,
            } => guard
                .into_rich_freeze(evidence_root, cause)
                .into_deferred(set_id)
                .map_err(RetainedPublicationPrepareError::FinalizationContainment),
            Self::Fallback { guard, cause } => guard.try_prepare_deferred(cause),
        }
    }

    #[cfg(test)]
    pub(super) fn test_evidence_is_frozen(&self) -> bool {
        match self.physical().evidence.as_ref() {
            Some(BorrowedTerminalEvidence::Dynamic(evidence)) => {
                evidence.allocation().as_ref().is_frozen()
            }
            Some(BorrowedTerminalEvidence::Finite) => false,
            None => unreachable_final_owner_state(),
        }
    }
}

pub(super) struct TerminalPrimaryGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    attribution: Option<TerminalPrimaryAttribution<'inv, O, P, PP, U, D>>,
    physical: Option<
        BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    >,
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPrimaryGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        ledger: TerminalInvocationLoan<'inv, O, P, PP, U, D>,
        baseline: crate::ledger::FallbackBatchView<'inv, O, P, D>,
        batches: PrimaryPublicationBatches<'inv, O, P, D>,
        physical: BorrowedTerminalPhysicalOwnershipManifest<
            'inv,
            P,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
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

    fn attribution_mut(&mut self) -> &mut TerminalPrimaryAttribution<'inv, O, P, PP, U, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn physical(
        &self,
    ) -> &BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    {
        match self.physical.as_ref() {
            Some(physical) => physical,
            None => unreachable_final_owner_state(),
        }
    }

    fn check_backend(self) -> Result<Self, RetainedPublicationPrepareError> {
        let is_finite = self
            .attribution
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .ledger
            .is_finite();
        if self.physical().matches_backend(is_finite) {
            Ok(self)
        } else {
            Err(RetainedPublicationPrepareError::FinalizationContainment(
                self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified),
            ))
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            attribution.retain_fail_closed();
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

    fn into_success_freeze(
        mut self,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    ) -> TerminalSuccessFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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
        TerminalSuccessFreezeGuard::new(proof, physical)
    }

    fn into_rich_freeze(
        mut self,
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
    ) -> TerminalRichFreezeGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        let physical = self
            .physical
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = self
            .attribution
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .finish(false);
        TerminalRichFreezeGuard::new(proof, physical, evidence_root, cause)
    }

    fn into_fallback(
        mut self,
    ) -> TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
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

    #[expect(
        clippy::type_complexity,
        reason = "the transition preserves the invocation lifetime and exact ledger and payload brands"
    )]
    pub(super) fn try_classify_frozen_success(
        mut self,
        terminal: RetainedTerminalMode,
    ) -> Result<
        TerminalPreFreezeTransition<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        self = self.check_backend()?;
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
        Ok(TerminalPreFreezeTransition::Success {
            guard: self,
            canonical_root,
            evidence_root,
        })
    }

    #[expect(
        clippy::type_complexity,
        reason = "both linear branches preserve the invocation lifetime and exact ledger and payload brands"
    )]
    pub(super) fn try_advance_evidence_only_success(
        mut self,
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
        self = self.check_backend()?;
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

    #[expect(
        clippy::type_complexity,
        reason = "the transition preserves the invocation lifetime and exact ledger and payload brands"
    )]
    pub(super) fn try_classify(
        mut self,
        mut terminal: RetainedTerminalMode,
    ) -> Result<
        TerminalPreFreezeTransition<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>,
        RetainedPublicationPrepareError,
    > {
        self = self.check_backend()?;
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
                        return Ok(TerminalPreFreezeTransition::Fallback {
                            guard: self.into_fallback(),
                            cause: cause_after_primary_ledger_rejection(cause),
                        });
                    }
                };
                match self.try_bind_once(false) {
                    Err(payload) => {
                        return Err(self.contain_primary_binding_panic(payload));
                    }
                    Ok(Err(_cause)) => {
                        return Ok(TerminalPreFreezeTransition::Fallback {
                            guard: self.into_fallback(),
                            cause: PreparationFailureCode::ledger(),
                        });
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
            return Ok(TerminalPreFreezeTransition::Success {
                guard: self,
                canonical_root,
                evidence_root,
            });
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
        Ok(TerminalPreFreezeTransition::Rich {
            guard: self,
            evidence_root,
            cause,
        })
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPrimaryGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            attribution.retain_fail_closed();
        }
    }
}

pub(super) struct TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    attribution: Option<TerminalFallbackAttribution<'inv, O, P, PP, U, D>>,
    physical: Option<
        BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>,
    >,
}

type MinimalFreezeResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    TerminalMinimalFreezeGuard<'inv, O, P, PP, U, D, F, CB, EB>,
    RetainedPublicationPrepareError,
>;

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
    TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        attribution: TerminalFallbackAttribution<'inv, O, P, PP, U, D>,
        physical: BorrowedTerminalPhysicalOwnershipManifest<
            'inv,
            P,
            D,
            F,
            CanonicalBlock,
            EvidenceBlock,
        >,
    ) -> Self {
        Self {
            attribution: Some(attribution),
            physical: Some(physical),
        }
    }

    fn attribution_mut(&mut self) -> &mut TerminalFallbackAttribution<'inv, O, P, PP, U, D> {
        match self.attribution.as_mut() {
            Some(attribution) => attribution,
            None => unreachable_final_owner_state(),
        }
    }

    fn retain_attribution_after_physical_containment(&mut self) {
        if let Some(attribution) = self.attribution.take() {
            attribution.retain_fail_closed();
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

    pub(super) fn try_prepare_deferred(
        self,
        cause: PreparationFailureCode,
    ) -> Result<DeferredAttachedTerminal, RetainedPublicationPrepareError> {
        self.into_minimal_freeze(cause)?
            .into_deferred()
            .map_err(RetainedPublicationPrepareError::FinalizationContainment)
    }

    fn into_minimal_freeze(
        mut self,
        cause: PreparationFailureCode,
    ) -> MinimalFreezeResult<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> {
        let is_finite = self
            .attribution
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .ledger
            .is_finite();
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .matches_backend(is_finite)
        {
            return Err(RetainedPublicationPrepareError::FinalizationContainment(
                self.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified),
            ));
        }
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
        Ok(TerminalMinimalFreezeGuard::new(proof, physical, cause))
    }
}

impl<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalFallbackGuard<'inv, O, P, PP, U, D, F, CanonicalBlock, EvidenceBlock>
where
    PP: ProgressPhase,
    U: ProgressUnit,
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if let Some(physical) = self.physical.take() {
            let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        }
        if let Some(attribution) = self.attribution.take() {
            attribution.retain_fail_closed();
        }
    }
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

pub(super) struct BorrowedQualifiedDetachedTerminalEvidence<'inv, P, D> {
    owner: Option<BorrowedTerminalEvidence<'inv, P, D>>,
    qualification: Option<TerminalEvidenceQualification>,
}

pub(super) enum BorrowedDetachedTerminalEvidenceQualification<'inv, P, D> {
    Qualified(BorrowedQualifiedDetachedTerminalEvidence<'inv, P, D>),
    Rejected(BorrowedTerminalEvidence<'inv, P, D>),
    Panicked(BorrowedTerminalEvidence<'inv, P, D>),
}

impl<'inv, P, D> BorrowedQualifiedDetachedTerminalEvidence<'inv, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn try_from_success_proof<O, PP: ProgressPhase, U: ProgressUnit>(
        owner: BorrowedTerminalEvidence<'inv, P, D>,
        proof: &QualifiedEvidenceDiscardFreezeProof<'inv, O, P, PP, U, D>,
    ) -> BorrowedDetachedTerminalEvidenceQualification<'inv, P, D>
    where
        O: OperationOwner,
    {
        Self::try_qualify(owner, proof.is_finite(), |evidence| {
            evidence.try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        })
    }

    pub(super) fn try_from_discard_proof<O, PP: ProgressPhase, U: ProgressUnit>(
        owner: BorrowedTerminalEvidence<'inv, P, D>,
        proof: &QualifiedTypedDiscardFreezeProof<'inv, O, P, PP, U, D>,
    ) -> BorrowedDetachedTerminalEvidenceQualification<'inv, P, D>
    where
        O: OperationOwner,
    {
        Self::try_qualify(owner, proof.is_finite(), |evidence| {
            evidence.try_qualify(
                proof.dimension_count(),
                |index| proof.dimension_at(index),
                proof.phase_count(),
                |index| proof.phase_at(index),
                proof.last_completed_phase(),
            )
        })
    }

    fn try_qualify(
        owner: BorrowedTerminalEvidence<'inv, P, D>,
        is_finite: bool,
        qualify: impl FnOnce(
            &TerminalLedgerEvidence<P, D>,
        ) -> Result<
            TerminalEvidenceFreezeQualification,
            super::evidence::TerminalEvidenceFreezeError,
        >,
    ) -> BorrowedDetachedTerminalEvidenceQualification<'inv, P, D> {
        if owner.is_finite() != is_finite {
            return BorrowedDetachedTerminalEvidenceQualification::Rejected(owner);
        }
        let qualification = match &owner {
            BorrowedTerminalEvidence::Dynamic(owner) => catch_unwind(AssertUnwindSafe(|| {
                qualify(owner.allocation().as_ref()).map(TerminalEvidenceQualification::Dynamic)
            })),
            BorrowedTerminalEvidence::Finite => Ok(Ok(TerminalEvidenceQualification::Finite)),
        };
        match qualification {
            Ok(Ok(qualification)) => {
                BorrowedDetachedTerminalEvidenceQualification::Qualified(Self {
                    owner: Some(owner),
                    qualification: Some(qualification),
                })
            }
            Ok(Err(_)) => BorrowedDetachedTerminalEvidenceQualification::Rejected(owner),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                BorrowedDetachedTerminalEvidenceQualification::Panicked(owner)
            }
        }
    }
}

impl<'inv, P, D> BorrowedQualifiedDetachedTerminalEvidence<'inv, P, D> {
    pub(super) fn into_freeze_ready(
        mut self,
    ) -> BorrowedFreezeReadyDetachedTerminalEvidence<'inv, P, D> {
        let qualification = self
            .qualification
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let owner = self
            .owner
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        BorrowedFreezeReadyDetachedTerminalEvidence {
            owner,
            qualification,
        }
    }
}

impl<'inv, P, D> BorrowedFreezeReadyDetachedTerminalEvidence<'inv, P, D> {
    pub(super) fn into_deferred_parts(
        self,
    ) -> (
        BorrowedTerminalEvidence<'inv, P, D>,
        TerminalEvidenceQualification,
    ) {
        (self.owner, self.qualification)
    }

    pub(super) const fn is_finite(&self) -> bool {
        self.owner.is_finite()
    }

    pub(super) fn matches_backend(&self, is_finite: bool) -> bool {
        self.owner.is_finite() == is_finite && self.qualification.is_finite() == is_finite
    }

    pub(super) fn account(&self) -> Option<&PhysicalStorageAccount> {
        self.owner
            .dynamic()
            .and_then(DetachedRetainedPart::account_anchor)
    }

    pub(super) fn into_unfrozen_owner(self) -> BorrowedTerminalEvidence<'inv, P, D> {
        self.owner
    }
}

impl<'inv, P, D> Drop for BorrowedQualifiedDetachedTerminalEvidence<'inv, P, D> {
    fn drop(&mut self) {
        if let Some(BorrowedTerminalEvidence::Dynamic(mut owner)) = self.owner.take() {
            owner.record_forgotten_physical_edge();
            owner.fail_closed_after_recorded_poison();
        }
    }
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

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedSuccessPhysical<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn into_terminal(self) -> TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock> {
        let Self {
            fallback,
            evidence,
            canonical_table,
            evidence_table,
            account,
            canonical_root: _,
            evidence_root: _,
        } = self;
        TerminalPhysicalState::new(
            fallback,
            evidence.into_unfrozen_owner(),
            Some(canonical_table),
            Some(evidence_table),
            Some(account),
        )
    }
}

struct QualifiedEvidenceOnlySuccessPhysical<P, D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
}

impl<P, D, F> QualifiedEvidenceOnlySuccessPhysical<P, D, F> {
    fn into_terminal(self) -> TerminalPhysicalState<P, D, F, (), ()> {
        TerminalPhysicalState::new(
            self.fallback,
            self.evidence.into_unfrozen_owner(),
            None,
            None,
            None,
        )
    }
}

pub(super) struct FreezeReadyDetachedTerminalEvidence<P, D> {
    owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    qualification: TerminalEvidenceFreezeQualification,
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
    pub(super) fn from_qualified_parts(
        owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        qualification: TerminalEvidenceFreezeQualification,
    ) -> Self {
        Self {
            owner,
            qualification,
        }
    }

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
