//! Admission adapters for the actual private node/geometry table owners.

#[cfg(test)]
use super::NodeSceneBuildFaults;
use crate::canonical_scene::store::production::{ValidatedGeometryBatch, ValidatedNodeBatch};
use crate::canonical_scene::store::typed_storage::build::{
    GeometryStoragePlan, NodeStoragePlan, TypedStorageBuildControl, TypedStorageBuildError,
};
use crate::canonical_scene::store::typed_storage::{GeometryStorage, NodeStorage};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalLane, CanonicalRetainedStorageBuildControl,
};
use baozi_operation::__retained_store_integration::authority::SharedRetainedStorage as RetainedStorage;
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};

impl<O, PP, U> TypedStorageBuildControl for CanonicalRetainedStorageBuildControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = StoreRuntimeError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        CanonicalRetainedStorageBuildControl::run_unit(self, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        CanonicalRetainedStorageBuildControl::run_string_unit(self, action)
    }
}

pub(super) fn retain_nodes<O, PP, U, L, R>(
    lane: &mut CanonicalLane<'_, O, PP, U, L>,
    plan: NodeStoragePlan,
    batch: &ValidatedNodeBatch<'_, R>,
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
) -> Result<RetainedStorage<NodeStorage>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_storage(
        plan.capacities(),
        plan.work().map_err(StoreRuntimeError::Plan)?,
        plan.name_bytes() as u64,
        |control| {
            #[allow(unused_mut)]
            let mut storage = plan.build(batch, control).map_err(map_build)?;
            #[cfg(test)]
            if faults.alter_first_authored_transform
                && let Some(node) = storage.nodes.get_mut(1)
            {
                match &mut node.transform {
                    crate::canonical_scene::store::typed_storage::StoredTransform::Matrix(
                        matrix,
                    ) => matrix.cols[0][0] = f32::NAN,
                    crate::canonical_scene::store::typed_storage::StoredTransform::Trs {
                        translation,
                        ..
                    } => translation.x = f32::NAN,
                    _ => {}
                }
                faults.alter_first_authored_transform = false;
            }
            #[cfg(test)]
            if let Some(fault) = faults.spatial {
                fault.corrupt_nodes(&mut storage);
            }
            Ok(storage)
        },
    )
}

pub(super) fn retain_geometry<O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'_, O, PP, U, L>,
    plan: GeometryStoragePlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
) -> Result<RetainedStorage<GeometryStorage>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_storage(
        plan.capacities(),
        plan.work().map_err(StoreRuntimeError::Plan)?,
        plan.name_bytes() as u64,
        |control| {
            #[allow(unused_mut)]
            let mut storage = plan.build(batch, nodes, control).map_err(map_build)?;
            #[cfg(test)]
            if faults.alter_first_geometry_f32
                && let Some(value) = storage.f32_values.first_mut()
            {
                *value = f32::NAN;
                faults.alter_first_geometry_f32 = false;
            }
            Ok(storage)
        },
    )
}

pub(super) fn map_build(error: TypedStorageBuildError<StoreRuntimeError>) -> StoreRuntimeError {
    match error {
        TypedStorageBuildError::AllocationFailed => StoreRuntimeError::BatchAllocationFailed,
        TypedStorageBuildError::Capacity(cause) => StoreRuntimeError::Plan(cause.into()),
        TypedStorageBuildError::Invariant => StoreRuntimeError::ConstructionInvariantUnverified,
        TypedStorageBuildError::Control(cause) => cause,
    }
}
