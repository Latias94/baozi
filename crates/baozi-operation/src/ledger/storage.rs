use super::*;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) mod finite;
use finite::{FiniteStorage, FiniteStorageRead, FiniteStorageWrite};

static NEXT_LEDGER_IDENTITY: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LedgerIdentity(NonZeroU64);

impl LedgerIdentity {
    pub(super) fn try_new() -> Option<Self> {
        Self::try_next(&NEXT_LEDGER_IDENTITY)
    }

    pub(super) fn try_next(sequence: &AtomicU64) -> Option<Self> {
        sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .ok()
            .and_then(NonZeroU64::new)
            .map(Self)
    }
}

pub(super) type SharedStorage<P, D> = Arc<Mutex<Option<LedgerState<P, D>>>>;
pub(super) type WeakStorage<P, D> = std::sync::Weak<Mutex<Option<LedgerState<P, D>>>>;

pub(super) fn phase_positions_are_valid<P>(
    phases: &[PhaseEntry<P>],
    phase_state_count: usize,
    phase_staging_count: usize,
    last_completed_position: Option<usize>,
) -> bool {
    let phase_count = phases.len();
    phase_count == phase_state_count
        && phase_count == phase_staging_count
        && (0..phase_count).all(|state_position| {
            phases
                .iter()
                .filter(|entry| entry.state_position == state_position)
                .count()
                == 1
        })
        && last_completed_position.is_none_or(|position| position < phase_count)
}

#[derive(Debug)]
pub(super) enum DynamicStorage<P, D> {
    Inline(Option<LedgerState<P, D>>),
    Shared(SharedStorage<P, D>),
}

pub(super) enum StorageRead<'storage, P, D> {
    Inline(&'storage LedgerState<P, D>),
    Shared(StorageGuard<'storage, P, D>),
}

pub(super) enum StorageWrite<'storage, P, D> {
    Inline(&'storage mut LedgerState<P, D>),
    Shared(StorageGuard<'storage, P, D>),
}

pub(super) enum StorageOwner<'storage, P, D> {
    Dynamic(&'storage mut DynamicStorage<P, D>),
    Finite(&'storage mut FiniteStorage<P, D>),
}

pub(super) enum StorageRef<'storage, P, D> {
    Dynamic(&'storage DynamicStorage<P, D>),
    Finite(&'storage FiniteStorage<P, D>),
}

impl<P, D> StorageRef<'_, P, D> {
    pub(super) fn read(&self) -> SelectedStorageRead<'_, P, D> {
        match self {
            Self::Dynamic(owner) => SelectedStorageRead::Dynamic(owner.read()),
            Self::Finite(owner) => SelectedStorageRead::Finite(owner.read()),
        }
    }

    pub(super) fn shared(&self) -> Option<&SharedStorage<P, D>> {
        match self {
            Self::Dynamic(owner) => owner.shared(),
            Self::Finite(owner) => owner.shared(),
        }
    }

    pub(super) fn owns(&self, owner: &SharedStorage<P, D>) -> bool {
        self.shared()
            .is_some_and(|shared| Arc::ptr_eq(shared, owner))
    }
}

pub(super) enum SelectedStorageRead<'storage, P, D> {
    Dynamic(StorageRead<'storage, P, D>),
    Finite(FiniteStorageRead<'storage, P, D>),
}

pub(super) enum SelectedStorageWrite<'storage, P, D> {
    Dynamic(StorageWrite<'storage, P, D>),
    Finite(FiniteStorageWrite<'storage, P, D>),
}

impl<P, D> StorageOwner<'_, P, D> {
    pub(super) fn as_read(&self) -> StorageRef<'_, P, D> {
        match self {
            Self::Dynamic(owner) => StorageRef::Dynamic(owner),
            Self::Finite(owner) => StorageRef::Finite(owner),
        }
    }

    pub(super) fn reborrow(&mut self) -> StorageOwner<'_, P, D> {
        match self {
            Self::Dynamic(owner) => StorageOwner::Dynamic(owner),
            Self::Finite(owner) => StorageOwner::Finite(owner),
        }
    }

    pub(super) fn read(&self) -> SelectedStorageRead<'_, P, D> {
        match self {
            Self::Dynamic(owner) => SelectedStorageRead::Dynamic(owner.read()),
            Self::Finite(owner) => SelectedStorageRead::Finite(owner.read()),
        }
    }

    pub(super) fn write(&mut self) -> SelectedStorageWrite<'_, P, D> {
        match self {
            Self::Dynamic(owner) => SelectedStorageWrite::Dynamic(owner.write()),
            Self::Finite(owner) => SelectedStorageWrite::Finite(owner.write()),
        }
    }

    pub(super) fn shared(&self) -> Option<&SharedStorage<P, D>> {
        match self {
            Self::Dynamic(owner) => owner.shared(),
            Self::Finite(owner) => owner.shared(),
        }
    }

    pub(super) fn ensure_shared(&mut self) -> Result<&SharedStorage<P, D>, LedgerEvidenceError> {
        match self {
            Self::Dynamic(owner) => owner.ensure_shared(),
            Self::Finite(owner) => owner.ensure_shared(),
        }
    }
}

impl<P, D> SelectedStorageRead<'_, P, D> {
    pub(super) fn phases(&self) -> &[PhaseEntry<P>] {
        match self {
            Self::Dynamic(state) => &state.phases,
            Self::Finite(state) => state.phases(),
        }
    }

    pub(super) fn snapshot_staging_lengths(&self) -> (usize, usize) {
        match self {
            Self::Dynamic(state) => (
                state.dimension_snapshot_staging.len(),
                state.phase_snapshot_staging.len(),
            ),
            Self::Finite(_) => (finite::FINITE_DIMENSION_COUNT, finite::FINITE_PHASE_COUNT),
        }
    }

    pub(super) fn view(&self) -> StorageView<'_, P, D> {
        match self {
            Self::Dynamic(state) => state.view(),
            Self::Finite(state) => state.view(),
        }
    }
}

impl<P, D> SelectedStorageWrite<'_, P, D> {
    pub(super) fn phases(&self) -> &[PhaseEntry<P>] {
        match self {
            Self::Dynamic(state) => &state.phases,
            Self::Finite(state) => state.phases(),
        }
    }

    pub(super) fn snapshot_staging_lengths(&self) -> (usize, usize) {
        match self {
            Self::Dynamic(state) => (
                state.dimension_snapshot_staging.len(),
                state.phase_snapshot_staging.len(),
            ),
            Self::Finite(_) => (finite::FINITE_DIMENSION_COUNT, finite::FINITE_PHASE_COUNT),
        }
    }

    pub(super) fn phase_positions_are_valid(&self) -> bool {
        match self {
            Self::Dynamic(state) => phase_catalog_shape_is_valid(state),
            Self::Finite(state) => state.phase_positions_are_valid(),
        }
    }

    pub(super) fn view(&self) -> StorageView<'_, P, D> {
        match self {
            Self::Dynamic(state) => state.view(),
            Self::Finite(state) => state.view(),
        }
    }

    pub(super) fn view_mut(&mut self) -> StorageViewMut<'_, P, D> {
        match self {
            Self::Dynamic(state) => state.view_mut(),
            Self::Finite(state) => state.view_mut(),
        }
    }
}

impl<P, D> Deref for SelectedStorageRead<'_, P, D> {
    type Target = LedgerHeader<P>;

    fn deref(&self) -> &Self::Target {
        self.view().header
    }
}

impl<P, D> Deref for SelectedStorageWrite<'_, P, D> {
    type Target = LedgerHeader<P>;

    fn deref(&self) -> &Self::Target {
        self.view().header
    }
}

impl<P, D> DerefMut for SelectedStorageWrite<'_, P, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.view_mut().header
    }
}

pub(super) struct StorageGuard<'storage, P, D>(MutexGuard<'storage, Option<LedgerState<P, D>>>);

struct PromotionAdmission<'storage, P, D> {
    source: &'storage mut Option<LedgerState<P, D>>,
    bytes: u64,
}

impl<P, D> Drop for PromotionAdmission<'_, P, D> {
    fn drop(&mut self) {
        if self.bytes != 0
            && let Some(state) = self.source.as_mut()
        {
            release_evidence(state, self.bytes);
        }
    }
}

impl<P, D> Deref for StorageGuard<'_, P, D> {
    type Target = LedgerState<P, D>;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("issued shared storage remains installed")
    }
}

impl<P, D> DerefMut for StorageGuard<'_, P, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("issued shared storage remains installed")
    }
}

impl<P, D> Deref for StorageRead<'_, P, D> {
    type Target = LedgerState<P, D>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Inline(state) => state,
            Self::Shared(state) => state,
        }
    }
}

impl<P, D> Deref for StorageWrite<'_, P, D> {
    type Target = LedgerState<P, D>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Inline(state) => state,
            Self::Shared(state) => state,
        }
    }
}

impl<P, D> DerefMut for StorageWrite<'_, P, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Inline(state) => state,
            Self::Shared(state) => state,
        }
    }
}

impl<P, D> DynamicStorage<P, D> {
    pub(super) fn read(&self) -> StorageRead<'_, P, D> {
        match self {
            Self::Inline(state) => StorageRead::Inline(
                state
                    .as_ref()
                    .expect("live inline storage remains installed"),
            ),
            Self::Shared(state) => StorageRead::Shared(lock(state)),
        }
    }

    pub(super) fn write(&mut self) -> StorageWrite<'_, P, D> {
        match self {
            Self::Inline(state) => StorageWrite::Inline(
                state
                    .as_mut()
                    .expect("live inline storage remains installed"),
            ),
            Self::Shared(state) => StorageWrite::Shared(lock(state)),
        }
    }

    pub(super) fn shared(&self) -> Option<&SharedStorage<P, D>> {
        match self {
            Self::Inline(_) => None,
            Self::Shared(state) => Some(state),
        }
    }

    pub(super) fn owns(&self, owner: &SharedStorage<P, D>) -> bool {
        self.shared().is_some_and(|state| Arc::ptr_eq(state, owner))
    }

    pub(super) fn ensure_shared(&mut self) -> Result<&SharedStorage<P, D>, LedgerEvidenceError> {
        if let Self::Inline(source) = self {
            let state = source
                .as_ref()
                .ok_or(LedgerEvidenceError::SnapshotShapeMismatch)?;
            if !phase_catalog_shape_is_valid(state)
                || state.dimensions.len() != state.dimension_snapshot_staging.len()
            {
                return Err(LedgerEvidenceError::SnapshotShapeMismatch);
            }
            let bytes =
                u64::try_from(size_of::<Mutex<Option<LedgerState<P, D>>>>()).map_err(|_| {
                    LedgerEvidenceError::Capacity {
                        purpose: LedgerEvidencePurpose::SharedStorage,
                        cause: CapacityPlanError::CapacityOverflow,
                    }
                })?;
            let admitted = state
                .current_administrative_bytes
                .checked_add(bytes)
                .ok_or(LedgerEvidenceError::Capacity {
                    purpose: LedgerEvidencePurpose::SharedStorage,
                    cause: CapacityPlanError::CapacityOverflow,
                })?;
            if admitted > state.evidence_byte_limit {
                return Err(LedgerEvidenceError::LimitExceeded {
                    purpose: LedgerEvidencePurpose::SharedStorage,
                    value: admitted,
                    limit: state.evidence_byte_limit,
                });
            }
            let state = source
                .as_mut()
                .expect("promotion qualified the original source");
            state.current_administrative_bytes = admitted;
            state.peak_administrative_bytes = state.peak_administrative_bytes.max(admitted);
            let mut admission = PromotionAdmission { source, bytes };
            let mut candidate = Arc::new(Mutex::new(None));
            let vacant = Arc::get_mut(&mut candidate)
                .expect("an unissued shared candidate has one owner")
                .get_mut()
                .expect("an unissued shared candidate is unpoisoned");
            let state = admission
                .source
                .take()
                .expect("promotion qualified the owned source");
            admission.bytes = 0;
            *vacant = Some(state);
            drop(admission);
            *self = Self::Shared(candidate);
        }
        match self {
            Self::Shared(state) => Ok(state),
            Self::Inline(_) => unreachable!("promotion selects the installed shared owner"),
        }
    }
}

pub(super) struct StorageView<'storage, P, D> {
    pub(super) dimensions: &'storage [DimensionEntry<D>],
    pub(super) phase_states: &'storage [PhaseState],
    pub(super) header: &'storage LedgerHeader<P>,
}

pub(super) struct StorageViewMut<'storage, P, D> {
    pub(super) dimensions: &'storage mut [DimensionEntry<D>],
    pub(super) phase_states: &'storage mut [PhaseState],
    pub(super) header: &'storage mut LedgerHeader<P>,
}

impl<P, D> Deref for StorageView<'_, P, D> {
    type Target = LedgerHeader<P>;

    fn deref(&self) -> &Self::Target {
        self.header
    }
}

impl<P, D> Deref for StorageViewMut<'_, P, D> {
    type Target = LedgerHeader<P>;

    fn deref(&self) -> &Self::Target {
        self.header
    }
}

impl<P, D> DerefMut for StorageViewMut<'_, P, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.header
    }
}

impl<P, D> LedgerState<P, D> {
    pub(super) fn view(&self) -> StorageView<'_, P, D> {
        StorageView {
            dimensions: &self.dimensions,
            phase_states: &self.phase_states,
            header: &self.header,
        }
    }

    pub(super) fn view_mut(&mut self) -> StorageViewMut<'_, P, D> {
        StorageViewMut {
            dimensions: &mut self.dimensions,
            phase_states: &mut self.phase_states,
            header: &mut self.header,
        }
    }
}

impl<P, D> Deref for LedgerState<P, D> {
    type Target = LedgerHeader<P>;

    fn deref(&self) -> &Self::Target {
        &self.header
    }
}

impl<P, D> DerefMut for LedgerState<P, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.header
    }
}

pub(super) fn lock_storage<P, D>(
    state: &Mutex<Option<LedgerState<P, D>>>,
) -> StorageGuard<'_, P, D> {
    StorageGuard(
        state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}
