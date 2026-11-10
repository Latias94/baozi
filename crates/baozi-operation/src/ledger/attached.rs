//! Concrete terminal extraction after the root has finished its borrowed qualification.

use super::snapshot::take_typed_terminal_projection;
use super::storage::SharedStorage;
use super::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn arm_partial_detach_panic_for_test() {
    tests::arm_partial_detach_panic();
}

#[cfg(test)]
pub(crate) fn arm_commit_panic_for_test() {
    tests::arm_commit_panic();
}

#[cfg(test)]
pub(super) fn panic_after_commitments_for_test() {
    tests::panic_after_commitments();
}

/// Selects the actual source arrangement used by a completed borrowed proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedTerminalLayout {
    Primary,
    Fallback,
}

/// Presence of the original deferred batch fields, without owning any publication authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TypedTerminalRoles {
    pub(crate) deferred_canonical: bool,
    pub(crate) deferred_evidence_first: bool,
    pub(crate) deferred_evidence_second: bool,
}

/// Short loans of the exact batches selected by the root's completed terminal decision.
pub(crate) struct TypedTerminalBatches<'loan, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) canonical: &'loan mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
    pub(crate) evidence:
        &'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) deferred_canonical_storage:
        Option<&'loan mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    pub(crate) deferred_evidence_storage_first:
        Option<&'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    pub(crate) deferred_evidence_storage_second:
        Option<&'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    pub(crate) discard_canonical:
        Option<&'loan mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    pub(crate) discard_evidence:
        Option<&'loan mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
}

pub(super) struct DetachedTypedBatchSlots<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical: Option<DetachedAttributionBatch<O, P, D, CanonicalOutput>>,
    evidence: Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
    deferred_canonical_storage: Option<DetachedAttributionBatch<O, P, D, CanonicalOutput>>,
    deferred_evidence_storage_first:
        Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
    deferred_evidence_storage_second:
        Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
    discard_canonical: Option<DetachedAttributionBatch<O, P, D, CanonicalOutput>>,
    discard_evidence: Option<DetachedAttributionBatch<O, P, D, OutcomeEvidenceDestination>>,
}

impl<O, P, D> DetachedTypedBatchSlots<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn new() -> Self {
        Self {
            canonical: None,
            evidence: None,
            deferred_canonical_storage: None,
            deferred_evidence_storage_first: None,
            deferred_evidence_storage_second: None,
            discard_canonical: None,
            discard_evidence: None,
        }
    }

    pub(super) fn detach_in_place(
        &mut self,
        batches: TypedTerminalBatches<'_, O, P, D>,
        expected: Option<&SharedStorage<P, D>>,
        identity: LedgerIdentity,
    ) {
        self.canonical = Some(batches.canonical.detach_in_place(expected, identity));
        #[cfg(test)]
        tests::panic_after_canonical_detach();
        self.evidence = Some(batches.evidence.detach_in_place(expected, identity));
        self.deferred_canonical_storage = batches
            .deferred_canonical_storage
            .map(|batch| batch.detach_in_place(expected, identity));
        self.deferred_evidence_storage_first = batches
            .deferred_evidence_storage_first
            .map(|batch| batch.detach_in_place(expected, identity));
        self.deferred_evidence_storage_second = batches
            .deferred_evidence_storage_second
            .map(|batch| batch.detach_in_place(expected, identity));
        self.discard_canonical = batches
            .discard_canonical
            .map(|batch| batch.detach_in_place(expected, identity));
        self.discard_evidence = batches
            .discard_evidence
            .map(|batch| batch.detach_in_place(expected, identity));
    }

    pub(super) fn take_batches(&mut self) -> DetachedTypedBatches<O, P, D> {
        // Check both required slots before transferring either; a failed invariant retains all
        // already-detached backing rather than dropping a partially assembled local product.
        assert!(self.canonical.is_some() && self.evidence.is_some());
        DetachedTypedBatches {
            canonical: self.canonical.take().expect("canonical batch was detached"),
            evidence: self.evidence.take().expect("evidence batch was detached"),
            deferred_canonical_storage: self.deferred_canonical_storage.take(),
            deferred_evidence_storage_first: self.deferred_evidence_storage_first.take(),
            deferred_evidence_storage_second: self.deferred_evidence_storage_second.take(),
            discard_canonical: self.discard_canonical.take(),
            discard_evidence: self.discard_evidence.take(),
        }
    }
}

impl<O, P, D> Drop for DetachedTypedBatchSlots<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        std::mem::forget(self.canonical.take());
        std::mem::forget(self.evidence.take());
        std::mem::forget(self.deferred_canonical_storage.take());
        std::mem::forget(self.deferred_evidence_storage_first.take());
        std::mem::forget(self.deferred_evidence_storage_second.take());
        std::mem::forget(self.discard_canonical.take());
        std::mem::forget(self.discard_evidence.take());
    }
}

pub(super) enum SharedStorageRecoveryError {
    SharedOwner,
    Poisoned,
}

/// The original shared shell or recovered state stays armed through every invariant check.
pub(super) struct SharedStorageRecovery<P, D> {
    shared: Option<SharedStorage<P, D>>,
    state: Option<LedgerState<P, D>>,
}

impl<P, D> SharedStorageRecovery<P, D> {
    pub(super) fn new(shared: SharedStorage<P, D>) -> Self {
        Self {
            shared: Some(shared),
            state: None,
        }
    }

    pub(super) fn recover(&mut self) -> Result<(), SharedStorageRecoveryError> {
        assert!(self.state.is_none(), "shared storage is recovered once");
        let shared = self
            .shared
            .take()
            .expect("recovery retains its original shared shell");
        let poisoned = match Arc::try_unwrap(shared) {
            Ok(shared) => match shared.into_inner() {
                Ok(state) => {
                    self.state = state;
                    false
                }
                Err(poisoned) => {
                    self.state = poisoned.into_inner();
                    true
                }
            },
            Err(shared) => {
                self.shared = Some(shared);
                return Err(SharedStorageRecoveryError::SharedOwner);
            }
        };
        // Install the actual state before assertions or accounting can unwind.
        let state = self
            .state
            .as_mut()
            .expect("detached shared storage retains its actual state");
        release_evidence(state, size_of::<Mutex<Option<LedgerState<P, D>>>>() as u64);
        if poisoned {
            Err(SharedStorageRecoveryError::Poisoned)
        } else {
            Ok(())
        }
    }

    pub(super) fn state_mut(&mut self) -> &mut LedgerState<P, D> {
        self.state
            .as_mut()
            .expect("exclusive storage was recovered")
    }

    pub(super) fn state(&self) -> Option<&LedgerState<P, D>> {
        self.state.as_ref()
    }

    pub(super) fn take_state(&mut self) -> Option<LedgerState<P, D>> {
        self.state.take()
    }

    pub(super) fn take_shared(&mut self) -> Option<SharedStorage<P, D>> {
        self.shared.take()
    }
}

impl<P, D> Drop for SharedStorageRecovery<P, D> {
    fn drop(&mut self) {
        std::mem::forget(self.shared.take());
        std::mem::forget(self.state.take());
    }
}

/// Every field moved out of a root stays armed until the whole numerical graph is transferred.
/// A source batch still being detached remains in its caller's actual root on unwind.
struct TypedDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    invocation: Option<OperationInvocation<O, P, PP, U, D>>,
    projection: Option<LedgerSnapshot<P, D>>,
    storage: Option<SharedStorageRecovery<P, D>>,
    batches: DetachedTypedBatchSlots<O, P, D>,
}

impl<O, P, PP, U, D> TypedDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn new(invocation: OperationInvocation<O, P, PP, U, D>) -> Self {
        Self {
            invocation: Some(invocation),
            projection: None,
            storage: None,
            batches: DetachedTypedBatchSlots::new(),
        }
    }

    fn into_error(
        mut self,
        kind: TypedFreezeDetachErrorKind,
    ) -> TypedFreezeDetachError<O, P, PP, U, D> {
        assert!(self.projection.is_some() && self.invocation.is_some());
        let batches = self.batches.take_batches();
        TypedFreezeDetachError::new(
            kind,
            self.projection
                .take()
                .expect("terminal projection was captured"),
            self.invocation
                .take()
                .expect("actual invocation remains installed"),
            batches,
        )
    }
}

impl<O, P, PP, U, D> Drop for TypedDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        std::mem::forget(self.projection.take());
        std::mem::forget(self.invocation.take());
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
    #[cfg(test)]
    pub(crate) fn test_panic_after_next_canonical_detach(&mut self) {
        tests::arm_partial_detach_panic();
    }

    #[expect(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "detach failure retains the actual invocation and every reservation without failure-path allocation"
    )]
    pub(crate) fn try_detach_typed(
        self,
        last_completed_phase: Option<P>,
        batches: TypedTerminalBatches<'_, O, P, D>,
    ) -> Result<DetachedTypedFreezeProof<O, P, D>, TypedFreezeDetachError<O, P, PP, U, D>> {
        let mut guard = TypedDetachGuard::new(self);
        // Discard acknowledgement has already retired its actual backing and bound cover.
        let known_administrative_bytes = guard
            .invocation
            .as_ref()
            .expect("actual invocation remains installed")
            .known_administrative_bytes();
        let raw = guard
            .invocation
            .as_mut()
            .expect("actual invocation remains installed")
            .terminal_ledger_mut();
        let identity = raw.identity;
        let shared = raw
            .state
            .shared()
            .expect("typed reservations retain the installed shared identity");
        guard.projection = Some({
            let mut storage = lock(shared);
            take_typed_terminal_projection(
                &mut storage,
                &raw.work,
                last_completed_phase,
                batches.canonical,
                batches.evidence,
            )
        });
        guard
            .projection
            .as_mut()
            .expect("terminal projection was captured")
            .qualify_administrative_coverage(known_administrative_bytes);

        let unexpected_discard =
            batches.discard_canonical.is_some() || batches.discard_evidence.is_some();
        guard
            .batches
            .detach_in_place(batches, Some(shared), identity);

        if unexpected_discard {
            return Err(guard.into_error(TypedFreezeDetachErrorKind::UnexpectedDiscardOwner));
        }
        if shared.is_poisoned() {
            return Err(guard.into_error(TypedFreezeDetachErrorKind::PoisonedLedger));
        }

        let selected = std::mem::replace(&mut raw.state, DynamicStorage::Inline(None));
        let shared = match selected {
            DynamicStorage::Shared(shared) => shared,
            DynamicStorage::Inline(state) => {
                raw.state = DynamicStorage::Inline(state);
                unreachable!("typed reservations require shared storage")
            }
        };
        guard.storage = Some(SharedStorageRecovery::new(shared));
        let storage = guard
            .storage
            .as_mut()
            .expect("the actual shared owner stays guarded");
        match storage.recover() {
            Ok(()) => {}
            Err(SharedStorageRecoveryError::Poisoned) => {
                raw.state = DynamicStorage::Inline(storage.take_state());
                return Err(guard.into_error(TypedFreezeDetachErrorKind::PoisonedLedger));
            }
            Err(SharedStorageRecoveryError::SharedOwner) => {
                raw.state = DynamicStorage::Shared(
                    storage
                        .take_shared()
                        .expect("refused recovery retains its original shared shell"),
                );
                return Err(guard.into_error(TypedFreezeDetachErrorKind::SharedLedgerOwner));
            }
        }
        let state = storage.state_mut();
        guard
            .invocation
            .take()
            .expect("actual invocation remains installed")
            .retire_after_detach(state);
        let batches = guard.batches.take_batches();
        Ok(DetachedTypedFreezeProof {
            projection: guard.projection.take(),
            cover: Some(DetachedLiveLedgerCover {
                state: storage
                    .take_state()
                    .expect("exclusive storage was detached"),
                batches,
            }),
        })
    }
}

impl<O, P, PP, U, D> FreezeReadyTypedProof<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    #[allow(
        clippy::result_large_err,
        reason = "seal refusal returns the same borrowed proof without allocation or moving an owner"
    )]
    pub(crate) fn seal_to_root(mut self) -> Result<crate::retained::AttachedTerminalStamp, Self> {
        let Some(qualified) = self.proof.as_mut() else {
            return Err(self);
        };
        let Some(proof) = qualified.proof.as_mut() else {
            return Err(self);
        };
        if proof.discard_canonical.is_some() || proof.discard_evidence.is_some() {
            return Err(self);
        }
        let decision = crate::retained::AttachedTerminalDecision {
            last_completed_phase: qualified.last_completed_phase,
            layout: proof.layout,
            roles: TypedTerminalRoles {
                deferred_canonical: proof.deferred_canonical_storage.is_some(),
                deferred_evidence_first: proof.deferred_evidence_storage_first.is_some(),
                deferred_evidence_second: proof.deferred_evidence_storage_second.is_some(),
            },
        };
        let stamp = match proof.ledger.try_seal_attached(decision) {
            Ok(stamp) => stamp,
            Err(_) => return Err(self),
        };

        // The actual root remains outstanding. Only these nested loan wrappers are disarmed;
        // dropping an armed qualified wrapper here would incorrectly request containment.
        let _ = qualified.proof.take();
        drop(self.proof.take());
        Ok(stamp)
    }
}
