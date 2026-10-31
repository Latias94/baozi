//! The concrete operation policy for a direct, same-account Scene edit.

use std::num::{NonZeroU32, NonZeroU64};

use baozi_core::__scene_integration::integration::SceneEditBootstrap;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSessionPolicy, CanonicalMaterializationRole, CanonicalValidationRole,
    RetainedStoreWriterRole,
};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, RetainedStoreOperation, RetainedStoreReservation,
    RetainedStoreSharePlan, RetainedStoreStartPlan, RetainedStoreWriterPlan,
};
use baozi_operation::{
    CapacityPlan, DimensionPolicy, FinalProgress, LedgerLimits, RetainedStoreBatchLimits,
    RetainedStoreFailureTerminalWorkPlan, StoreDimensionKey, StoreDimensionSemanticId,
    StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan, StorePhaseKey, StorePhaseSemanticId,
    StoreRuntimeError, StoreSemanticCatalogId,
};

use super::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};

pub(crate) const MAX_WORK: u64 = 100_000_000;
pub(crate) const MAX_BYTES: u64 = 1024 * 1024 * 1024;
// `face_work_bound(n) = 32*n² + 128*n + 32` covers each polygon's triangulation and
// origin construction. Its retained remap values, `3*(n-2)` corner origins plus one face
// prefix, are below 1% of that bound for every n >= 3. One additional prefix starts at zero.
// This terminal allowance lets central rejection validate every success-evidence member after
// ordinary work has already been charged, without reserving unbounded work per input.
const MAX_REMAP_EVIDENCE_VALIDATION_WORK: u64 = MAX_WORK / 100 + 1;
pub(crate) const REJECTION_CODE: u32 = 0x5050_0001;
const CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-postproc-1", NonZeroU32::MIN);

#[derive(Clone, Copy)]
pub(crate) enum RuntimeDimension {
    Work = 1,
    ScratchBytes = 2,
    CanonicalBytes = 3,
    OutcomeEvidenceBytes = 4,
    FailureTerminalWork = 5,
    TerminalEvidenceRecords = 6,
}

pub(crate) type EditOperation = RetainedStoreOperation<
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    CanonicalInheritanceSource,
>;

#[derive(Clone, Copy)]
pub(crate) struct RuntimeKeys {
    pub(crate) planning: StorePhaseKey,
    pub(crate) validation: StorePhaseKey,
    pub(crate) publication: StorePhaseKey,
    pub(crate) work: StoreDimensionKey,
    pub(crate) scratch: StoreDimensionKey,
    pub(crate) canonical: StoreDimensionKey,
    pub(crate) evidence: StoreDimensionKey,
}

impl RuntimeKeys {
    pub(crate) fn session_policy(self) -> CanonicalActivationSessionPolicy {
        CanonicalActivationSessionPolicy::new(
            RetainedStoreWriterRole::new(self.planning, self.canonical, self.canonical),
            RetainedStoreWriterRole::new(self.publication, self.evidence, self.evidence),
            CanonicalMaterializationRole::new(self.work, self.scratch),
            CanonicalValidationRole::new(self.validation, self.work, self.scratch),
            self.canonical,
            self.evidence,
        )
    }
}

pub(crate) fn plan_runtime(
    writer_limits: RetainedStoreBatchLimits,
) -> Option<(RuntimeKeys, StoreLedgerPlan, RetainedStoreStartPlan)> {
    let domain = StoreKeyDomain::try_new(CATALOG).ok()?;
    let phase =
        |local| domain.try_admit_phase(StorePhaseSemanticId::new(CATALOG, NonZeroU64::new(local)?));
    let dimension = |kind| domain.try_admit_dimension(dimension_semantic(kind));
    let keys = RuntimeKeys {
        planning: phase(1)?,
        validation: phase(2)?,
        publication: phase(3)?,
        work: dimension(RuntimeDimension::Work)?,
        scratch: dimension(RuntimeDimension::ScratchBytes)?,
        canonical: dimension(RuntimeDimension::CanonicalBytes)?,
        evidence: dimension(RuntimeDimension::OutcomeEvidenceBytes)?,
    };
    let terminal = dimension(RuntimeDimension::FailureTerminalWork)?;
    let terminal_records = dimension(RuntimeDimension::TerminalEvidenceRecords)?;
    let validation_work =
        SceneEditBootstrap::failure_field_work().checked_add(MAX_REMAP_EVIDENCE_VALIDATION_WORK)?;
    let terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        terminal,
        SceneEditBootstrap::success_evidence_members(),
        SceneEditBootstrap::failure_field_work(),
        validation_work,
        MAX_WORK,
        MAX_WORK,
    )
    .ok()?;
    let limits = LedgerLimits::new(
        MAX_BYTES,
        [
            (
                keys.work,
                DimensionPolicy::counter(
                    terminal_work.ordinary_attempted_work_limit(),
                    terminal_work.ordinary_consumed_work_limit(),
                    0,
                ),
            ),
            (
                keys.scratch,
                DimensionPolicy::reservable_bytes(0, 0, 0, MAX_BYTES),
            ),
            (
                keys.canonical,
                DimensionPolicy::reservable_bytes(0, 0, MAX_BYTES, MAX_BYTES),
            ),
            (
                keys.evidence,
                DimensionPolicy::reservable_bytes(0, 0, 0, MAX_BYTES)
                    .with_published_outcome_evidence_limit(MAX_BYTES),
            ),
            (terminal, terminal_work.requirement().dimension_policy()),
            (
                terminal_records,
                DimensionPolicy::counter(0, 0, 0).with_published_outcome_evidence_limit(2),
            ),
        ],
    )
    .ok()?;
    let ledger = StoreLedgerPlan::try_new(
        domain,
        limits,
        &[keys.planning, keys.validation, keys.publication],
    )
    .ok()?;
    let start = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            keys.publication,
            keys.evidence,
            keys.evidence,
            writer_limits,
        ),
        RetainedStoreSharePlan::new(keys.work, keys.work),
        StoreFallbackEnvelope::new(REJECTION_CODE),
        terminal_records,
        1,
        terminal_records,
        1,
        terminal_work,
    );
    Some((keys, ledger, start))
}

pub(crate) const fn dimension_semantic(kind: RuntimeDimension) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        CATALOG,
        NonZeroU64::new(kind as u64).expect("the fixed runtime dimension is nonzero"),
    )
}

pub(crate) const fn final_progress()
-> FinalProgress<PostProcessProgressPhase, PostProcessProgressUnit> {
    FinalProgress::new(
        PostProcessProgressPhase::Publication,
        PostProcessProgressUnit::Scene,
        1,
    )
}

/// Executes one bounded action after admission and records completion only on normal return.
#[allow(
    clippy::result_large_err,
    reason = "propagating a runtime failure must not allocate after admission or cancellation fails"
)]
pub(crate) fn run_unit<R>(
    operation: &mut EditOperation,
    keys: RuntimeKeys,
    action: impl FnOnce() -> R,
) -> Result<R, StoreRuntimeError> {
    let mut host = operation.producer_host();
    let mut work = host.try_begin_observed_work([(keys.work, 1)])?;
    let value = action();
    if let Err(cause) = work.complete([1]) {
        work.finish_partial();
        return Err(cause);
    }
    work.finish()?;
    Ok(value)
}

/// A fixed Vec and its actual capacity charge; storage is always destroyed before its lease.
pub(crate) struct ScratchVec<T> {
    values: Option<Vec<T>>,
    reservation: Option<RetainedStoreReservation<PostProcessOwner>>,
}

impl<T> ScratchVec<T> {
    #[allow(
        clippy::result_large_err,
        reason = "allocation failure propagates inline without a second allocation"
    )]
    pub(crate) fn new(
        operation: &mut EditOperation,
        keys: RuntimeKeys,
        count: usize,
    ) -> Result<Self, StoreRuntimeError> {
        let plan = CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let mut reservation = operation.reserve_retained_storage(
            keys.planning,
            keys.scratch,
            bytes,
            u64::from(bytes != 0),
        )?;
        let mut values = Vec::new();
        if values
            .try_reserve_exact(plan.allocator_additional())
            .is_err()
        {
            reservation.release_storage(values);
            return Err(StoreRuntimeError::AllocationFailed);
        }
        let actual = match plan.finish(values.len(), values.capacity()) {
            Ok(bytes) => bytes,
            Err(cause) => {
                reservation.release_storage(values);
                return Err(StoreRuntimeError::Plan(cause.into()));
            }
        };
        if let Err(cause) = operation
            .control_checkpoint()
            .resize(&mut reservation, actual)
        {
            reservation.release_storage(values);
            return Err(cause);
        }
        Ok(Self {
            values: Some(values),
            reservation: Some(reservation),
        })
    }

    pub(crate) fn values(&self) -> &Vec<T> {
        self.values
            .as_ref()
            .expect("a live scratch owner has storage")
    }

    pub(crate) fn values_mut(&mut self) -> &mut Vec<T> {
        self.values
            .as_mut()
            .expect("a live scratch owner has storage")
    }
}

impl<T> Drop for ScratchVec<T> {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            reservation.release_storage(self.values.take());
        }
    }
}
