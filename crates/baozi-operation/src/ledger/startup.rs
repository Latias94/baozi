use super::{
    AttributionBatchPlan, CanonicalOutput, CounterClass, LedgerCause, LedgerLimits,
    OutcomeEvidenceDestination, OutputCommitment, dimension_entry,
};
use crate::OperationOwner;
use std::fmt;

const STARTUP_ACCOUNT_SHARE_STEP_COUNT: usize = 4;

/// One retained allocation attribution that startup must be able to reserve.
///
/// The phase remains part of the qualified value even though the current frozen policy has no
/// phase-specific limit. That keeps startup qualification ready for a future phase-qualified
/// `LedgerLimits<P, D>` without weakening today's dimension and operation-wide checks.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartupRetainedAttribution<P, D> {
    phase: P,
    dimension: D,
    bytes: u64,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<P, D> StartupRetainedAttribution<P, D> {
    pub(crate) const fn new(phase: P, dimension: D, bytes: u64) -> Self {
        Self {
            phase,
            dimension,
            bytes,
        }
    }

    pub(crate) fn into_parts(self) -> (P, D, u64) {
        (self.phase, self.dimension, self.bytes)
    }
}

/// One linear attempted-and-consumed work step paired with one startup account share.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAccountShareStep<D> {
    charges: [(D, u64); 2],
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<D> StartupAccountShareStep<D> {
    pub(crate) fn into_charges(self) -> [(D, u64); 2] {
        self.charges
    }
}

/// The exact linear work steps consumed by retained startup's four account shares.
///
/// The fixed step count is defined only by this envelope. Qualification and execution both consume
/// these stored steps, so adding or removing an account share cannot silently drift from the pure
/// ledger proof.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAccountShareEnvelope<D> {
    steps: [StartupAccountShareStep<D>; STARTUP_ACCOUNT_SHARE_STEP_COUNT],
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<D: Copy> StartupAccountShareEnvelope<D> {
    pub(crate) const fn four_identical(charges: [(D, u64); 2]) -> Self {
        Self {
            steps: [
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
            ],
        }
    }

    pub(crate) fn into_steps(
        self,
    ) -> [StartupAccountShareStep<D>; STARTUP_ACCOUNT_SHARE_STEP_COUNT] {
        self.steps
    }
}

/// The four destination-qualified attribution batches required by retained startup.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupAttributionBatchPlans<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_assembly: AttributionBatchPlan<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    fallback: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    evidence: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<O, P, D> StartupAttributionBatchPlans<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) const fn new(
        canonical_assembly: AttributionBatchPlan<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        fallback: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        evidence: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            canonical_assembly,
            evidence_assembly,
            fallback,
            evidence,
        }
    }

    #[allow(
        clippy::type_complexity,
        reason = "the tuple preserves four static destinations"
    )]
    pub(crate) fn into_parts(
        self,
    ) -> (
        AttributionBatchPlan<O, P, D, CanonicalOutput>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
        AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    ) {
        (
            self.canonical_assembly,
            self.evidence_assembly,
            self.fallback,
            self.evidence,
        )
    }
}

/// Consuming output of one successful startup-ledger qualification.
///
/// This remains crate-private: outside callers cannot inspect or synthesize startup admission
/// evidence, while the retained owner can consume the exact typed inputs without reconstructing
/// them after crossing the side-effect boundary.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
pub(crate) struct StartupLedgerPlanParts<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) batches: StartupAttributionBatchPlans<O, P, D>,
    pub(crate) retained_attributions: [StartupRetainedAttribution<P, D>; 2],
    pub(crate) account_shares: StartupAccountShareEnvelope<D>,
    pub(crate) commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
}

/// Sealed proof that every deterministic ledger action needed by retained startup fits policy.
///
/// The only constructor is `LedgerLimits::try_qualify_startup`. Qualification is read-only and
/// allocation-free, so ordinary policy rejection happens before a ledger, coordinator, physical
/// account admission, or allocator can observe startup.
#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
#[must_use = "startup qualification must be consumed before crossing the side-effect boundary"]
pub(crate) struct StartupLedgerPlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    limits: LedgerLimits<D>,
    parts: StartupLedgerPlanParts<O, P, D>,
}

#[allow(
    dead_code,
    reason = "crate-private until retained startup consumes the pure qualification proof"
)]
impl<O, P, D> StartupLedgerPlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn into_parts(self) -> (LedgerLimits<D>, StartupLedgerPlanParts<O, P, D>) {
        (self.limits, self.parts)
    }
}

#[derive(Debug, Clone, Copy)]
struct StartupLiveEnvelope<D> {
    dimension: D,
    bytes: u64,
}

impl<D> StartupLiveEnvelope<D> {
    const fn new(dimension: D, bytes: u64) -> Self {
        Self { dimension, bytes }
    }
}

impl<D> LedgerLimits<D>
where
    D: Copy + Ord + fmt::Debug,
{
    /// Purely qualifies the complete deterministic ledger envelope for retained startup.
    #[allow(
        dead_code,
        reason = "crate-private until retained startup consumes the pure qualification proof"
    )]
    pub(crate) fn try_qualify_startup<O, P>(
        self,
        batches: StartupAttributionBatchPlans<O, P, D>,
        retained_attributions: [StartupRetainedAttribution<P, D>; 2],
        account_shares: StartupAccountShareEnvelope<D>,
        commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<StartupLedgerPlan<O, P, D>, LedgerCause<D>>
    where
        O: OperationOwner,
        P: Copy + Ord,
    {
        let live = [
            StartupLiveEnvelope::new(
                batches.fallback.capacity_dimension,
                batches.fallback.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                retained_attributions[0].dimension,
                retained_attributions[0].bytes,
            ),
            StartupLiveEnvelope::new(
                batches.evidence.capacity_dimension,
                batches.evidence.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                retained_attributions[1].dimension,
                retained_attributions[1].bytes,
            ),
            StartupLiveEnvelope::new(
                batches.canonical_assembly.capacity_dimension,
                batches.canonical_assembly.planned_bytes,
            ),
            StartupLiveEnvelope::new(
                batches.evidence_assembly.capacity_dimension,
                batches.evidence_assembly.planned_bytes,
            ),
        ];

        self.validate_startup_live_envelope(&live)?;
        self.validate_startup_account_shares(&account_shares)?;
        self.validate_startup_outcome_commitments(commitments)?;

        Ok(StartupLedgerPlan {
            limits: self,
            parts: StartupLedgerPlanParts {
                batches,
                retained_attributions,
                account_shares,
                commitments,
            },
        })
    }

    fn validate_startup_live_envelope<const N: usize>(
        &self,
        live: &[StartupLiveEnvelope<D>; N],
    ) -> Result<(), LedgerCause<D>> {
        let mut operation_live = 0_u64;
        for (index, envelope) in live.iter().copied().enumerate() {
            let entry = dimension_entry(&self.dimensions, envelope.dimension).ok_or(
                LedgerCause::UnknownDimension {
                    dimension: envelope.dimension,
                },
            )?;
            let dimension_limit =
                entry
                    .policy
                    .live_byte_limit
                    .ok_or(LedgerCause::DimensionNotReservable {
                        dimension: envelope.dimension,
                    })?;
            let dimension_live = live[..=index]
                .iter()
                .filter(|candidate| candidate.dimension == envelope.dimension)
                .try_fold(0_u64, |bytes, candidate| {
                    bytes
                        .checked_add(candidate.bytes)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Live,
                            dimension: envelope.dimension,
                        })
                })?;
            if dimension_live > dimension_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                    value: dimension_live,
                    limit: dimension_limit,
                });
            }
            operation_live = operation_live.checked_add(envelope.bytes).ok_or(
                LedgerCause::ArithmeticOverflow {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                },
            )?;
            if operation_live > self.operation_live_byte_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Live,
                    dimension: envelope.dimension,
                    value: operation_live,
                    limit: self.operation_live_byte_limit,
                });
            }
        }
        Ok(())
    }

    fn validate_startup_account_shares(
        &self,
        account_shares: &StartupAccountShareEnvelope<D>,
    ) -> Result<(), LedgerCause<D>> {
        let all = || {
            account_shares
                .steps
                .iter()
                .flat_map(|step| step.charges.iter())
                .copied()
        };
        for (index, (dimension, _)) in all().enumerate() {
            let entry = dimension_entry(&self.dimensions, dimension)
                .ok_or(LedgerCause::UnknownDimension { dimension })?;
            if all().take(index).any(|(previous, _)| previous == dimension) {
                continue;
            }

            let attempted = all()
                .filter(|(candidate, _)| *candidate == dimension)
                .try_fold(0_u64, |total, (_, amount)| {
                    total
                        .checked_add(amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Attempted,
                            dimension,
                        })
                })?;
            if attempted > entry.policy.attempted_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Attempted,
                    dimension,
                    value: attempted,
                    limit: entry.policy.attempted_limit,
                });
            }

            let consumed = all()
                .filter(|(candidate, _)| *candidate == dimension)
                .try_fold(0_u64, |total, (_, amount)| {
                    total
                        .checked_add(amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::Consumed,
                            dimension,
                        })
                })?;
            if consumed > attempted {
                return Err(LedgerCause::ConsumedExceedsAttempted {
                    dimension,
                    consumed,
                    attempted,
                });
            }
            if consumed > entry.policy.consumed_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::Consumed,
                    dimension,
                    value: consumed,
                    limit: entry.policy.consumed_limit,
                });
            }
        }
        Ok(())
    }

    fn validate_startup_outcome_commitments(
        &self,
        commitments: [OutputCommitment<D, OutcomeEvidenceDestination>; 2],
    ) -> Result<(), LedgerCause<D>> {
        for (index, commitment) in commitments.iter().copied().enumerate() {
            let entry = dimension_entry(&self.dimensions, commitment.dimension).ok_or(
                LedgerCause::UnknownDimension {
                    dimension: commitment.dimension,
                },
            )?;
            if commitments[..index]
                .iter()
                .any(|previous| previous.dimension == commitment.dimension)
            {
                continue;
            }
            let amount = commitments
                .iter()
                .filter(|candidate| candidate.dimension == commitment.dimension)
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.amount)
                        .ok_or(LedgerCause::ArithmeticOverflow {
                            class: CounterClass::PublishedOutcomeEvidence,
                            dimension: commitment.dimension,
                        })
                })?;
            if amount > entry.policy.published_outcome_evidence_limit {
                return Err(LedgerCause::LimitExceeded {
                    class: CounterClass::PublishedOutcomeEvidence,
                    dimension: commitment.dimension,
                    value: amount,
                    limit: entry.policy.published_outcome_evidence_limit,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
