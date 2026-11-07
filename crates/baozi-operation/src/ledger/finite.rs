//! Owned fixed-shape numerical state for the closed scene-edit invocation.

mod terminal;
pub(crate) use terminal::FiniteLedgerSnapshot;
mod attached;
pub(crate) use attached::{FiniteDetachedTypedFreezeProof, FiniteTypedFreezeDetachError};

use super::storage::finite::{
    FINITE_DIMENSION_COUNT, FINITE_PHASE_COUNT, FiniteStorage, FiniteStorageState,
};
use super::*;

pub(crate) struct FiniteLedgerLimits<D> {
    dimensions: [DimensionEntry<D>; FINITE_DIMENSION_COUNT],
    catalog: [D; FINITE_DIMENSION_COUNT],
    work: [WorkEntry<D>; FINITE_DIMENSION_COUNT],
    operation_live_byte_limit: u64,
    evidence_byte_limit: u64,
}

impl<D: Copy + Ord + fmt::Debug> FiniteLedgerLimits<D> {
    pub(crate) fn try_new(
        operation_live_byte_limit: u64,
        evidence_byte_limit: u64,
        mut policies: [(D, DimensionPolicy); FINITE_DIMENSION_COUNT],
    ) -> Result<Self, LedgerLimitsError<D>> {
        for (position, (dimension, policy)) in policies.iter().copied().enumerate() {
            validate_dimension_policy(dimension, policy)?;
            if policies[..position]
                .iter()
                .any(|(previous, _)| *previous == dimension)
            {
                return Err(LedgerLimitsError::DuplicateDimension { dimension });
            }
        }
        policies.sort_unstable_by_key(|(dimension, _)| *dimension);
        let catalog = policies.map(|(dimension, _)| dimension);
        let work = policies.map(|(dimension, policy)| WorkEntry {
            dimension,
            policy: WorkPolicy::from(policy),
            state: WorkDimensionState::default(),
        });
        let dimensions = policies.map(|(dimension, policy)| DimensionEntry {
            dimension,
            policy: StoragePolicy::from_policy(policy),
            state: StorageDimensionState::default(),
        });
        Ok(Self {
            dimensions,
            catalog,
            work,
            operation_live_byte_limit,
            evidence_byte_limit,
        })
    }

    pub(crate) fn dimension(&self, dimension: D) -> Option<DimensionPolicy> {
        let position = self.catalog.binary_search(&dimension).ok()?;
        Some(
            self.dimensions[position]
                .policy
                .with_work(self.work[position].policy),
        )
    }

    pub(crate) fn dimensions(&self) -> &[D; FINITE_DIMENSION_COUNT] {
        &self.catalog
    }
}

pub(crate) struct FiniteLedger<O, P, D> {
    pub(super) identity: LedgerIdentity,
    pub(super) catalog: [D; FINITE_DIMENSION_COUNT],
    pub(super) work: [WorkEntry<D>; FINITE_DIMENSION_COUNT],
    pub(super) work_header: work::WorkHeader,
    pub(super) phases: [PhaseEntry<P>; FINITE_PHASE_COUNT],
    pub(super) storage: FiniteStorage<P, D>,
    pub(super) dimension_snapshot_staging: [(D, DimensionSnapshot); FINITE_DIMENSION_COUNT],
    pub(super) phase_snapshot_staging: [(P, PhaseSnapshot); FINITE_PHASE_COUNT],
    pub(super) operation_live_byte_limit: u64,
    owner: PhantomData<fn() -> O>,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> FiniteLedger<O, P, D> {
    pub(crate) fn try_new(
        limits: FiniteLedgerLimits<D>,
        phases: [P; FINITE_PHASE_COUNT],
    ) -> Result<Self, LedgerCause<D>> {
        for position in 0..phases.len() {
            if phases[..position].contains(&phases[position]) {
                return Err(LedgerCause::DuplicatePhase);
            }
        }
        let mut phases = std::array::from_fn(|state_position| PhaseEntry {
            phase: phases[state_position],
            state_position,
        });
        phases.sort_unstable_by_key(|entry| entry.phase);
        let identity = LedgerIdentity::try_new().ok_or(LedgerCause::OwnerIdentityExhausted)?;
        let FiniteLedgerLimits {
            dimensions,
            catalog,
            work,
            operation_live_byte_limit,
            evidence_byte_limit,
        } = limits;
        let header = LedgerHeader {
            current_live_bytes: 0,
            peak_live_bytes: 0,
            active_reservations: 0,
            retained_output_bytes: 0,
            retained_output_reservations: 0,
            evidence_byte_limit,
            current_administrative_bytes: 0,
            peak_administrative_bytes: 0,
            publication: PublicationState::Open,
            last_completed_phase: None,
        };
        Ok(Self {
            identity,
            catalog,
            work,
            work_header: work::WorkHeader::new(),
            phases,
            storage: FiniteStorage::Inline(Some(FiniteStorageState::new(
                dimensions, phases, header,
            ))),
            dimension_snapshot_staging: catalog
                .map(|dimension| (dimension, DimensionSnapshot::default())),
            phase_snapshot_staging: phases.map(|entry| (entry.phase, PhaseSnapshot::default())),
            operation_live_byte_limit,
            owner: PhantomData,
        })
    }

    pub(crate) fn view(&mut self) -> LedgerView<'_, O, P, D> {
        LedgerView::new(
            self.identity,
            WorkView {
                catalog: &self.catalog,
                rows: &mut self.work,
                header: &mut self.work_header,
            },
            ExclusiveStorage::new(
                StorageOwner::Finite(&mut self.storage),
                &self.phases,
                &self.catalog,
                self.operation_live_byte_limit,
            ),
        )
    }

    pub(crate) fn as_read(&self) -> LedgerReadView<'_, O, P, D> {
        LedgerReadView {
            identity: self.identity,
            work: work::WorkReadView {
                catalog: &self.catalog,
                rows: &self.work,
                header: &self.work_header,
            },
            storage: storage::StorageRef::Finite(&self.storage),
            phases: &self.phases,
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    pub(crate) fn known_administrative_bytes(&self, progress_bytes: u64) -> Option<u64> {
        match &self.storage {
            FiniteStorage::Inline(Some(_)) => Some(progress_bytes),
            FiniteStorage::Inline(None) => None,
            FiniteStorage::Shared(shared) => {
                let state = lock(shared);
                [
                    capacity_bytes(&state.dimensions).ok()?,
                    capacity_bytes(&state.phases).ok()?,
                    capacity_bytes(&state.phase_states).ok()?,
                    capacity_bytes(&state.dimension_snapshot_staging).ok()?,
                    capacity_bytes(&state.phase_snapshot_staging).ok()?,
                    u64::try_from(size_of::<Mutex<Option<LedgerState<P, D>>>>()).ok()?,
                    progress_bytes,
                ]
                .into_iter()
                .try_fold(0_u64, u64::checked_add)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_shared_probe(&self) -> Option<LedgerTestProbe<P, D>> {
        self.storage.shared().map(|state| LedgerTestProbe {
            state: Arc::downgrade(state),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_shared_owner(&self) -> Option<crate::ledger::storage::SharedStorage<P, D>> {
        self.storage.shared().cloned()
    }

    #[cfg(test)]
    pub(crate) fn test_corrupt_storage_phase_position(&mut self) {
        match &mut self.storage {
            FiniteStorage::Inline(Some(state)) => state.phases[0].state_position = usize::MAX,
            FiniteStorage::Shared(shared) => lock(shared).phases[0].state_position = usize::MAX,
            FiniteStorage::Inline(None) => panic!("a test mutates the original installed storage"),
        }
    }
}

#[cfg(test)]
mod tests;
