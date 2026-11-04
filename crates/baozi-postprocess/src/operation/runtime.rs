//! The concrete operation policy for a direct, same-account Scene edit.

use std::num::{NonZeroU32, NonZeroU64};

use baozi_core::__scene_integration::integration::{
    SceneCanonicalRootTag, SceneEditBootstrap, SceneEditEvidenceTag, SceneEditGenerationRejection,
    SceneEditOutput, SceneEditRejection, SceneEditValidationProof,
};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSessionPolicy, CanonicalBodyOperation, CanonicalBoundInvocationFinish,
    CanonicalDispositionSlot, CanonicalInvocationFinish, CanonicalInvocationHost,
    CanonicalMaterializationRole, CanonicalValidationRole, RetainedStoreWriterRole,
    StoreActionExecution,
};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, RetainedStoreReservation, RetainedStoreSharePlan,
    RetainedStoreStartPlan, RetainedStoreWriterPlan,
};
use baozi_operation::{
    CapacityPlan, DimensionPolicy, FinalProgress, LedgerLimits, RecordBlock,
    RetainedStoreBatchLimits, RetainedStoreFailureTerminalWorkPlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan,
    StorePhaseKey, StorePhaseSemanticId, StoreRuntimeError, StoreSemanticCatalogId,
};

use super::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};

pub(crate) const MAX_WORK: u64 = 100_000_000;
pub(crate) const MAX_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const REJECTION_CODE: u32 = 0x5050_0001;
const CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-postproc-2", NonZeroU32::MIN);

pub(crate) const WORK_ACTION_CONTRACT: &str = "bounded-normal-actions/v2";

#[derive(Clone, Copy)]
pub(crate) enum RuntimeDimension {
    Work = 1,
    ScratchBytes = 2,
    CanonicalBytes = 3,
    OutcomeEvidenceBytes = 4,
    FailureTerminalWork = 5,
    TerminalEvidenceRecords = 6,
}

pub(crate) type EditHost = CanonicalInvocationHost<
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditValidationProof,
    SceneEditRejection,
    SceneEditRejection,
    SceneEditGenerationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;

pub(crate) type EditDisposition<'inv> = CanonicalDispositionSlot<
    'inv,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;

pub(crate) type EditFinish<'inv> = CanonicalInvocationFinish<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditValidationProof,
    SceneEditRejection,
    SceneEditRejection,
    SceneEditGenerationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;

pub(crate) type EditBoundFinish<'inv, 'account> = CanonicalBoundInvocationFinish<
    'inv,
    'account,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditValidationProof,
    SceneEditRejection,
    SceneEditRejection,
    SceneEditGenerationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;

pub(crate) type EditOperation<'inv> = CanonicalBodyOperation<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    CanonicalInheritanceSource,
>;

#[cfg(test)]
pub(crate) fn with_test_operation<T>(
    scene: &baozi_core::ValidatedScene,
    control: crate::PostProcessControl,
    mut body: impl for<'inv> FnMut(&mut EditOperation<'inv>, RuntimeKeys) -> T,
) -> (
    baozi_operation::__retained_store_integration::activation::CanonicalOperationPublication,
    T,
) {
    use baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome;

    let (writer, _, _) = SceneEditBootstrap::try_new().unwrap().into_parts();
    let (keys, ledger, plan) =
        plan_selected_runtime(writer, crate::PostProcessBudgets::default()).unwrap();
    let mut inputs = Some((control, ledger, plan));
    let mut result = None;
    let result_slot = &mut result;
    let mut host = EditHost::new();
    let finish = host
        .run_body(move |parts, disposition| {
            let (control, ledger, plan) = inputs.take().expect("one test operation body");
            let mut operation =
                baozi_core::__scene_integration::integration::try_start_scene_edit_operation::<
                    PostProcessOwner,
                    _,
                    _,
                >(parts.startup, scene, control.into_runtime(), ledger, plan)
                .unwrap();
            *result_slot = Some(body(&mut operation, keys));
            operation.request_unchanged(disposition).unwrap();
        })
        .unwrap();
    let CanonicalInvocationOutcome::Operation(operation) = finish.resume().outcome else {
        panic!("the bounded execution fixture must restore its actual operation");
    };
    (
        operation.finish(final_progress()).unwrap(),
        result.expect("the test operation body must run once"),
    )
}

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
    // The whole-Scene transaction uses the common four-slot publication table, while its
    // two-member evidence has a fixed field count and needs no input-derived allowance.
    plan_runtime_with_members(
        writer_limits,
        crate::PostProcessBudgets::default(),
        SceneEditBootstrap::success_evidence_members(),
    )
}

pub(crate) fn plan_selected_runtime(
    writer_limits: RetainedStoreBatchLimits,
    budgets: crate::PostProcessBudgets,
) -> Option<(RuntimeKeys, StoreLedgerPlan, RetainedStoreStartPlan)> {
    plan_runtime_with_members(
        writer_limits,
        budgets,
        SceneEditBootstrap::failure_evidence_members(),
    )
}

fn plan_runtime_with_members(
    writer_limits: RetainedStoreBatchLimits,
    budgets: crate::PostProcessBudgets,
    max_evidence_members: usize,
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
    let terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        terminal,
        max_evidence_members,
        SceneEditBootstrap::failure_field_work(),
        SceneEditBootstrap::failure_field_work(),
        budgets.max_work_actions,
        budgets.max_work_actions,
    )
    .ok()?;
    let limits = LedgerLimits::new(
        budgets.storage_bytes,
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
                DimensionPolicy::reservable_bytes(0, 0, 0, budgets.storage_bytes),
            ),
            (
                keys.canonical,
                DimensionPolicy::reservable_bytes(
                    0,
                    0,
                    budgets.storage_bytes,
                    budgets.storage_bytes,
                ),
            ),
            (
                keys.evidence,
                DimensionPolicy::reservable_bytes(0, 0, 0, budgets.storage_bytes)
                    .with_published_outcome_evidence_limit(budgets.storage_bytes),
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

/// One concrete execution region; all borrowed roles end before consuming the edit owner.
pub(crate) type ActiveRuntime<'op> =
    StoreActionExecution<'op, PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit>;

pub(crate) fn with_execution<R, E: From<StoreRuntimeError>>(
    operation: &mut EditOperation<'_>,
    keys: RuntimeKeys,
    action: impl FnOnce(&mut ActiveRuntime<'_>) -> Result<R, E>,
) -> Result<R, E> {
    operation.with_action_execution(keys.work, action)
}

/// Accounts the actual bounded leaf; control cadence is independent from numeric admission.
#[allow(
    clippy::result_large_err,
    reason = "action failures retain exact inline accounting and control causes"
)]
pub(crate) fn run_unit<R>(
    operation: &mut ActiveRuntime<'_>,
    action: impl FnOnce() -> R,
) -> Result<R, StoreRuntimeError> {
    operation.begin_action(1)?;
    let value = action();
    operation.complete_action()?;
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
        operation: &mut ActiveRuntime<'_>,
        keys: RuntimeKeys,
        count: usize,
    ) -> Result<Self, StoreRuntimeError> {
        let plan = CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let mut reservation = operation.reserve_retained(
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
        if let Err(cause) = operation.resize(&mut reservation, actual) {
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
            #[cfg(test)]
            let released = self
                .values
                .as_ref()
                .map(|values| (values.as_ptr() as usize, values.len(), values.capacity()));
            reservation.release_storage(self.values.take());
            #[cfg(test)]
            if let Some((address, len, capacity)) = released {
                crate::prepare::cleanup_tests::record(
                    crate::prepare::cleanup_tests::Event::ScratchDropped {
                        address,
                        len,
                        capacity,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "runtime/execution_tests.rs"]
mod execution_tests;
