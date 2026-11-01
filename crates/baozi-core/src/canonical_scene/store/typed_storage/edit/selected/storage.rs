//! Unique metered storage for one selection's immutable mapping rows.

use super::{EditResult, LocalMapping, TypedStorageBuildControl, TypedStorageBuildError};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, RetainedStoreOperation, RetainedStoreReservation,
};
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, StoreDimensionKey, StorePhaseKey,
    StoreRuntimeError,
};

/// One invocation owns this scratch until candidate correspondence and evidence are complete.
/// Destroy it before terminal preparation. Shared replacements never contain plan state.
pub struct SceneGeometryPlanStorage<O: OperationOwner> {
    mappings: Option<Vec<LocalMapping>>,
    reservation: Option<RetainedStoreReservation<O>>,
}

/// Exclusive initialization capability. A sealed plan borrows this owner's rows immutably,
/// preventing their reuse while any plan, profile selector, or postflight view remains live.
pub struct SceneGeometryPlanScratch<'a> {
    rows: MappingRows<'a>,
}

enum MappingRows<'a> {
    Reserved(&'a mut Vec<LocalMapping>),
    #[cfg(test)]
    Single(&'a mut [LocalMapping; 1]),
}

impl<'a> SceneGeometryPlanScratch<'a> {
    pub(super) fn initialize<C: TypedStorageBuildControl>(
        self,
        count: usize,
        control: &mut C,
    ) -> EditResult<&'a mut [LocalMapping], C::Error> {
        match self.rows {
            MappingRows::Reserved(rows) => {
                if rows.capacity() < count {
                    return Err(TypedStorageBuildError::Invariant);
                }
                rows.clear();
                for _ in 0..count {
                    control
                        .run_unit(|| rows.push(LocalMapping::EMPTY))
                        .map_err(TypedStorageBuildError::Control)?;
                }
                Ok(rows.as_mut_slice())
            }
            #[cfg(test)]
            MappingRows::Single(rows) if count == 1 => {
                control
                    .run_unit(|| rows[0] = LocalMapping::EMPTY)
                    .map_err(TypedStorageBuildError::Control)?;
                Ok(rows.as_mut_slice())
            }
            #[cfg(test)]
            _ => Err(TypedStorageBuildError::Invariant),
        }
    }
}

impl<O: OperationOwner> SceneGeometryPlanStorage<O> {
    #[allow(
        clippy::result_large_err,
        reason = "rejection retains the existing inline operation failure"
    )]
    pub fn try_new<PP: ProgressPhase, U: ProgressUnit>(
        operation: &mut RetainedStoreOperation<O, PP, U, CanonicalInheritanceSource>,
        phase: StorePhaseKey,
        scratch_dimension: StoreDimensionKey,
        count: usize,
    ) -> Result<Self, StoreRuntimeError> {
        let plan = CapacityPlan::for_type::<LocalMapping>(0, 0, count)
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let mut reservation = operation.reserve_retained_storage(
            phase,
            scratch_dimension,
            bytes,
            u64::from(bytes != 0),
        )?;
        let mut mappings = Vec::new();
        if mappings
            .try_reserve_exact(plan.allocator_additional())
            .is_err()
        {
            reservation.release_storage(mappings);
            return Err(StoreRuntimeError::AllocationFailed);
        }
        let actual = match plan.finish(0, mappings.capacity()) {
            Ok(actual) => actual,
            Err(cause) => {
                reservation.release_storage(mappings);
                return Err(StoreRuntimeError::Plan(cause.into()));
            }
        };
        if let Err(cause) = operation
            .control_checkpoint()
            .resize(&mut reservation, actual)
        {
            reservation.release_storage(mappings);
            return Err(cause);
        }
        Ok(Self {
            mappings: Some(mappings),
            reservation: Some(reservation),
        })
    }

    pub fn scratch(&mut self) -> SceneGeometryPlanScratch<'_> {
        SceneGeometryPlanScratch {
            rows: MappingRows::Reserved(
                self.mappings
                    .as_mut()
                    .expect("live selection mapping owner"),
            ),
        }
    }
}

impl<O: OperationOwner> Drop for SceneGeometryPlanStorage<O> {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            reservation.release_storage(self.mappings.take());
        }
    }
}

#[cfg(test)]
impl<'a> SceneGeometryPlanScratch<'a> {
    pub(in crate::canonical_scene::store) fn for_test(mappings: &'a mut Vec<LocalMapping>) -> Self {
        Self {
            rows: MappingRows::Reserved(mappings),
        }
    }
    pub(in crate::canonical_scene::store) fn single_for_test(
        mapping: &'a mut [LocalMapping; 1],
    ) -> Self {
        Self {
            rows: MappingRows::Single(mapping),
        }
    }
}
