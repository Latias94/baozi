use super::evidence::StorageCandidate;
use super::storage::{DynamicStorage, StorageOwner, StorageView};
use super::{
    AttributionReservationBatch, Commitment, CounterClass, DestinationKind, DestinationState,
    DimensionEntry, LedgerCause, LedgerEvidenceError, LedgerEvidencePurpose,
    PublicationDestination, release_evidence,
};
use crate::OperationOwner;

pub(super) enum AggregateCommitmentError<D> {
    Ledger(LedgerCause<D>),
    Evidence(LedgerEvidenceError),
}

pub(super) struct CommitmentAggregate<'a, P, D> {
    values: Vec<AggregatedCommitment<D>>,
    admission: CommitmentEvidenceAdmission<'a, P, D>,
}

struct CommitmentEvidenceAdmission<'a, P, D> {
    storage: &'a mut DynamicStorage<P, D>,
    bytes: u64,
}

impl<P, D> Drop for CommitmentEvidenceAdmission<'_, P, D> {
    fn drop(&mut self) {
        if self.bytes != 0 {
            release_evidence(&mut self.storage.write(), self.bytes);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AggregatedCommitment<D> {
    pub(super) dimension: D,
    pub(super) dimension_position: usize,
    pub(super) amount: u64,
}

impl<P, D> CommitmentAggregate<'_, P, D> {
    pub(super) fn parts(&mut self) -> (&mut DynamicStorage<P, D>, &[AggregatedCommitment<D>]) {
        (self.admission.storage, &self.values)
    }

    /// Transfers backing and its charge together to the pending publication owner.
    pub(super) fn into_parts(mut self) -> (Vec<AggregatedCommitment<D>>, u64) {
        (
            std::mem::take(&mut self.values),
            std::mem::take(&mut self.admission.bytes),
        )
    }
}

/// Builds the positioned batch before the caller acquires work or storage guards.
pub(super) fn aggregate_commitments<'a, P, D>(
    storage: &'a mut DynamicStorage<P, D>,
    catalogue: &[D],
    commitments: &[Commitment<D>],
) -> Result<CommitmentAggregate<'a, P, D>, AggregateCommitmentError<D>>
where
    D: Copy + Ord,
{
    let (values, bytes) = if commitments.is_empty() {
        (Vec::new(), 0)
    } else {
        StorageCandidate::<P, D, AggregatedCommitment<D>>::try_with_capacity(
            StorageOwner::Dynamic(&mut *storage),
            commitments.len(),
            LedgerEvidencePurpose::Publication,
        )
        .map_err(AggregateCommitmentError::Evidence)?
        .into_parts()
    };
    let mut aggregate = CommitmentAggregate {
        values,
        admission: CommitmentEvidenceAdmission { storage, bytes },
    };
    for commitment in commitments {
        let dimension_position = catalogue
            .binary_search(&commitment.dimension)
            .map_err(|_| {
                AggregateCommitmentError::Ledger(LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                })
            })?;
        match aggregate
            .values
            .binary_search_by_key(&dimension_position, |entry| entry.dimension_position)
        {
            Ok(position) => {
                let Some(amount) = aggregate.values[position]
                    .amount
                    .checked_add(commitment.amount)
                else {
                    return Err(AggregateCommitmentError::Ledger(
                        LedgerCause::ArithmeticOverflow {
                            class: CounterClass::PendingPublication,
                            dimension: commitment.dimension,
                        },
                    ));
                };
                aggregate.values[position].amount = amount;
            }
            Err(position) => aggregate.values.insert(
                position,
                AggregatedCommitment {
                    dimension: commitment.dimension,
                    dimension_position,
                    amount: commitment.amount,
                },
            ),
        }
    }
    Ok(aggregate)
}

pub(super) fn validate_commitments<D: Copy>(
    dimensions: &[DimensionEntry<D>],
    commitments: &[AggregatedCommitment<D>],
) -> Result<(), LedgerCause<D>> {
    for commitment in commitments {
        let entry = dimensions
            .get(commitment.dimension_position)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        let admitted = entry
            .state
            .committed
            .checked_add(entry.state.pending_publication)
            .and_then(|value| value.checked_add(commitment.amount))
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: CounterClass::PendingPublication,
                dimension: commitment.dimension,
            })?;
        if admitted > entry.policy.committed_limit {
            return Err(LedgerCause::LimitExceeded {
                class: CounterClass::Committed,
                dimension: commitment.dimension,
                value: admitted,
                limit: entry.policy.committed_limit,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_typed_commitments<O, P, D, Destination>(
    dimensions: &[DimensionEntry<D>],
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    validate_typed_commitment_batches(dimensions, [Some(batch)])
}

pub(super) fn validate_typed_commitment_pair<O, P, D, Destination>(
    dimensions: &[DimensionEntry<D>],
    first: &AttributionReservationBatch<O, P, D, Destination>,
    second: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    validate_typed_commitment_batches(dimensions, [Some(first), Some(second)])
}

pub(super) fn validate_typed_commitment_batches<O, P, D, Destination, const N: usize>(
    dimensions: &[DimensionEntry<D>],
    batches: [Option<&AttributionReservationBatch<O, P, D, Destination>>; N],
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    let all = || {
        batches
            .into_iter()
            .flatten()
            .flat_map(|batch| batch.commitments.iter())
    };
    for (index, commitment) in all().enumerate() {
        if all()
            .take(index)
            .any(|prior| prior.dimension_position == commitment.dimension_position)
        {
            continue;
        }
        let amount = all()
            .filter(|candidate| candidate.dimension_position == commitment.dimension_position)
            .try_fold(0_u64, |total, candidate| {
                total.checked_add(candidate.amount)
            })
            .ok_or(LedgerCause::ArithmeticOverflow {
                class: match Destination::KIND {
                    DestinationKind::CanonicalOutput => CounterClass::Committed,
                    DestinationKind::OutcomeEvidence => CounterClass::PublishedOutcomeEvidence,
                },
                dimension: commitment.dimension,
            })?;
        let entry = dimensions
            .get(commitment.dimension_position)
            .ok_or(LedgerCause::FinalSnapshotShapeMismatch)?;
        let (current, limit, class) = match Destination::KIND {
            DestinationKind::CanonicalOutput => (
                entry.state.committed,
                entry.policy.committed_limit,
                CounterClass::Committed,
            ),
            DestinationKind::OutcomeEvidence => (
                entry.state.published_outcome_evidence,
                entry.policy.published_outcome_evidence_limit,
                CounterClass::PublishedOutcomeEvidence,
            ),
        };
        let value = current
            .checked_add(amount)
            .ok_or(LedgerCause::ArithmeticOverflow {
                class,
                dimension: commitment.dimension,
            })?;
        if value > limit {
            return Err(LedgerCause::LimitExceeded {
                class,
                dimension: commitment.dimension,
                value,
                limit,
            });
        }
    }
    Ok(())
}

pub(super) fn preflight_typed_publication_batch<O, P, D, Destination>(
    state: &StorageView<'_, P, D>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !batch.bound_positions_are_in_bounds(state.dimensions.len(), state.phase_states.len()) {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

pub(super) fn preflight_typed_discard_batch<O, P, D, Destination>(
    state: &StorageView<'_, P, D>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) -> Result<(), LedgerCause<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !batch.discard_positions_are_in_bounds(state.dimensions.len(), state.phase_states.len()) {
        return Err(LedgerCause::FinalSnapshotShapeMismatch);
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) fn apply_typed_commitments<O, P, D, Destination>(
    dimensions: &mut [DimensionEntry<D>],
    batch: &AttributionReservationBatch<O, P, D, Destination>,
) where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: DestinationState,
{
    for commitment in &batch.commitments {
        let entry = &mut dimensions[commitment.dimension_position];
        match Destination::KIND {
            DestinationKind::CanonicalOutput => {
                entry.state.committed += commitment.amount;
                entry.state.committed_canonical_output += commitment.amount;
            }
            DestinationKind::OutcomeEvidence => {
                entry.state.published_outcome_evidence += commitment.amount;
            }
        }
    }
}
