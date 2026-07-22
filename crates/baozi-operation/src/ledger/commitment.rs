use super::{
    AttributionReservationBatch, Commitment, CounterClass, DestinationKind, DestinationState,
    DimensionEntry, LedgerCause, LedgerEvidenceError, LedgerEvidencePurpose, LedgerState,
    PublicationDestination, capacity_bytes, dimension_entry, release_evidence,
    try_grow_evidence_vec,
};
use crate::OperationOwner;

pub(super) enum AggregateCommitmentError<D> {
    Ledger(LedgerCause<D>),
    Evidence(LedgerEvidenceError),
}

pub(super) struct CommitmentAggregate<D> {
    pub(super) entries: Vec<AggregatedCommitment<D>>,
    pub(super) evidence_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AggregatedCommitment<D> {
    pub(super) dimension: D,
    pub(super) dimension_position: usize,
    pub(super) amount: u64,
}

struct CommitmentAggregateAssembly<'a, P, D> {
    state: &'a mut LedgerState<P, D>,
    entries: Vec<AggregatedCommitment<D>>,
    evidence_bytes: u64,
    armed: bool,
}

impl<P, D> CommitmentAggregateAssembly<'_, P, D> {
    fn finish(mut self) -> CommitmentAggregate<D> {
        self.armed = false;
        CommitmentAggregate {
            entries: std::mem::take(&mut self.entries),
            evidence_bytes: self.evidence_bytes,
        }
    }
}

impl<P, D> Drop for CommitmentAggregateAssembly<'_, P, D> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        drop(std::mem::take(&mut self.entries));
        release_evidence(self.state, self.evidence_bytes);
    }
}

pub(super) fn aggregate_commitments<P, D>(
    state: &mut LedgerState<P, D>,
    commitments: &[Commitment<D>],
) -> Result<CommitmentAggregate<D>, AggregateCommitmentError<D>>
where
    D: Copy + Ord,
{
    let mut aggregate = Vec::new();
    let evidence_before = state.current_evidence_bytes;
    if let Err(cause) = try_grow_evidence_vec(
        &mut aggregate,
        commitments.len(),
        &mut state.current_evidence_bytes,
        &mut state.peak_evidence_bytes,
        state.evidence_byte_limit,
        LedgerEvidencePurpose::Publication,
    ) {
        let retained = capacity_bytes(&aggregate).unwrap_or(0);
        drop(aggregate);
        release_evidence(state, retained);
        return Err(AggregateCommitmentError::Evidence(cause));
    }
    let evidence_bytes = state.current_evidence_bytes.saturating_sub(evidence_before);
    let mut assembly = CommitmentAggregateAssembly {
        state,
        entries: aggregate,
        evidence_bytes,
        armed: true,
    };
    for commitment in commitments {
        let dimension_position = assembly
            .state
            .dimensions
            .binary_search_by_key(&commitment.dimension, |entry| entry.dimension)
            .map_err(|_| {
                AggregateCommitmentError::Ledger(LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                })
            })?;
        match assembly
            .entries
            .binary_search_by_key(&dimension_position, |entry| entry.dimension_position)
        {
            Ok(position) => {
                let Some(amount) = assembly.entries[position]
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
                assembly.entries[position].amount = amount;
            }
            Err(position) => assembly.entries.insert(
                position,
                AggregatedCommitment {
                    dimension: commitment.dimension,
                    dimension_position,
                    amount: commitment.amount,
                },
            ),
        }
    }
    Ok(assembly.finish())
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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
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
            .any(|prior| prior.dimension == commitment.dimension)
        {
            continue;
        }
        let amount = all()
            .filter(|candidate| candidate.dimension == commitment.dimension)
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
        let entry = dimension_entry(dimensions, commitment.dimension).ok_or(
            LedgerCause::UnknownDimension {
                dimension: commitment.dimension,
            },
        )?;
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

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) fn preflight_typed_publication_batch<O, P, D, Destination>(
    state: &LedgerState<P, D>,
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
    state: &LedgerState<P, D>,
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
