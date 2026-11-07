//! The actual three-batch, one-physical-part startup graph of a finite invocation.

use super::*;
use crate::ledger::reservation::AttributionBatchOwner;
use crate::ledger::{FallbackBatchReadView, FallbackBatchView};

pub(crate) struct FiniteStartupLedgerPlanParts<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) canonical_assembly: AttributionBatchPlan<O, P, D, CanonicalOutput>,
    pub(crate) evidence_assembly: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) fallback: AttributionBatchPlan<O, P, D, OutcomeEvidenceDestination>,
    pub(crate) fallback_attribution: StartupRetainedAttribution<P, D>,
    pub(crate) account_shares: StartupAccountShareEnvelope<D, 3>,
    pub(crate) fallback_commitment: OutputCommitment<D, OutcomeEvidenceDestination>,
}

/// Genuine runtime batch owners; there is no numerical-evidence batch or allocation.
pub(crate) struct FiniteFallbackBatchBaseline<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
}

type FiniteFallbackBatchParts<O, P, D> = (
    AttributionReservationBatch<O, P, D, CanonicalOutput>,
    AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
);

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FiniteFallbackBatchBaseline<O, P, D> {
    pub(crate) fn new_unbound(
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        fallback: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    ) -> Self {
        Self {
            canonical_assembly,
            evidence_assembly,
            fallback,
        }
    }

    pub(crate) fn view(&mut self) -> FallbackBatchView<'_, O, P, D> {
        FallbackBatchView {
            canonical_assembly: &mut self.canonical_assembly,
            evidence_assembly: &mut self.evidence_assembly,
            fallback: &mut self.fallback,
            numerical_evidence: None,
        }
    }

    pub(crate) fn as_read(&self) -> FallbackBatchReadView<'_, O, P, D> {
        FallbackBatchReadView {
            canonical_assembly: &self.canonical_assembly,
            evidence_assembly: &self.evidence_assembly,
            fallback: &self.fallback,
            numerical_evidence: None,
        }
    }

    pub(crate) fn into_parts(self) -> FiniteFallbackBatchParts<O, P, D> {
        (
            self.canonical_assembly,
            self.evidence_assembly,
            self.fallback,
        )
    }
}

pub(crate) struct FiniteStartupAttributionGraph<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ownership: Option<FiniteStartupGraphOwnership<O, P, D>>,
}

struct FiniteStartupGraphOwnership<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    identity: super::super::storage::LedgerIdentity,
    runtime_owners: [Option<SharedStorage<P, D>>; 3],
    canonical_assembly: StartupAttributionBatchBackings<O, P, D, CanonicalOutput>,
    evidence_assembly: StartupAttributionBatchBackings<O, P, D, OutcomeEvidenceDestination>,
    fallback: StartupAttributionBatchBackings<O, P, D, OutcomeEvidenceDestination>,
    // Empty assembly backings issue neither a capacity token nor a shared owner.
    cores: [Option<ReservationCore<O, P, D>>; 4],
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FiniteStartupAttributionGraph<O, P, D> {
    pub(crate) fn belongs_to_view(&self, ledger: &LedgerReadView<'_, O, P, D>) -> bool {
        self.ownership.as_ref().is_some_and(|ownership| {
            ownership.identity == ledger.identity
                && ledger.storage.shared().is_some_and(|shared| {
                    ownership
                        .runtime_owners
                        .iter()
                        .flatten()
                        .all(|owner| Arc::ptr_eq(shared, owner))
                })
        })
    }

    /// The physical Ready owner alone can discharge this allocation-free consuming move.
    pub(crate) fn into_runtime_baseline(
        mut self,
        _authority: ReadyStartupGraphSplitAuthority,
    ) -> FiniteFallbackBatchBaseline<O, P, D> {
        let FiniteStartupGraphOwnership {
            identity,
            runtime_owners: [canonical_owner, evidence_owner, fallback_owner],
            canonical_assembly,
            evidence_assembly,
            mut fallback,
            cores:
                [
                    canonical_cover,
                    evidence_cover,
                    fallback_cover,
                    fallback_attribution,
                ],
        } = self
            .ownership
            .take()
            .expect("Ready owns the original finite graph");
        fallback.attributions.push(OperationAttributionReservation {
            core: fallback_attribution.expect("the physical fallback has one real attribution"),
            destination: PhantomData,
        });
        let owner = |shared: Option<SharedStorage<P, D>>| {
            shared.map_or(
                AttributionBatchOwner::Empty(identity),
                AttributionBatchOwner::Shared,
            )
        };
        FiniteFallbackBatchBaseline {
            canonical_assembly: into_runtime_batch(
                canonical_assembly,
                canonical_cover,
                owner(canonical_owner),
            ),
            evidence_assembly: into_runtime_batch(
                evidence_assembly,
                evidence_cover,
                owner(evidence_owner),
            ),
            fallback: into_runtime_batch(fallback, fallback_cover, owner(fallback_owner)),
        }
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> Drop
    for FiniteStartupAttributionGraph<O, P, D>
{
    fn drop(&mut self) {
        let Some(mut ownership) = self.ownership.take() else {
            return;
        };
        // All real backing dies before any credit is refunded, including partial allocation.
        drop_startup_batch_backings(&mut ownership.canonical_assembly);
        drop_startup_batch_backings(&mut ownership.evidence_assembly);
        drop_startup_batch_backings(&mut ownership.fallback);
        let shared = ownership.runtime_owners[2]
            .as_ref()
            .expect("fallback owns storage identity");
        let mut state = lock(shared);
        test_note_startup_graph_ledger_release_lock();
        for core in ownership.cores.iter_mut().flatten() {
            core.release_locked(&mut state);
        }
    }
}

type BuiltFiniteStartupGraph<O, P, D> = (
    FiniteStartupAttributionGraph<O, P, D>,
    StartupAccountShareEnvelope<D, 3>,
);

impl<O, P, D> LedgerView<'_, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn try_bind_finite_startup_attribution_graph(
        &mut self,
        parts: FiniteStartupLedgerPlanParts<O, P, D>,
    ) -> Result<BuiltFiniteStartupGraph<O, P, D>, AttributionBatchCreateError<D>> {
        if parts.fallback.attribution_limit == 0 || parts.fallback.commitment_limit == 0 {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        }
        let canonical_spec = StartupReservationSpec::for_batch(&parts.canonical_assembly);
        let evidence_spec = StartupReservationSpec::for_batch(&parts.evidence_assembly);
        let fallback_spec = StartupReservationSpec::for_batch(&parts.fallback);
        let attribution_spec = StartupReservationSpec::for_retained(parts.fallback_attribution);
        let work = parts.account_shares.work_specs();
        let commitments = [parts.fallback_commitment];
        // Only genuine capacity owners enter the shared numerical algorithm. The four arms
        // select fixed input/output shapes; they contain no alternative accounting logic.
        let (cores, [commitment]) = match (canonical_spec.bytes != 0, evidence_spec.bytes != 0) {
            (true, true) => {
                let StartupLedgerCommit {
                    cores: [c, e, f, a],
                    commitments,
                } = self
                    .try_admit_startup_ledger(
                        [
                            canonical_spec,
                            evidence_spec,
                            fallback_spec,
                            attribution_spec,
                        ],
                        work,
                        commitments,
                    )
                    .map_err(AttributionBatchCreateError::Ledger)?;
                ([Some(c), Some(e), Some(f), Some(a)], commitments)
            }
            (true, false) => {
                let StartupLedgerCommit {
                    cores: [c, f, a],
                    commitments,
                } = self
                    .try_admit_startup_ledger(
                        [canonical_spec, fallback_spec, attribution_spec],
                        work,
                        commitments,
                    )
                    .map_err(AttributionBatchCreateError::Ledger)?;
                ([Some(c), None, Some(f), Some(a)], commitments)
            }
            (false, true) => {
                let StartupLedgerCommit {
                    cores: [e, f, a],
                    commitments,
                } = self
                    .try_admit_startup_ledger(
                        [evidence_spec, fallback_spec, attribution_spec],
                        work,
                        commitments,
                    )
                    .map_err(AttributionBatchCreateError::Ledger)?;
                ([None, Some(e), Some(f), Some(a)], commitments)
            }
            (false, false) => {
                let StartupLedgerCommit {
                    cores: [f, a],
                    commitments,
                } = self
                    .try_admit_startup_ledger([fallback_spec, attribution_spec], work, commitments)
                    .map_err(AttributionBatchCreateError::Ledger)?;
                ([None, None, Some(f), Some(a)], commitments)
            }
        };
        let FiniteStartupLedgerPlanParts {
            canonical_assembly,
            evidence_assembly,
            fallback,
            account_shares,
            ..
        } = parts;
        let (canonical_assembly, canonical_plan) = empty_startup_batch(canonical_assembly);
        let (evidence_assembly, evidence_plan) = empty_startup_batch(evidence_assembly);
        let (fallback, fallback_plan) = empty_startup_batch(fallback);
        let shared = self
            .storage
            .state
            .shared()
            .expect("actual fallback attribution promoted storage");
        let runtime_owners =
            std::array::from_fn(|position| cores[position].as_ref().map(|_| Arc::clone(shared)));
        let mut graph = FiniteStartupAttributionGraph {
            ownership: Some(FiniteStartupGraphOwnership {
                identity: self.as_read().identity,
                runtime_owners,
                canonical_assembly,
                evidence_assembly,
                fallback,
                cores,
            }),
        };
        let ownership = graph
            .ownership
            .as_mut()
            .expect("the original graph is installed");
        try_allocate_startup_batch(&mut ownership.canonical_assembly, canonical_plan)?;
        try_allocate_startup_batch(&mut ownership.evidence_assembly, evidence_plan)?;
        try_allocate_startup_batch(&mut ownership.fallback, fallback_plan)?;
        ownership.fallback.commitments.push(commitment);
        Ok((graph, account_shares))
    }
}

#[cfg(test)]
mod tests;
