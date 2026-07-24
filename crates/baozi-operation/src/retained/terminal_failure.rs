use super::*;

pub(super) struct TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
    evidence_root: RetainedSlotIdentity,
    cause: PreparationFailureCode,
}

struct QualifiedRichOutput<P, D, F, EvidenceBlock> {
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    evidence_root: RetainedSlotIdentity,
    cause: PreparationFailureCode,
    set_id: RetainedSetId,
    fallback_type: PhantomData<fn() -> F>,
}

enum QualifiedRichDiscard<O, P, D, F, CanonicalBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Evidence {
        fallback: DetachedRetainedPart<F>,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    },
    Both {
        canonical_table: RetainedTableAllocation<CanonicalBlock>,
        fallback: DetachedRetainedPart<F>,
        proof: QualifiedBothDiscardFreezeProof<O, P, D>,
    },
}

struct QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
}

struct QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
    eligibility: PublicationEligibility,
}

struct EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct FreezeReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    proof: FreezeReadyTypedProof<O, P, D>,
    eligibility: PublicationEligibility,
}

struct FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyRichState<O, P, D, F, EvidenceBlock>>,
}

#[allow(
    clippy::type_complexity,
    reason = "the tuple moves one exact physical graph beside its matching typed ledger proof"
)]
fn rich_state_into_terminal<P, D, F, CanonicalBlock, EvidenceBlock, O>(
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    discard: QualifiedRichDiscard<O, P, D, F, CanonicalBlock>,
) -> (
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    QualifiedTypedDiscardFreezeProof<O, P, D>,
)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    match discard {
        QualifiedRichDiscard::Evidence { fallback, proof } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Evidence(proof),
        ),
        QualifiedRichDiscard::Both {
            canonical_table,
            fallback,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Both(proof),
        ),
    }
}

fn contain_qualified_rich_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: QualifiedRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (physical, proof) = rich_state_into_terminal(state.output, state.discard);
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    cause
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
        evidence_root: RetainedSlotIdentity,
        cause: PreparationFailureCode,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            evidence_root,
            cause,
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
        set_id: RetainedSetId,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_discards() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        let expect_canonical = match &proof {
            QualifiedTypedDiscardFreezeProof::Evidence(_) => false,
            QualifiedTypedDiscardFreezeProof::Both(_) => true,
            _ => {
                let physical = self
                    .physical
                    .take()
                    .unwrap_or_else(|| unreachable_final_owner_state());
                let cause =
                    physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                proof.retain_fail_closed();
                return Err(cause);
            }
        };
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .qualifies_pre_cleanup_rich(expect_canonical)
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
        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_discard_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => {
                    evidence.into_freeze_ready()
                }
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = TerminalPhysicalOwnershipManifest::new(
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                    );
                    let cause = physical
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };
        let evidence_table = evidence_table.unwrap_or_else(|| unreachable_final_owner_state());
        let account = account.unwrap_or_else(|| unreachable_final_owner_state());
        let output = QualifiedRichOutput {
            evidence: qualified_evidence,
            evidence_table,
            account,
            evidence_root: self.evidence_root,
            cause: self.cause,
            set_id,
            fallback_type: PhantomData,
        };
        let discard = match (proof, canonical_table) {
            (QualifiedTypedDiscardFreezeProof::Evidence(proof), None) => {
                QualifiedRichDiscard::Evidence { fallback, proof }
            }
            (QualifiedTypedDiscardFreezeProof::Both(proof), Some(canonical_table)) => {
                QualifiedRichDiscard::Both {
                    canonical_table,
                    fallback,
                    proof,
                }
            }
            _ => unreachable_final_owner_state(),
        };
        let qualified = QualifiedRichGraph {
            state: Some(QualifiedRichState { output, discard }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::Qualified,
            qualified,
        );
        qualified.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalRichFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
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

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let eligibility = match state.output.account.try_publication_eligibility() {
            Ok(eligibility) => eligibility,
            Err(quarantine) => {
                let reason = quarantine.snapshot().first_poison_evidence().map_or(
                    AccountPoisonReason::OwnershipGraphUnverified,
                    AccountPoisonEvidence::reason,
                );
                return Err(contain_qualified_rich_state(state, reason, true));
            }
        };
        let eligible = EligibleRichGraph {
            state: Some(EligibleRichState {
                output: state.output,
                discard: state.discard,
                eligibility,
            }),
        };
        #[cfg(test)]
        let eligible = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::Eligible,
            eligible,
        );
        eligible.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_qualified_rich_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_eligible_rich_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: EligibleRichState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let EligibleRichState {
        output,
        discard,
        eligibility,
    } = state;
    let cause = contain_qualified_rich_state(
        QualifiedRichState { output, discard },
        reason,
        already_recorded,
    );
    drop(eligibility);
    cause
}

enum RichDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Both(QualifiedBothDiscardFreezeProof<O, P, D>),
    Evidence(QualifiedEvidenceDiscardFreezeProof<O, P, D>),
}

impl<O, P, D> RichDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn retain_fail_closed(self) {
        match self {
            Self::Both(proof) => proof.retain_fail_closed(),
            Self::Evidence(proof) => proof.retain_fail_closed(),
        }
    }
}

struct RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    fallback: DetachedRetainedPart<F>,
    proof: RichDiscardContinuationProof<O, P, D>,
    eligibility: PublicationEligibility,
    discarded_account: Option<PhysicalStorageAccount>,
}

fn contain_rich_discard_continuation_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let RichDiscardContinuationState {
        output,
        canonical_table,
        fallback,
        proof,
        eligibility,
        discarded_account,
    } = state;
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::new(
        fallback,
        evidence.into_unfrozen_owner(),
        canonical_table,
        Some(evidence_table),
        Some(account),
    );
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    drop(discarded_account);
    drop(eligibility);
    cause
}

fn contain_rich_discard_continuation_on_drop<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let _ = contain_rich_discard_continuation_state(
        state,
        AccountPoisonReason::OwnershipGraphUnverified,
        false,
    );
}

fn retained_part_cleanup_reason(cleanup: RetainedPartCleanupError) -> (AccountPoisonReason, bool) {
    match cleanup {
        RetainedPartCleanupError::MissingFaultAnchor
        | RetainedPartCleanupError::DeferredOwnershipFault => {
            (AccountPoisonReason::OwnershipGraphUnverified, false)
        }
        RetainedPartCleanupError::NewFaultAlreadyRecorded(_)
        | RetainedPartCleanupError::StillSharedOwnershipFault(_) => {
            (AccountPoisonReason::OwnershipGraphUnverified, true)
        }
        RetainedPartCleanupError::StorageReleaseUnverified(_) => {
            (AccountPoisonReason::RetainedStorageReleaseUnverified, true)
        }
        RetainedPartCleanupError::ChargeReleaseUnverified(witness) => (witness.reason, true),
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn finish_discard(
        state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Result<FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>, FinalizationContainmentError> {
        let mut state = scopeguard::guard(
            state,
            contain_rich_discard_continuation_on_drop::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        if matches!(&state.proof, RichDiscardContinuationProof::Both(_)) {
            let canonical_table = state
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let outcome =
                drop_retained_table_with_account_anchor(canonical_table, &state.output.account);
            if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
                return Err(contain_rich_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
            let evidence_proof = match &mut state.proof {
                RichDiscardContinuationProof::Both(proof) => {
                    proof.release_canonical_after_physical_cleanup_in_place()
                }
                RichDiscardContinuationProof::Evidence(_) => unreachable_final_owner_state(),
            };
            state.proof = RichDiscardContinuationProof::Evidence(evidence_proof);
            #[cfg(test)]
            maybe_panic_failure_guard_stage(
                FailureTerminalRail::Rich,
                FailureGuardDropStage::CanonicalAcknowledged,
            );
        }

        let discarded_account = match state.fallback.try_discard_allocation() {
            Ok(account) => account,
            Err(cleanup) => {
                let (reason, already_recorded) = retained_part_cleanup_reason(cleanup);
                return Err(contain_rich_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
        };
        state.discarded_account = Some(discarded_account);
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::DiscardCleaned,
        );
        let freeze_ready = match &mut state.proof {
            RichDiscardContinuationProof::Evidence(proof) => {
                proof.release_discard_after_physical_cleanup_in_place()
            }
            RichDiscardContinuationProof::Both(_) => unreachable_final_owner_state(),
        };
        let state = scopeguard::ScopeGuard::into_inner(state);
        let RichDiscardContinuationState {
            output,
            canonical_table,
            fallback,
            proof: _,
            eligibility,
            discarded_account,
        } = state;
        drop(canonical_table);
        drop(fallback);
        drop(discarded_account);
        Ok(FreezeReadyRichGraph {
            state: Some(FreezeReadyRichState {
                output,
                proof: freeze_ready,
                eligibility,
            }),
        })
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let continuation = match state.discard {
            QualifiedRichDiscard::Evidence { fallback, proof } => RichDiscardContinuationState {
                output: state.output,
                canonical_table: None,
                fallback,
                proof: RichDiscardContinuationProof::Evidence(proof),
                eligibility: state.eligibility,
                discarded_account: None,
            },
            QualifiedRichDiscard::Both {
                canonical_table,
                fallback,
                proof,
            } => RichDiscardContinuationState {
                output: state.output,
                canonical_table: Some(canonical_table),
                fallback,
                proof: RichDiscardContinuationProof::Both(proof),
                eligibility: state.eligibility,
                discarded_account: None,
            },
        };
        let freeze_ready = Self::finish_discard(continuation)?;
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        Ok(freeze_ready.freeze())
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleRichGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_eligible_rich_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_freeze_ready_rich_state<O, P, D, F, EvidenceBlock>(
    state: FreezeReadyRichState<O, P, D, F, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyRichState {
        output,
        proof,
        eligibility,
    } = state;
    let QualifiedRichOutput {
        evidence,
        evidence_table,
        account,
        ..
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), EvidenceBlock>::new(
        DetachedRetainedPart::empty(),
        evidence.into_unfrozen_owner(),
        None,
        Some(evidence_table),
        Some(account),
    );
    let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
    drop(eligibility);
}

impl<O, P, D, F, EvidenceBlock> FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze<CanonicalBlock>(
        mut self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let mut state = scopeguard::guard(state, contain_freeze_ready_rich_state);
        let snapshot = state.proof.freeze_in_place();
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::LedgerPublished,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        state
            .evidence_mut()
            .freeze_in_place(terminal_cause, &snapshot);
        let state = scopeguard::ScopeGuard::into_inner(state);
        let FreezeReadyRichState {
            output,
            proof: _,
            eligibility: _,
        } = state;
        let QualifiedRichOutput {
            evidence,
            evidence_table,
            account,
            evidence_root,
            cause: _,
            set_id,
            fallback_type: _,
        } = output;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::RichFailure {
                outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(
                    evidence_table,
                    set_id,
                ),
                outcome_evidence_root: evidence_root,
                account,
            },
            evidence: PublishedPart::from_exact_owner(evidence.into_frozen_owner()),
        }
    }
}

impl<O, P, D, F, EvidenceBlock> FreezeReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn evidence_mut(&mut self) -> &mut FreezeReadyDetachedTerminalEvidence<P, D> {
        &mut self.output.evidence
    }
}

impl<O, P, D, F, EvidenceBlock> Drop for FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_freeze_ready_rich_state(state);
        }
    }
}

pub(super) struct TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    proof: Option<crate::ledger::TypedFreezeProof<O, P, D>>,
    physical: Option<TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>>,
    cause: PreparationFailureCode,
}

struct QualifiedMinimalOutput<P, D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: FreezeReadyDetachedTerminalEvidence<P, D>,
    cause: PreparationFailureCode,
}

enum QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    None {
        proof: FreezeReadyTypedProof<O, P, D>,
    },
    Evidence {
        table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        proof: QualifiedEvidenceDiscardFreezeProof<O, P, D>,
    },
    Both {
        canonical_table: RetainedTableAllocation<CanonicalBlock>,
        evidence_table: RetainedTableAllocation<EvidenceBlock>,
        account: PhysicalStorageAccount,
        proof: QualifiedBothDiscardFreezeProof<O, P, D>,
    },
}

struct QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
}

struct QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
    eligibility: PublicationEligibility,
}

struct EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>>,
}

struct FreezeReadyMinimalState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    proof: FreezeReadyTypedProof<O, P, D>,
    eligibility: PublicationEligibility,
}

struct FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyMinimalState<O, P, D, F>>,
}

#[allow(
    clippy::type_complexity,
    reason = "the tuple moves one exact physical graph beside its matching typed ledger proof"
)]
fn minimal_state_into_terminal<P, D, F, CanonicalBlock, EvidenceBlock, O>(
    output: QualifiedMinimalOutput<P, D, F>,
    discard: QualifiedMinimalDiscard<O, P, D, CanonicalBlock, EvidenceBlock>,
) -> (
    TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
    QualifiedTypedDiscardFreezeProof<O, P, D>,
)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    match discard {
        QualifiedMinimalDiscard::None { proof } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                None,
                None,
            ),
            QualifiedTypedDiscardFreezeProof::None(proof),
        ),
        QualifiedMinimalDiscard::Evidence {
            table,
            account,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                None,
                Some(table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Evidence(proof),
        ),
        QualifiedMinimalDiscard::Both {
            canonical_table,
            evidence_table,
            account,
            proof,
        } => (
            TerminalPhysicalOwnershipManifest::new(
                fallback,
                evidence.into_unfrozen_owner(),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ),
            QualifiedTypedDiscardFreezeProof::Both(proof),
        ),
    }
}

fn contain_qualified_minimal_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: QualifiedMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let (physical, proof) = minimal_state_into_terminal(state.output, state.discard);
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    cause
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    pub(super) fn new(
        proof: crate::ledger::TypedFreezeProof<O, P, D>,
        physical: TerminalPhysicalOwnershipManifest<P, D, F, CanonicalBlock, EvidenceBlock>,
        cause: PreparationFailureCode,
    ) -> Self {
        Self {
            proof: Some(proof),
            physical: Some(physical),
            cause,
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
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let proof = self
            .proof
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let proof = match proof.try_into_qualified_discards() {
            Ok(proof) => proof,
            Err(proof) => {
                self.proof = Some(proof);
                return Err(
                    self.contain_unqualified(AccountPoisonReason::TerminalEvidenceFreezeUnverified)
                );
            }
        };
        if matches!(&proof, QualifiedTypedDiscardFreezeProof::Canonical(_)) {
            let physical = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let cause = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            proof.retain_fail_closed();
            return Err(cause);
        }
        let (expect_canonical, expect_evidence) = match &proof {
            QualifiedTypedDiscardFreezeProof::None(_) => (false, false),
            QualifiedTypedDiscardFreezeProof::Evidence(_) => (false, true),
            QualifiedTypedDiscardFreezeProof::Both(_) => (true, true),
            QualifiedTypedDiscardFreezeProof::Canonical(_) => unreachable_final_owner_state(),
        };
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .qualifies_pre_cleanup_fallback(expect_canonical, expect_evidence)
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
        let qualified_evidence =
            match QualifiedDetachedTerminalEvidence::try_from_discard_proof(evidence, &proof) {
                DetachedTerminalEvidenceQualification::Qualified(evidence) => {
                    evidence.into_freeze_ready()
                }
                DetachedTerminalEvidenceQualification::Rejected(evidence)
                | DetachedTerminalEvidenceQualification::Panicked(evidence) => {
                    let physical = TerminalPhysicalOwnershipManifest::new(
                        fallback,
                        evidence,
                        canonical_table,
                        evidence_table,
                        account,
                    );
                    let cause = physical
                        .contain_new_fault(AccountPoisonReason::TerminalEvidenceFreezeUnverified);
                    proof.retain_fail_closed();
                    return Err(cause);
                }
            };
        let output = QualifiedMinimalOutput {
            fallback,
            evidence: qualified_evidence,
            cause: self.cause,
        };
        let discard = match (proof, canonical_table, evidence_table, account) {
            (QualifiedTypedDiscardFreezeProof::None(proof), None, None, None) => {
                QualifiedMinimalDiscard::None { proof }
            }
            (
                QualifiedTypedDiscardFreezeProof::Evidence(proof),
                None,
                Some(table),
                Some(account),
            ) => QualifiedMinimalDiscard::Evidence {
                table,
                account,
                proof,
            },
            (
                QualifiedTypedDiscardFreezeProof::Both(proof),
                Some(canonical_table),
                Some(evidence_table),
                Some(account),
            ) => QualifiedMinimalDiscard::Both {
                canonical_table,
                evidence_table,
                account,
                proof,
            },
            _ => unreachable_final_owner_state(),
        };
        let qualified = QualifiedMinimalGraph {
            state: Some(QualifiedMinimalState { output, discard }),
        };
        #[cfg(test)]
        let qualified = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::Qualified,
            qualified,
        );
        qualified.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalMinimalFreezeGuard<O, P, D, F, CanonicalBlock, EvidenceBlock>
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

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let eligibility = match state
            .output
            .evidence
            .owner
            .account()
            .try_publication_eligibility()
        {
            Ok(eligibility) => eligibility,
            Err(quarantine) => {
                let reason = quarantine.snapshot().first_poison_evidence().map_or(
                    AccountPoisonReason::OwnershipGraphUnverified,
                    AccountPoisonEvidence::reason,
                );
                return Err(contain_qualified_minimal_state(state, reason, true));
            }
        };
        let eligible = EligibleMinimalGraph {
            state: Some(EligibleMinimalState {
                output: state.output,
                discard: state.discard,
                eligibility,
            }),
        };
        #[cfg(test)]
        let eligible = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::Eligible,
            eligible,
        );
        eligible.into_prepared()
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_qualified_minimal_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_eligible_minimal_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: EligibleMinimalState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let EligibleMinimalState {
        output,
        discard,
        eligibility,
    } = state;
    let cause = contain_qualified_minimal_state(
        QualifiedMinimalState { output, discard },
        reason,
        already_recorded,
    );
    drop(eligibility);
    cause
}

enum MinimalDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    Both(QualifiedBothDiscardFreezeProof<O, P, D>),
    Evidence(QualifiedEvidenceDiscardFreezeProof<O, P, D>),
}

impl<O, P, D> MinimalDiscardContinuationProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn retain_fail_closed(self) {
        match self {
            Self::Both(proof) => proof.retain_fail_closed(),
            Self::Evidence(proof) => proof.retain_fail_closed(),
        }
    }
}

struct MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    account: PhysicalStorageAccount,
    proof: MinimalDiscardContinuationProof<O, P, D>,
    eligibility: PublicationEligibility,
}

fn contain_minimal_discard_continuation_state<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    reason: AccountPoisonReason,
    already_recorded: bool,
) -> FinalizationContainmentError
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let MinimalDiscardContinuationState {
        output,
        canonical_table,
        evidence_table,
        account,
        proof,
        eligibility,
    } = state;
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::new(
        fallback,
        evidence.into_unfrozen_owner(),
        canonical_table,
        evidence_table,
        Some(account),
    );
    let cause = if already_recorded {
        physical.contain_after_recorded(reason)
    } else {
        physical.contain_new_fault(reason)
    };
    proof.retain_fail_closed();
    drop(eligibility);
    cause
}

fn contain_minimal_discard_continuation_on_drop<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let _ = contain_minimal_discard_continuation_state(
        state,
        AccountPoisonReason::OwnershipGraphUnverified,
        false,
    );
}

fn table_cleanup_reason(
    outcome: RetainedStorageDropOutcome,
) -> Option<(AccountPoisonReason, bool)> {
    match outcome {
        RetainedStorageDropOutcome::Clean => None,
        RetainedStorageDropOutcome::MissingFaultAnchor => {
            Some((AccountPoisonReason::OwnershipGraphUnverified, false))
        }
        RetainedStorageDropOutcome::StorageReleaseUnverified(witness)
        | RetainedStorageDropOutcome::ChargeReleaseUnverified(witness)
        | RetainedStorageDropOutcome::StillSharedOwnershipFault(witness)
        | RetainedStorageDropOutcome::ForeignOwnershipFault(witness) => {
            Some((witness.reason, true))
        }
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock>
    EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
    fn finish_discard(
        state: MinimalDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
    ) -> Result<FreezeReadyMinimalGraph<O, P, D, F>, FinalizationContainmentError> {
        let mut state = scopeguard::guard(
            state,
            contain_minimal_discard_continuation_on_drop::<O, P, D, F, CanonicalBlock, EvidenceBlock>,
        );
        if matches!(&state.proof, MinimalDiscardContinuationProof::Both(_)) {
            let canonical_table = state
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state());
            let outcome = drop_retained_table_with_account_anchor(canonical_table, &state.account);
            if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
                return Err(contain_minimal_discard_continuation_state(
                    scopeguard::ScopeGuard::into_inner(state),
                    reason,
                    already_recorded,
                ));
            }
            let evidence_proof = match &mut state.proof {
                MinimalDiscardContinuationProof::Both(proof) => {
                    proof.release_canonical_after_physical_cleanup_in_place()
                }
                MinimalDiscardContinuationProof::Evidence(_) => unreachable_final_owner_state(),
            };
            state.proof = MinimalDiscardContinuationProof::Evidence(evidence_proof);
            #[cfg(test)]
            maybe_panic_failure_guard_stage(
                FailureTerminalRail::Minimal,
                FailureGuardDropStage::CanonicalAcknowledged,
            );
        }

        let evidence_table = state
            .evidence_table
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let outcome = drop_retained_table_with_account_anchor(evidence_table, &state.account);
        if let Some((reason, already_recorded)) = table_cleanup_reason(outcome) {
            return Err(contain_minimal_discard_continuation_state(
                scopeguard::ScopeGuard::into_inner(state),
                reason,
                already_recorded,
            ));
        }
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::DiscardCleaned,
        );
        let freeze_ready = match &mut state.proof {
            MinimalDiscardContinuationProof::Evidence(proof) => {
                proof.release_discard_after_physical_cleanup_in_place()
            }
            MinimalDiscardContinuationProof::Both(_) => unreachable_final_owner_state(),
        };
        let state = scopeguard::ScopeGuard::into_inner(state);
        let MinimalDiscardContinuationState {
            output,
            canonical_table,
            evidence_table,
            account,
            proof: _,
            eligibility,
        } = state;
        drop(canonical_table);
        drop(evidence_table);
        drop(account);
        Ok(FreezeReadyMinimalGraph {
            state: Some(FreezeReadyMinimalState {
                output,
                proof: freeze_ready,
                eligibility,
            }),
        })
    }

    fn into_prepared(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let freeze_ready = match state.discard {
            QualifiedMinimalDiscard::None { proof } => FreezeReadyMinimalGraph {
                state: Some(FreezeReadyMinimalState {
                    output: state.output,
                    proof,
                    eligibility: state.eligibility,
                }),
            },
            QualifiedMinimalDiscard::Evidence {
                table,
                account,
                proof,
            } => Self::finish_discard(MinimalDiscardContinuationState {
                output: state.output,
                canonical_table: None,
                evidence_table: Some(table),
                account,
                proof: MinimalDiscardContinuationProof::Evidence(proof),
                eligibility: state.eligibility,
            })?,
            QualifiedMinimalDiscard::Both {
                canonical_table,
                evidence_table,
                account,
                proof,
            } => Self::finish_discard(MinimalDiscardContinuationState {
                output: state.output,
                canonical_table: Some(canonical_table),
                evidence_table: Some(evidence_table),
                account,
                proof: MinimalDiscardContinuationProof::Both(proof),
                eligibility: state.eligibility,
            })?,
        };
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        Ok(freeze_ready.freeze())
    }
}

impl<O, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for EligibleMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = contain_eligible_minimal_state(
                state,
                AccountPoisonReason::OwnershipGraphUnverified,
                false,
            );
        }
    }
}

fn contain_freeze_ready_minimal_state<O, P, D, F>(state: FreezeReadyMinimalState<O, P, D, F>)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyMinimalState {
        output,
        proof,
        eligibility,
    } = state;
    let QualifiedMinimalOutput {
        fallback,
        evidence,
        cause: _,
    } = output;
    let physical = TerminalPhysicalOwnershipManifest::<P, D, F, (), ()>::new(
        fallback,
        evidence.into_unfrozen_owner(),
        None,
        None,
        None,
    );
    let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
    proof.retain_fail_closed();
    drop(eligibility);
}

impl<O, P, D, F> FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn freeze<CanonicalBlock, EvidenceBlock>(
        mut self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let mut state = scopeguard::guard(state, contain_freeze_ready_minimal_state);
        let snapshot = state.proof.freeze_in_place();
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::LedgerPublished,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        state
            .output
            .evidence
            .freeze_in_place(terminal_cause, &snapshot);
        let state = scopeguard::ScopeGuard::into_inner(state);
        let FreezeReadyMinimalState {
            output,
            proof: _,
            eligibility: _,
        } = state;
        let QualifiedMinimalOutput {
            fallback,
            evidence,
            cause: _,
        } = output;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::MinimalFallback {
                part: PublishedPart::from_exact_owner(fallback),
            },
            evidence: PublishedPart::from_exact_owner(evidence.into_frozen_owner()),
        }
    }
}

impl<O, P, D, F> Drop for FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            contain_freeze_ready_minimal_state(state);
        }
    }
}
