use super::{
    SharedStorage, StorageGuard, StorageView, StorageViewMut, lock_storage,
    phase_positions_are_valid,
};
use crate::ledger::{
    DimensionEntry, LedgerEvidenceError, LedgerEvidencePurpose, LedgerHeader, LedgerState,
    PhaseEntry, PhaseState, release_evidence,
};
use crate::{CapacityPlan, CapacityPlanError};
use std::mem::size_of;
use std::sync::{Arc, Mutex};

pub(in crate::ledger) const FINITE_DIMENSION_COUNT: usize = 6;
pub(in crate::ledger) const FINITE_PHASE_COUNT: usize = 3;

#[derive(Debug)]
pub(in crate::ledger) struct FiniteStorageState<P, D> {
    pub(in crate::ledger) dimensions: [DimensionEntry<D>; FINITE_DIMENSION_COUNT],
    pub(in crate::ledger) phases: [PhaseEntry<P>; FINITE_PHASE_COUNT],
    pub(in crate::ledger) phase_states: [PhaseState; FINITE_PHASE_COUNT],
    pub(in crate::ledger) header: LedgerHeader<P>,
}

impl<P, D> FiniteStorageState<P, D> {
    pub(in crate::ledger) fn new(
        dimensions: [DimensionEntry<D>; FINITE_DIMENSION_COUNT],
        phases: [PhaseEntry<P>; FINITE_PHASE_COUNT],
        header: LedgerHeader<P>,
    ) -> Self {
        Self {
            dimensions,
            phases,
            phase_states: [PhaseState::default(); FINITE_PHASE_COUNT],
            header,
        }
    }
}

impl<P, D> FiniteStorageState<P, D> {
    pub(in crate::ledger) fn view(&self) -> StorageView<'_, P, D> {
        StorageView {
            dimensions: &self.dimensions,
            phase_states: &self.phase_states,
            header: &self.header,
        }
    }

    pub(in crate::ledger) fn view_mut(&mut self) -> StorageViewMut<'_, P, D> {
        StorageViewMut {
            dimensions: &mut self.dimensions,
            phase_states: &mut self.phase_states,
            header: &mut self.header,
        }
    }

    fn qualify_shape(&self) -> Result<(), LedgerEvidenceError> {
        if !phase_positions_are_valid(
            &self.phases,
            self.phase_states.len(),
            FINITE_PHASE_COUNT,
            self.header
                .last_completed_phase
                .as_ref()
                .map(|completed| completed.state_position),
        ) {
            return Err(LedgerEvidenceError::SnapshotShapeMismatch);
        }
        Ok(())
    }
}

#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "the finite owner keeps its fixed storage inline until the first escaping lease"
)]
pub(in crate::ledger) enum FiniteStorage<P, D> {
    Inline(Option<FiniteStorageState<P, D>>),
    Shared(SharedStorage<P, D>),
}

pub(in crate::ledger) enum FiniteStorageRead<'storage, P, D> {
    Inline(&'storage FiniteStorageState<P, D>),
    Shared(StorageGuard<'storage, P, D>),
}

pub(in crate::ledger) enum FiniteStorageWrite<'storage, P, D> {
    Inline(&'storage mut FiniteStorageState<P, D>),
    Shared(StorageGuard<'storage, P, D>),
}

impl<P, D> FiniteStorageRead<'_, P, D> {
    pub(in crate::ledger) fn view(&self) -> StorageView<'_, P, D> {
        match self {
            Self::Inline(state) => state.view(),
            Self::Shared(state) => state.view(),
        }
    }

    pub(in crate::ledger) fn phases(&self) -> &[PhaseEntry<P>] {
        match self {
            Self::Inline(state) => &state.phases,
            Self::Shared(state) => &state.phases,
        }
    }
}

impl<P, D> FiniteStorageWrite<'_, P, D> {
    pub(in crate::ledger) fn phases(&self) -> &[PhaseEntry<P>] {
        match self {
            Self::Inline(state) => &state.phases,
            Self::Shared(state) => &state.phases,
        }
    }

    pub(in crate::ledger) fn phase_positions_are_valid(&self) -> bool {
        let view = self.view();
        phase_positions_are_valid(
            self.phases(),
            view.phase_states.len(),
            FINITE_PHASE_COUNT,
            view.last_completed_phase
                .as_ref()
                .map(|entry| entry.state_position),
        )
    }

    pub(in crate::ledger) fn view(&self) -> StorageView<'_, P, D> {
        match self {
            Self::Inline(state) => state.view(),
            Self::Shared(state) => state.view(),
        }
    }

    pub(in crate::ledger) fn view_mut(&mut self) -> StorageViewMut<'_, P, D> {
        match self {
            Self::Inline(state) => state.view_mut(),
            Self::Shared(state) => state.view_mut(),
        }
    }
}

impl<P, D> FiniteStorage<P, D> {
    pub(in crate::ledger) fn read(&self) -> FiniteStorageRead<'_, P, D> {
        match self {
            Self::Inline(state) => FiniteStorageRead::Inline(
                state
                    .as_ref()
                    .expect("live finite inline storage remains installed"),
            ),
            Self::Shared(state) => FiniteStorageRead::Shared(lock_storage(state)),
        }
    }

    pub(in crate::ledger) fn write(&mut self) -> FiniteStorageWrite<'_, P, D> {
        match self {
            Self::Inline(state) => FiniteStorageWrite::Inline(
                state
                    .as_mut()
                    .expect("live finite inline storage remains installed"),
            ),
            Self::Shared(state) => FiniteStorageWrite::Shared(lock_storage(state)),
        }
    }

    pub(in crate::ledger) fn shared(&self) -> Option<&SharedStorage<P, D>> {
        match self {
            Self::Inline(_) => None,
            Self::Shared(state) => Some(state),
        }
    }

    pub(in crate::ledger) fn ensure_shared(
        &mut self,
    ) -> Result<&SharedStorage<P, D>, LedgerEvidenceError> {
        if let Self::Inline(source) = self {
            let plan = FinitePromotionPlan::new(
                source
                    .as_ref()
                    .ok_or(LedgerEvidenceError::SnapshotShapeMismatch)?,
            )?;
            let mut candidate = FinitePromotionCandidate::new(source, &plan)?;
            candidate.allocate(&plan)?;
            let shared = candidate.install(&plan)?;
            *self = Self::Shared(shared);
        }
        match self {
            Self::Shared(state) => Ok(state),
            Self::Inline(_) => unreachable!("finite promotion selects the installed shared owner"),
        }
    }
}

struct FinitePromotionPlan {
    dimensions: CapacityPlan,
    phases: CapacityPlan,
    phase_states: CapacityPlan,
    payload_bytes: u64,
    envelope_bytes: u64,
}

impl FinitePromotionPlan {
    fn new<P, D>(source: &FiniteStorageState<P, D>) -> Result<Self, LedgerEvidenceError> {
        source.qualify_shape()?;
        let dimensions = CapacityPlan::for_type::<DimensionEntry<D>>(0, 0, source.dimensions.len())
            .map_err(promotion_capacity_error)?;
        let phases = CapacityPlan::for_type::<PhaseEntry<P>>(0, 0, source.phases.len())
            .map_err(promotion_capacity_error)?;
        let phase_states = CapacityPlan::for_type::<PhaseState>(0, 0, source.phase_states.len())
            .map_err(promotion_capacity_error)?;
        let payload_bytes = u64::try_from(size_of::<Mutex<Option<LedgerState<P, D>>>>())
            .map_err(|_| promotion_capacity_error(CapacityPlanError::CapacityOverflow))?;
        let envelope_bytes = [dimensions, phases, phase_states]
            .into_iter()
            .try_fold(payload_bytes, |total, plan| {
                total
                    .checked_add(plan.envelope_delta_bytes()?)
                    .ok_or(CapacityPlanError::CapacityOverflow)
            })
            .map_err(promotion_capacity_error)?;
        Ok(Self {
            dimensions,
            phases,
            phase_states,
            payload_bytes,
            envelope_bytes,
        })
    }
}

struct FinitePromotionAdmission<'storage, P, D> {
    source: &'storage mut Option<FiniteStorageState<P, D>>,
    bytes: u64,
}

impl<P, D> Drop for FinitePromotionAdmission<'_, P, D> {
    fn drop(&mut self) {
        if self.bytes != 0
            && let Some(state) = self.source.as_mut()
        {
            release_evidence(&mut state.header, self.bytes);
        }
    }
}

struct FinitePromotionCandidate<'storage, P, D> {
    shared: SharedStorage<P, D>,
    dimensions: Vec<DimensionEntry<D>>,
    phases: Vec<PhaseEntry<P>>,
    phase_states: Vec<PhaseState>,
    admission: FinitePromotionAdmission<'storage, P, D>,
}

impl<'storage, P, D> FinitePromotionCandidate<'storage, P, D> {
    fn new(
        source: &'storage mut Option<FiniteStorageState<P, D>>,
        plan: &FinitePromotionPlan,
    ) -> Result<Self, LedgerEvidenceError> {
        let state = source
            .as_mut()
            .ok_or(LedgerEvidenceError::SnapshotShapeMismatch)?;
        let admitted = state
            .header
            .current_administrative_bytes
            .checked_add(plan.envelope_bytes)
            .ok_or_else(|| promotion_capacity_error(CapacityPlanError::CapacityOverflow))?;
        if admitted > state.header.evidence_byte_limit {
            return Err(LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::SharedStorage,
                value: admitted,
                limit: state.header.evidence_byte_limit,
            });
        }
        state.header.current_administrative_bytes = admitted;
        state.header.peak_administrative_bytes =
            state.header.peak_administrative_bytes.max(admitted);
        let admission = FinitePromotionAdmission {
            source,
            bytes: plan.envelope_bytes,
        };
        #[cfg(test)]
        if crate::ledger::test_take_evidence_allocation_failure() {
            return Err(LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::SharedStorage,
                requested_bytes: plan.payload_bytes,
            });
        }
        Ok(Self {
            shared: Arc::new(Mutex::new(None)),
            dimensions: Vec::new(),
            phases: Vec::new(),
            phase_states: Vec::new(),
            admission,
        })
    }

    fn allocate(&mut self, plan: &FinitePromotionPlan) -> Result<(), LedgerEvidenceError> {
        allocate_promotion_vec(&mut self.dimensions, plan.dimensions)?;
        allocate_promotion_vec(&mut self.phases, plan.phases)?;
        allocate_promotion_vec(&mut self.phase_states, plan.phase_states)?;
        Ok(())
    }

    fn install(
        mut self,
        plan: &FinitePromotionPlan,
    ) -> Result<SharedStorage<P, D>, LedgerEvidenceError> {
        let source = self
            .admission
            .source
            .as_ref()
            .ok_or(LedgerEvidenceError::SnapshotShapeMismatch)?;
        source.qualify_shape()?;
        if source.header.current_administrative_bytes > source.header.evidence_byte_limit {
            return Err(LedgerEvidenceError::LimitExceeded {
                purpose: LedgerEvidencePurpose::SharedStorage,
                value: source.header.current_administrative_bytes,
                limit: source.header.evidence_byte_limit,
            });
        }
        source
            .header
            .current_administrative_bytes
            .checked_sub(self.admission.bytes)
            .ok_or_else(|| promotion_capacity_error(CapacityPlanError::CapacityOverflow))?;
        let actual_bytes = [
            qualify_promotion_vec(&self.dimensions, plan.dimensions)?,
            qualify_promotion_vec(&self.phases, plan.phases)?,
            qualify_promotion_vec(&self.phase_states, plan.phase_states)?,
        ]
        .into_iter()
        .try_fold(plan.payload_bytes, |total, bytes| {
            total
                .checked_add(bytes)
                .ok_or(CapacityPlanError::CapacityOverflow)
        })
        .map_err(promotion_capacity_error)?;
        let unused_bytes = self
            .admission
            .bytes
            .checked_sub(actual_bytes)
            .ok_or_else(|| promotion_capacity_error(CapacityPlanError::CapacityOverflow))?;
        let vacant = Arc::get_mut(&mut self.shared)
            .ok_or(LedgerEvidenceError::SnapshotShapeMismatch)?
            .get_mut()
            .map_err(|_| LedgerEvidenceError::SnapshotShapeMismatch)?;
        if vacant.is_some() {
            return Err(LedgerEvidenceError::SnapshotShapeMismatch);
        }
        let source = self
            .admission
            .source
            .as_mut()
            .expect("finite promotion qualified the original source");
        source.header.current_administrative_bytes -= unused_bytes;
        self.admission.bytes = actual_bytes;
        let FiniteStorageState {
            dimensions,
            phases,
            phase_states,
            header,
        } = self
            .admission
            .source
            .take()
            .expect("finite promotion qualified the owned source");
        self.admission.bytes = 0;
        for entry in dimensions {
            self.dimensions.push(entry);
        }
        for entry in phases {
            self.phases.push(entry);
        }
        for state in phase_states {
            self.phase_states.push(state);
        }
        *vacant = Some(LedgerState {
            dimensions: std::mem::take(&mut self.dimensions),
            phases: std::mem::take(&mut self.phases),
            phase_states: std::mem::take(&mut self.phase_states),
            dimension_snapshot_staging: Vec::new(),
            phase_snapshot_staging: Vec::new(),
            header,
        });
        Ok(self.shared)
    }
}

fn allocate_promotion_vec<T>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
) -> Result<(), LedgerEvidenceError> {
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(promotion_capacity_error)?;
    let requested_bytes = plan
        .envelope_delta_bytes()
        .map_err(promotion_capacity_error)?;
    #[cfg(test)]
    if crate::ledger::test_take_evidence_allocation_failure() {
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::SharedStorage,
            requested_bytes,
        });
    }
    if values
        .try_reserve_exact(plan.allocator_additional())
        .is_err()
    {
        return Err(LedgerEvidenceError::AllocationFailed {
            purpose: LedgerEvidencePurpose::SharedStorage,
            requested_bytes,
        });
    }
    Ok(())
}

fn qualify_promotion_vec<T>(
    values: &Vec<T>,
    plan: CapacityPlan,
) -> Result<u64, LedgerEvidenceError> {
    #[cfg(test)]
    let observed_capacity = if crate::ledger::test_take_evidence_over_envelope_failure() {
        plan.envelope_capacity().saturating_add(1)
    } else {
        values.capacity()
    };
    #[cfg(not(test))]
    let observed_capacity = values.capacity();
    plan.finish(values.len(), observed_capacity)
        .map_err(promotion_capacity_error)
}

fn promotion_capacity_error(cause: CapacityPlanError) -> LedgerEvidenceError {
    LedgerEvidenceError::Capacity {
        purpose: LedgerEvidencePurpose::SharedStorage,
        cause,
    }
}

#[cfg(test)]
#[path = "finite/tests.rs"]
mod tests;
