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

struct FreezeReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    proof: FreezeReadyTypedProof<O, P, D>,
}

struct FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyRichState<O, P, D, F, EvidenceBlock>>,
}

struct DetachedReadyRichState<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedRichOutput<P, D, F, EvidenceBlock>,
    proof: crate::ledger::DetachedTypedFreezeProof<O, P, D>,
}

struct EligibilityReadyRichPhysical<P, D, F, EvidenceBlock> {
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
    evidence_table: RetainedTableAllocation<EvidenceBlock>,
    account: PhysicalStorageAccount,
    evidence_root: RetainedSlotIdentity,
    set_id: RetainedSetId,
    fallback_type: PhantomData<fn() -> F>,
}

type EligibilityReadyRichJointCommit<O, P, D, F, EvidenceBlock> =
    AccountBoundJointCommit<EligibilityReadyRichPhysical<P, D, F, EvidenceBlock>, O, P, D>;

struct EligibilityReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibilityReadyRichJointCommit<O, P, D, F, EvidenceBlock>>,
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
        let continuation = match state.discard {
            QualifiedRichDiscard::Evidence { fallback, proof } => RichDiscardContinuationState {
                output: state.output,
                canonical_table: None,
                fallback,
                proof: RichDiscardContinuationProof::Evidence(proof),
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
                discarded_account: None,
            },
        };
        let freeze_ready = finish_rich_discard(continuation)?;
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        let eligibility_ready = freeze_ready.try_detach_and_freeze()?;
        #[cfg(test)]
        let eligibility_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::EligibilityReady,
            eligibility_ready,
        );
        eligibility_ready.try_into_prepared()
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

fn finish_rich_discard<O, P, D, F, CanonicalBlock, EvidenceBlock>(
    state: RichDiscardContinuationState<O, P, D, F, CanonicalBlock, EvidenceBlock>,
) -> Result<FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>, FinalizationContainmentError>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
{
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
        discarded_account,
    } = state;
    drop(canonical_table);
    drop(fallback);
    drop(discarded_account);
    Ok(FreezeReadyRichGraph {
        state: Some(FreezeReadyRichState {
            output,
            proof: freeze_ready,
        }),
    })
}

fn contain_freeze_ready_rich_state<O, P, D, F, EvidenceBlock>(
    state: FreezeReadyRichState<O, P, D, F, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyRichState { output, proof } = state;
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
}

impl<O, P, D, F, EvidenceBlock> FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_detach_and_freeze(
        mut self,
    ) -> Result<EligibilityReadyRichGraph<O, P, D, F, EvidenceBlock>, FinalizationContainmentError>
    {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let FreezeReadyRichState { output, proof } = state;
        let proof = match proof.try_into_detached() {
            Ok(proof) => proof,
            Err(error) => {
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
                let cause =
                    physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                error.retain_fail_closed();
                return Err(cause);
            }
        };
        let state = DetachedReadyRichState { output, proof };
        let mut state = scopeguard::guard(state, contain_detached_ready_rich_state);
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Rich,
            FailureGuardDropStage::LedgerDetached,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        let DetachedReadyRichState { output, proof } = &mut *state;
        output
            .evidence
            .freeze_in_place(terminal_cause, proof.terminal_projection());
        let state = scopeguard::ScopeGuard::into_inner(state);
        let DetachedReadyRichState { output, proof } = state;
        let QualifiedRichOutput {
            evidence,
            evidence_table,
            account,
            evidence_root,
            cause: _,
            set_id,
            fallback_type: _,
        } = output;
        Ok(EligibilityReadyRichGraph {
            state: Some(AccountBoundJointCommit::new(
                EligibilityReadyRichPhysical {
                    evidence: evidence.into_frozen_owner(),
                    evidence_table,
                    account,
                    evidence_root,
                    set_id,
                    fallback_type: PhantomData,
                },
                proof,
            )),
        })
    }
}

fn contain_detached_ready_rich_state<O, P, D, F, EvidenceBlock>(
    state: DetachedReadyRichState<O, P, D, F, EvidenceBlock>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let DetachedReadyRichState { output, proof } = state;
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
}

impl<O, P, D, F, EvidenceBlock> EligibilityReadyRichGraph<O, P, D, F, EvidenceBlock>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_into_prepared<CanonicalBlock>(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        Ok(state.try_commit()?.into_prepared())
    }
}

impl<P, D, F, EvidenceBlock> EligibilityReadyRichPhysical<P, D, F, EvidenceBlock> {
    fn into_terminal(self) -> TerminalPhysicalOwnershipManifest<P, D, F, (), EvidenceBlock> {
        let Self {
            evidence,
            evidence_table,
            account,
            evidence_root: _,
            set_id: _,
            fallback_type: _,
        } = self;
        TerminalPhysicalOwnershipManifest::new(
            DetachedRetainedPart::empty(),
            evidence,
            None,
            Some(evidence_table),
            Some(account),
        )
    }

    fn into_prepared<CanonicalBlock>(
        self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let Self {
            evidence,
            evidence_table,
            account,
            evidence_root,
            set_id,
            fallback_type: _,
        } = self;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::RichFailure {
                outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(
                    evidence_table,
                    set_id,
                ),
                outcome_evidence_root: evidence_root,
                account,
            },
            evidence: PublishedPart::from_exact_owner(evidence),
        }
    }
}

impl<P, D, F, EvidenceBlock> AccountBoundPublicationManifest
    for EligibilityReadyRichPhysical<P, D, F, EvidenceBlock>
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

impl<O, P, D, F, EvidenceBlock> Drop for EligibilityReadyRichGraph<O, P, D, F, EvidenceBlock>
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

struct FreezeReadyMinimalState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    proof: FreezeReadyTypedProof<O, P, D>,
}

struct FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<FreezeReadyMinimalState<O, P, D, F>>,
}

struct DetachedReadyMinimalState<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    output: QualifiedMinimalOutput<P, D, F>,
    proof: crate::ledger::DetachedTypedFreezeProof<O, P, D>,
}

struct EligibilityReadyMinimalPhysical<P, D, F> {
    fallback: DetachedRetainedPart<F>,
    evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
}

type EligibilityReadyMinimalJointCommit<O, P, D, F> =
    AccountBoundJointCommit<EligibilityReadyMinimalPhysical<P, D, F>, O, P, D>;

struct EligibilityReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    state: Option<EligibilityReadyMinimalJointCommit<O, P, D, F>>,
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
        let freeze_ready = match state.discard {
            QualifiedMinimalDiscard::None { proof } => FreezeReadyMinimalGraph {
                state: Some(FreezeReadyMinimalState {
                    output: state.output,
                    proof,
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
            })?,
        };
        #[cfg(test)]
        let freeze_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::FreezeReady,
            freeze_ready,
        );
        let eligibility_ready = freeze_ready.try_detach_and_freeze()?;
        #[cfg(test)]
        let eligibility_ready = maybe_drop_failure_guard(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::EligibilityReady,
            eligibility_ready,
        );
        eligibility_ready.try_into_prepared()
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
    QualifiedMinimalGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>
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
        } = state;
        drop(canonical_table);
        drop(evidence_table);
        drop(account);
        Ok(FreezeReadyMinimalGraph {
            state: Some(FreezeReadyMinimalState {
                output,
                proof: freeze_ready,
            }),
        })
    }
}

fn contain_freeze_ready_minimal_state<O, P, D, F>(state: FreezeReadyMinimalState<O, P, D, F>)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let FreezeReadyMinimalState { output, proof } = state;
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
}

impl<O, P, D, F> FreezeReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_detach_and_freeze(
        mut self,
    ) -> Result<EligibilityReadyMinimalGraph<O, P, D, F>, FinalizationContainmentError> {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let FreezeReadyMinimalState { output, proof } = state;
        let proof = match proof.try_into_detached() {
            Ok(proof) => proof,
            Err(error) => {
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
                let cause =
                    physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
                error.retain_fail_closed();
                return Err(cause);
            }
        };
        let state = DetachedReadyMinimalState { output, proof };
        let mut state = scopeguard::guard(state, contain_detached_ready_minimal_state);
        #[cfg(test)]
        maybe_panic_failure_guard_stage(
            FailureTerminalRail::Minimal,
            FailureGuardDropStage::LedgerDetached,
        );
        let terminal_cause = TerminalEvidenceCause::Failure(state.output.cause);
        let DetachedReadyMinimalState { output, proof } = &mut *state;
        output
            .evidence
            .freeze_in_place(terminal_cause, proof.terminal_projection());
        let state = scopeguard::ScopeGuard::into_inner(state);
        let DetachedReadyMinimalState { output, proof } = state;
        let QualifiedMinimalOutput {
            fallback,
            evidence,
            cause: _,
        } = output;
        Ok(EligibilityReadyMinimalGraph {
            state: Some(AccountBoundJointCommit::new(
                EligibilityReadyMinimalPhysical {
                    fallback,
                    evidence: evidence.into_frozen_owner(),
                },
                proof,
            )),
        })
    }
}

fn contain_detached_ready_minimal_state<O, P, D, F>(state: DetachedReadyMinimalState<O, P, D, F>)
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    let DetachedReadyMinimalState { output, proof } = state;
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
}

impl<O, P, D, F> EligibilityReadyMinimalGraph<O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn try_into_prepared<CanonicalBlock, EvidenceBlock>(
        mut self,
    ) -> Result<
        PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
        FinalizationContainmentError,
    > {
        let state = self
            .state
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        Ok(state.try_commit()?.into_prepared())
    }
}

impl<P, D, F> EligibilityReadyMinimalPhysical<P, D, F> {
    fn into_terminal(self) -> TerminalPhysicalOwnershipManifest<P, D, F, (), ()> {
        let Self { fallback, evidence } = self;
        TerminalPhysicalOwnershipManifest::new(fallback, evidence, None, None, None)
    }

    fn into_prepared<CanonicalBlock, EvidenceBlock>(
        self,
    ) -> PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
        let Self { fallback, evidence } = self;
        PreparedRetainedPublication {
            payload: RetainedPublicationPayload::MinimalFallback {
                part: PublishedPart::from_exact_owner(fallback),
            },
            evidence: PublishedPart::from_exact_owner(evidence),
        }
    }
}

impl<P, D, F> AccountBoundPublicationManifest for EligibilityReadyMinimalPhysical<P, D, F> {
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

impl<O, P, D, F> Drop for EligibilityReadyMinimalGraph<O, P, D, F>
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
