use baozi_operation::__retained_store_integration::activation::{
    CanonicalLane, CanonicalRecordBuildRow, CanonicalReservation,
};
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan,
    ScalarSequence, SequencePlan, StoreBuildError, StoreControlledError, StorePlanError,
    StoreRuntimeError, Utf8Block, Utf8Plan,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryAttributePayloadKind, GeometryAttributeScalarKind,
    GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch, ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::appearance::AppearanceMaterialTableSchema;
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    GEOMETRY_MESH_INSTANCE_FIELD_COUNT, GEOMETRY_NORMALIZATION_NONE, GEOMETRY_PAYLOAD_DENSE,
    GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_PRIMITIVE_FIELD_COUNT, GEOMETRY_SCALAR_F32,
    GEOMETRY_SCALAR_I32, GEOMETRY_SCALAR_U32, GeometryAttributeSemanticNameSchema,
    GeometryAttributeTableSchema, GeometryF32ValuesSchema, GeometryI32ValuesSchema,
    GeometryLogicalMeshNameSchema, GeometryLogicalMeshTableSchema, GeometryMeshInstanceTableSchema,
    GeometryPolygonBoundariesSchema, GeometryPrimitiveIndicesSchema, GeometryPrimitiveTableSchema,
    GeometrySparseF32DefaultsSchema, GeometrySparseI32DefaultsSchema, GeometrySparseIndicesSchema,
    GeometrySparseU32DefaultsSchema, GeometryU32ValuesSchema, encode_geometry_domain,
    encode_geometry_semantic, encode_geometry_topology,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT, SCENE_GEOMETRY_SCHEMA_REVISION,
    SceneGeometryDirectorySchema,
};

#[cfg(test)]
use super::NodeSceneBuildFaults;
use super::{
    checked_validation_work_sum, optional_record_validation_work, optional_utf8_validation_work,
    record_validation_work,
};

pub(super) struct GeometryMaterializationPlans {
    logical_mesh_names: Option<Utf8Plan>,
    attribute_semantic_names: Option<Utf8Plan>,
    logical_meshes: Option<RecordPlan>,
    primitives: Option<RecordPlan>,
    attributes: Option<RecordPlan>,
    mesh_instances: Option<RecordPlan>,
    directory: RecordPlan,
    logical_mesh_count: u32,
    logical_mesh_name_byte_count: u32,
    attribute_semantic_name_byte_count: u32,
    primitive_count: u32,
    attribute_count: u32,
    mesh_instance_count: u32,
    f32_scalar_count: u64,
    u32_scalar_count: u64,
    i32_scalar_count: u64,
    payload_block_count: u32,
    canonical_block_count: usize,
}

impl GeometryMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedGeometryBatch<'_, R>) -> Result<Self, StorePlanError> {
        let logical_mesh_count = batch.logical_mesh_count();
        let logical_mesh_name_byte_count = batch.logical_mesh_name_byte_count();
        let attribute_semantic_name_byte_count = batch.attribute_semantic_name_byte_count();
        let primitive_count = batch.primitive_count();
        let attribute_count = batch.attribute_count();
        let mesh_instance_count = batch.mesh_instance_count();
        let f32_scalar_count = batch.f32_scalar_count();
        let u32_scalar_count = batch.u32_scalar_count();
        let i32_scalar_count = batch.i32_scalar_count();
        let logical_meshes =
            record_plan_if_nonempty(logical_mesh_count, GEOMETRY_LOGICAL_MESH_FIELD_COUNT)?;
        let logical_mesh_names = utf8_plan_if_nonempty(logical_mesh_name_byte_count)?;
        let attribute_semantic_names = utf8_plan_if_nonempty(attribute_semantic_name_byte_count)?;
        let primitives = record_plan_if_nonempty(primitive_count, GEOMETRY_PRIMITIVE_FIELD_COUNT)?;
        let attributes = record_plan_if_nonempty(attribute_count, GEOMETRY_ATTRIBUTE_FIELD_COUNT)?;
        let mesh_instances =
            record_plan_if_nonempty(mesh_instance_count, GEOMETRY_MESH_INSTANCE_FIELD_COUNT)?;
        let table_block_count = [
            logical_meshes.is_some(),
            primitives.is_some(),
            attributes.is_some(),
            mesh_instances.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let payload_block_count = batch.payload_block_count();
        if (logical_mesh_name_byte_count != 0) != logical_mesh_names.is_some() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        if (attribute_semantic_name_byte_count != 0) != attribute_semantic_names.is_some() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let canonical_block_count = 1_usize
            .checked_add(table_block_count)
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()
                    .and_then(|payload| count.checked_add(payload))
            })
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            logical_mesh_names,
            attribute_semantic_names,
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            directory: RecordPlan::new(1, SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT)?,
            logical_mesh_count,
            logical_mesh_name_byte_count,
            attribute_semantic_name_byte_count,
            primitive_count,
            attribute_count,
            mesh_instance_count,
            f32_scalar_count,
            u32_scalar_count,
            i32_scalar_count,
            payload_block_count,
            canonical_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn logical_mesh_count(&self) -> u32 {
        self.logical_mesh_count
    }

    pub(super) const fn logical_mesh_name_byte_count(&self) -> u32 {
        self.logical_mesh_name_byte_count
    }

    pub(super) const fn attribute_semantic_name_byte_count(&self) -> u32 {
        self.attribute_semantic_name_byte_count
    }

    pub(super) const fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    pub(super) const fn attribute_count(&self) -> u32 {
        self.attribute_count
    }

    pub(super) const fn mesh_instance_count(&self) -> u32 {
        self.mesh_instance_count
    }

    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(super) fn validation_work_units(&self) -> Result<u64, StorePlanError> {
        checked_validation_work_sum([
            self.f32_scalar_count,
            self.u32_scalar_count,
            self.i32_scalar_count,
            optional_utf8_validation_work(self.logical_mesh_names)?,
            optional_utf8_validation_work(self.attribute_semantic_names)?,
            optional_record_validation_work(self.logical_meshes)?,
            optional_record_validation_work(self.primitives)?,
            optional_record_validation_work(self.attributes)?,
            optional_record_validation_work(self.mesh_instances)?,
            record_validation_work(self.directory)?,
        ])
    }
}

#[derive(Clone, Copy)]
struct PrimitiveLeafRefs {
    polygon_boundaries: Option<CanonicalBlockRef<GeometryPolygonBoundariesSchema>>,
    indices: Option<CanonicalBlockRef<GeometryPrimitiveIndicesSchema>>,
}

impl PrimitiveLeafRefs {
    const EMPTY: Self = Self {
        polygon_boundaries: None,
        indices: None,
    };
}

#[derive(Clone, Copy)]
enum AttributeValuesRef {
    Absent,
    F32(CanonicalBlockRef<GeometryF32ValuesSchema>),
    U32(CanonicalBlockRef<GeometryU32ValuesSchema>),
    I32(CanonicalBlockRef<GeometryI32ValuesSchema>),
}

#[derive(Clone, Copy)]
enum AttributeDefaultRef {
    Absent,
    F32(CanonicalBlockRef<GeometrySparseF32DefaultsSchema>),
    U32(CanonicalBlockRef<GeometrySparseU32DefaultsSchema>),
    I32(CanonicalBlockRef<GeometrySparseI32DefaultsSchema>),
}

#[derive(Clone, Copy)]
struct AttributeLeafRefs {
    values: AttributeValuesRef,
    sparse_indices: Option<CanonicalBlockRef<GeometrySparseIndicesSchema>>,
    sparse_default: AttributeDefaultRef,
}

impl AttributeLeafRefs {
    const EMPTY: Self = Self {
        values: AttributeValuesRef::Absent,
        sparse_indices: None,
        sparse_default: AttributeDefaultRef::Absent,
    };
}

pub(super) struct GeometryReferenceScratch<'lane, O>
where
    O: OperationOwner,
{
    primitive_refs: Vec<PrimitiveLeafRefs>,
    attribute_refs: Vec<AttributeLeafRefs>,
    _reservation: CanonicalReservation<'lane, O>,
}

#[derive(Clone, Copy)]
pub(super) struct GeometryReferenceScratchPlan {
    primitive_plan: CapacityPlan,
    attribute_plan: CapacityPlan,
    primitive_count: usize,
    attribute_count: usize,
    envelope: u64,
}

impl GeometryReferenceScratchPlan {
    pub(super) fn try_new<R>(
        batch: &ValidatedGeometryBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let primitive_count =
            usize::try_from(batch.primitive_count()).map_err(|_| StorePlanError::SizeOverflow)?;
        let attribute_count =
            usize::try_from(batch.attribute_count()).map_err(|_| StorePlanError::SizeOverflow)?;
        let primitive_plan = CapacityPlan::for_type::<PrimitiveLeafRefs>(0, 0, primitive_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let attribute_plan = CapacityPlan::for_type::<AttributeLeafRefs>(0, 0, attribute_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let envelope = primitive_plan
            .envelope_delta_bytes()
            .ok()
            .and_then(|first| {
                attribute_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|second| first.checked_add(second))
            })
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            primitive_plan,
            attribute_plan,
            primitive_count,
            attribute_count,
            envelope,
        })
    }
}

impl<'lane, O> GeometryReferenceScratch<'lane, O>
where
    O: OperationOwner,
{
    pub(super) fn try_new<PP, U, L>(
        lane: &mut CanonicalLane<'lane, O, PP, U, L>,
        plan: GeometryReferenceScratchPlan,
    ) -> Result<Self, StoreRuntimeError>
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let GeometryReferenceScratchPlan {
            primitive_plan,
            attribute_plan,
            primitive_count,
            attribute_count,
            envelope,
        } = plan;
        let mut checkpoint = lane.control_checkpoint()?;
        let mut reservation = checkpoint.reserve(envelope)?;
        checkpoint.validate_live_owner(&reservation)?;

        primitive_plan
            .verify_preflight(0, 0)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let mut primitive_refs = Vec::new();
        primitive_refs
            .try_reserve_exact(primitive_plan.allocator_additional())
            .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
        attribute_plan
            .verify_preflight(0, 0)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let mut attribute_refs = Vec::new();
        attribute_refs
            .try_reserve_exact(attribute_plan.allocator_additional())
            .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
        let actual = primitive_plan
            .finish(primitive_refs.len(), primitive_refs.capacity())
            .ok()
            .and_then(|first| {
                attribute_plan
                    .finish(attribute_refs.len(), attribute_refs.capacity())
                    .ok()
                    .and_then(|second| first.checked_add(second))
            })
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        checkpoint.resize(&mut reservation, actual)?;

        fill_reference_scratch(
            &mut primitive_refs,
            primitive_count,
            PrimitiveLeafRefs::EMPTY,
            || checkpoint.poll(),
        )?;
        fill_reference_scratch(
            &mut attribute_refs,
            attribute_count,
            AttributeLeafRefs::EMPTY,
            || checkpoint.poll(),
        )?;
        checkpoint.poll()?;
        Ok(Self {
            primitive_refs,
            attribute_refs,
            _reservation: reservation,
        })
    }
}

fn fill_reference_scratch<T: Copy>(
    values: &mut Vec<T>,
    count: usize,
    value: T,
    mut poll: impl FnMut() -> Result<(), StoreRuntimeError>,
) -> Result<(), StoreRuntimeError> {
    let poll_interval = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
    for index in 0..count {
        if index.is_multiple_of(poll_interval) {
            poll()?;
        }
        values.push(value);
    }
    Ok(())
}

fn record_plan_if_nonempty(
    count: u32,
    fields: usize,
) -> Result<Option<RecordPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(RecordPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
            fields,
        )?))
    }
}

fn utf8_plan_if_nonempty(count: u32) -> Result<Option<Utf8Plan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(Utf8Plan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryTableRefs {
    pub(in crate::canonical_scene::store) logical_meshes:
        Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    pub(in crate::canonical_scene::store) primitives:
        Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    pub(in crate::canonical_scene::store) attributes:
        Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    pub(in crate::canonical_scene::store) mesh_instances:
        Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryPayloadRefs {
    pub(in crate::canonical_scene::store) logical_mesh_names:
        Option<CanonicalBlockRef<GeometryLogicalMeshNameSchema>>,
    pub(in crate::canonical_scene::store) attribute_semantic_names:
        Option<CanonicalBlockRef<GeometryAttributeSemanticNameSchema>>,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedGeometryRefs {
    pub(in crate::canonical_scene::store) directory:
        CanonicalBlockRef<SceneGeometryDirectorySchema>,
    pub(in crate::canonical_scene::store) tables: GeometryTableRefs,
    pub(in crate::canonical_scene::store) payloads: GeometryPayloadRefs,
}

impl RetainedGeometryRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<SceneGeometryDirectorySchema> {
        self.directory
    }

    pub(super) const fn logical_meshes(
        self,
    ) -> Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>> {
        self.tables.logical_meshes
    }

    pub(super) const fn primitives(
        self,
    ) -> Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>> {
        self.tables.primitives
    }

    pub(super) const fn logical_mesh_names(
        self,
    ) -> Option<CanonicalBlockRef<GeometryLogicalMeshNameSchema>> {
        self.payloads.logical_mesh_names
    }

    pub(super) const fn attribute_semantic_names(
        self,
    ) -> Option<CanonicalBlockRef<GeometryAttributeSemanticNameSchema>> {
        self.payloads.attribute_semantic_names
    }

    pub(super) const fn mesh_instances(
        self,
    ) -> Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>> {
        self.tables.mesh_instances
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn retain_geometry<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &GeometryMaterializationPlans,
    mut scratch: GeometryReferenceScratch<'lane, O>,
    batch: &ValidatedGeometryBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    node_table: CanonicalBlockRef<super::super::super::schema::NodeSceneNodeTableSchema>,
    appearance_materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<RetainedGeometryRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let logical_mesh_names = match plans.logical_mesh_names {
        Some(plan) => Some(retain_logical_mesh_names(lane, plan, batch, failure)?),
        None => None,
    };
    let attribute_semantic_names = match plans.attribute_semantic_names {
        Some(plan) => Some(retain_attribute_semantic_names(lane, plan, batch, failure)?),
        None => None,
    };
    let primitive_payload_blocks =
        retain_primitive_leaves(lane, batch, &mut scratch.primitive_refs, failure)?;
    let attribute_payload_blocks = retain_attribute_leaves(
        lane,
        batch,
        &mut scratch.attribute_refs,
        #[cfg(test)]
        faults,
        failure,
    )?;
    let payload_block_count = primitive_payload_blocks
        .checked_add(attribute_payload_blocks)
        .and_then(|count| count.checked_add(u32::from(logical_mesh_names.is_some())))
        .and_then(|count| count.checked_add(u32::from(attribute_semantic_names.is_some())))
        .ok_or(StoreRuntimeError::Build(failure))?;
    if payload_block_count != batch.payload_block_count() {
        return Err(StoreRuntimeError::Build(failure));
    }

    let attributes = match plans.attributes {
        Some(plan) => Some(retain_attribute_table(
            lane,
            plan,
            batch,
            &scratch.attribute_refs,
            attribute_semantic_names,
            failure,
        )?),
        None => None,
    };
    let primitives = match plans.primitives {
        Some(plan) => Some(retain_primitive_table(
            lane,
            plan,
            batch,
            &scratch.primitive_refs,
            appearance_materials,
            attributes,
            failure,
        )?),
        None => None,
    };
    let logical_meshes = match plans.logical_meshes {
        Some(plan) => Some(retain_logical_mesh_table(
            lane,
            plan,
            batch,
            primitives,
            logical_mesh_names,
            failure,
        )?),
        None => None,
    };
    let mesh_instances = match plans.mesh_instances {
        Some(plan) => Some(retain_mesh_instance_table(
            lane,
            plan,
            batch,
            nodes,
            node_table,
            logical_meshes,
            failure,
        )?),
        None => None,
    };
    let refs = GeometryTableRefs {
        logical_meshes,
        primitives,
        attributes,
        mesh_instances,
    };
    let payloads = GeometryPayloadRefs {
        logical_mesh_names,
        attribute_semantic_names,
    };
    let directory = retain_geometry_directory(lane, plans, refs, payloads, failure)?;
    Ok(RetainedGeometryRefs {
        directory,
        tables: refs,
        payloads,
    })
}

fn retain_logical_mesh_names<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    batch: &ValidatedGeometryBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryLogicalMeshNameSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_utf8(plan, GeometryLogicalMeshNameSchema, |control| {
        let mut expected_offset = 0_u32;
        for logical_mesh in batch.input().logical_meshes() {
            match logical_mesh.name() {
                Presence::Absent => {}
                Presence::Present(name) => {
                    let count = u32::try_from(name.len()).map_err(|_| failure)?;
                    control.write_str(name)?;
                    expected_offset = expected_offset.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if expected_offset != batch.logical_mesh_name_byte_count() {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_attribute_semantic_names<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    batch: &ValidatedGeometryBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryAttributeSemanticNameSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_utf8(plan, GeometryAttributeSemanticNameSchema, |control| {
        let mut expected_offset = 0_u32;
        for attribute_index in 0..usize::try_from(batch.attribute_count()).map_err(|_| failure)? {
            let attribute = batch.attribute(attribute_index).ok_or(failure)?;
            if let crate::attribute::AttributeSemantic::Custom(custom) = attribute.semantic() {
                let name = custom.as_str();
                let count = u32::try_from(name.len()).map_err(|_| failure)?;
                if attribute.semantic_name_offset() != expected_offset
                    || attribute.semantic_name_count() != count
                {
                    return Err(failure.into());
                }
                control.write_str(name)?;
                expected_offset = expected_offset.checked_add(count).ok_or(failure)?;
            } else if attribute.semantic_name_offset() != expected_offset
                || attribute.semantic_name_count() != 0
            {
                return Err(failure.into());
            }
        }
        if expected_offset != batch.attribute_semantic_name_byte_count() {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_primitive_leaves<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &mut [PrimitiveLeafRefs],
    failure: StoreBuildError,
) -> Result<u32, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut payload_block_count = 0_u32;
    for primitive_index in 0..refs.len() {
        let primitive = batch
            .primitive(primitive_index)
            .ok_or(StoreRuntimeError::Build(failure))?;
        let target = refs
            .get_mut(primitive_index)
            .ok_or(StoreRuntimeError::Build(failure))?;
        if let GeometryTopologyAuthoring::Polygons(boundaries) = primitive.authored_topology()
            && !boundaries.is_empty()
        {
            target.polygon_boundaries = Some(retain_u32_sequence(
                lane,
                boundaries,
                GeometryPolygonBoundariesSchema,
            )?);
            record_payload_block(&mut payload_block_count, failure)?;
        }
        if let Presence::Present(indices) = primitive.authored_indices()
            && !indices.is_empty()
        {
            target.indices = Some(retain_u32_sequence(
                lane,
                indices,
                GeometryPrimitiveIndicesSchema,
            )?);
            record_payload_block(&mut payload_block_count, failure)?;
        }
    }
    Ok(payload_block_count)
}

fn retain_attribute_leaves<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &mut [AttributeLeafRefs],
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<u32, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut payload_block_count = 0_u32;
    for attribute_index in 0..refs.len() {
        let attribute = batch
            .attribute(attribute_index)
            .ok_or(StoreRuntimeError::Build(failure))?;
        let target = refs
            .get_mut(attribute_index)
            .ok_or(StoreRuntimeError::Build(failure))?;
        match attribute.authored_payload() {
            GeometryAttributePayloadAuthoring::DenseF32 {
                lanes,
                source_values,
                source_indices,
            } => match source_indices {
                Presence::Absent if !source_values.is_empty() => {
                    target.values = AttributeValuesRef::F32(retain_f32_sequence(
                        lane,
                        source_values,
                        GeometryF32ValuesSchema,
                        #[cfg(test)]
                        faults,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                Presence::Present(indices) if !indices.is_empty() => {
                    target.values = AttributeValuesRef::F32(retain_projected_f32_sequence(
                        lane,
                        source_values,
                        indices,
                        lanes,
                        attribute.canonical_scalar_count(),
                        #[cfg(test)]
                        faults,
                        failure,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                Presence::Absent | Presence::Present(_) => {}
            },
            GeometryAttributePayloadAuthoring::DenseU32 { values, .. } => {
                if !values.is_empty() {
                    target.values = AttributeValuesRef::U32(retain_u32_sequence(
                        lane,
                        values,
                        GeometryU32ValuesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
            }
            GeometryAttributePayloadAuthoring::DenseI32 { values, .. } => {
                if !values.is_empty() {
                    target.values = AttributeValuesRef::I32(retain_i32_sequence(
                        lane,
                        values,
                        GeometryI32ValuesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
            }
            GeometryAttributePayloadAuthoring::SparseF32 {
                indices,
                values,
                default,
                ..
            } => {
                if !values.is_empty() {
                    target.values = AttributeValuesRef::F32(retain_f32_sequence(
                        lane,
                        values,
                        GeometryF32ValuesSchema,
                        #[cfg(test)]
                        faults,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !indices.is_empty() {
                    target.sparse_indices = Some(retain_u32_sequence(
                        lane,
                        indices,
                        GeometrySparseIndicesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !default.is_empty() {
                    target.sparse_default = AttributeDefaultRef::F32(retain_f32_sequence(
                        lane,
                        default,
                        GeometrySparseF32DefaultsSchema,
                        #[cfg(test)]
                        faults,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
            }
            GeometryAttributePayloadAuthoring::SparseU32 {
                indices,
                values,
                default,
                ..
            } => {
                if !values.is_empty() {
                    target.values = AttributeValuesRef::U32(retain_u32_sequence(
                        lane,
                        values,
                        GeometryU32ValuesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !indices.is_empty() {
                    target.sparse_indices = Some(retain_u32_sequence(
                        lane,
                        indices,
                        GeometrySparseIndicesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !default.is_empty() {
                    target.sparse_default = AttributeDefaultRef::U32(retain_u32_sequence(
                        lane,
                        default,
                        GeometrySparseU32DefaultsSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
            }
            GeometryAttributePayloadAuthoring::SparseI32 {
                indices,
                values,
                default,
                ..
            } => {
                if !values.is_empty() {
                    target.values = AttributeValuesRef::I32(retain_i32_sequence(
                        lane,
                        values,
                        GeometryI32ValuesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !indices.is_empty() {
                    target.sparse_indices = Some(retain_u32_sequence(
                        lane,
                        indices,
                        GeometrySparseIndicesSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !default.is_empty() {
                    target.sparse_default = AttributeDefaultRef::I32(retain_i32_sequence(
                        lane,
                        default,
                        GeometrySparseI32DefaultsSchema,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
            }
        }
    }
    Ok(payload_block_count)
}

fn record_payload_block(
    payload_block_count: &mut u32,
    failure: StoreBuildError,
) -> Result<(), StoreRuntimeError> {
    *payload_block_count = payload_block_count
        .checked_add(1)
        .ok_or(StoreRuntimeError::Build(failure))?;
    Ok(())
}

fn retain_u32_sequence<'lane, O, PP, U, L, Tag>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    values: &[u32],
    tag: Tag,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<u32>>,
{
    let plan = SequencePlan::<u32>::new(values.len()).map_err(StoreRuntimeError::Plan)?;
    lane.try_retain_sequence(plan, tag, |control| control.extend_from_slice(values))
        .map(CanonicalBlockRef::from_store)
}

fn retain_f32_sequence<'lane, O, PP, U, L, Tag>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    values: &[f32],
    tag: Tag,
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    let plan = SequencePlan::<f32>::new(values.len()).map_err(StoreRuntimeError::Plan)?;
    lane.try_retain_sequence(plan, tag, |control| {
        for &value in values {
            #[cfg(test)]
            let value = faults.geometry_f32(value);
            control.push(value)?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_i32_sequence<'lane, O, PP, U, L, Tag>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    values: &[i32],
    tag: Tag,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<i32>>,
{
    let plan = SequencePlan::<i32>::new(values.len()).map_err(StoreRuntimeError::Plan)?;
    lane.try_retain_sequence(plan, tag, |control| control.extend_from_slice(values))
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_projected_f32_sequence<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    source_values: &[f32],
    source_indices: &[u32],
    lanes: u8,
    scalar_count: u64,
    #[cfg(test)] faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryF32ValuesSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let scalar_count = usize::try_from(scalar_count)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let plan = SequencePlan::<f32>::new(scalar_count).map_err(StoreRuntimeError::Plan)?;
    lane.try_retain_sequence(plan, GeometryF32ValuesSchema, |control| {
        let lanes_usize = usize::from(lanes);
        for &source_index in source_indices {
            let start = usize::try_from(source_index)
                .map_err(|_| failure)?
                .checked_mul(lanes_usize)
                .ok_or(failure)?;
            let end = start.checked_add(lanes_usize).ok_or(failure)?;
            let values = source_values.get(start..end).ok_or(failure)?;
            for &value in values {
                #[cfg(test)]
                let value = faults.geometry_f32(value);
                control.push(value)?;
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_attribute_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &[AttributeLeafRefs],
    attribute_semantic_names: Option<CanonicalBlockRef<GeometryAttributeSemanticNameSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryAttributeTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, GeometryAttributeTableSchema, |control| {
        for attribute_index in 0..refs.len() {
            let attribute = batch.attribute(attribute_index).ok_or(failure)?;
            let payload_refs = *refs.get(attribute_index).ok_or(failure)?;
            let attribute_id = u32::try_from(attribute_index).map_err(|_| failure)?;
            let (semantic_kind, semantic_set) =
                encode_geometry_semantic(attribute.semantic()).ok_or(failure)?;
            let domain = encode_geometry_domain(attribute.domain()).ok_or(failure)?;
            let scalar_kind = match attribute.scalar_kind() {
                GeometryAttributeScalarKind::F32 => GEOMETRY_SCALAR_F32,
                GeometryAttributeScalarKind::U32 => GEOMETRY_SCALAR_U32,
                GeometryAttributeScalarKind::I32 => GEOMETRY_SCALAR_I32,
            };
            let payload_kind = match attribute.payload_kind() {
                GeometryAttributePayloadKind::Dense => GEOMETRY_PAYLOAD_DENSE,
                GeometryAttributePayloadKind::Sparse => GEOMETRY_PAYLOAD_SPARSE,
            };
            control.write_row(|row| {
                row.write_scalar(attribute_id)?;
                row.write_scalar(semantic_kind)?;
                row.write_scalar(semantic_set)?;
                row.write_scalar(domain)?;
                row.write_scalar(scalar_kind)?;
                row.write_scalar(attribute.lanes())?;
                row.write_scalar(GEOMETRY_NORMALIZATION_NONE)?;
                row.write_scalar(payload_kind)?;
                row.write_scalar(attribute.element_count())?;
                row.write_scalar(attribute.explicit_element_count())?;
                row.write_scalar(attribute.canonical_scalar_count())?;
                match payload_refs.values {
                    AttributeValuesRef::Absent => row.write_scalar(0_u8)?,
                    AttributeValuesRef::F32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                    AttributeValuesRef::U32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                    AttributeValuesRef::I32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                }
                match payload_refs.sparse_indices {
                    Some(reference) => row.write_sequence_ref(reference.into_store())?,
                    None => row.write_scalar(0_u8)?,
                }
                match payload_refs.sparse_default {
                    AttributeDefaultRef::Absent => row.write_scalar(0_u8)?,
                    AttributeDefaultRef::F32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                    AttributeDefaultRef::U32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                    AttributeDefaultRef::I32(reference) => {
                        row.write_sequence_ref(reference.into_store())?;
                    }
                }
                row.write_scalar(attribute.semantic_name_offset())?;
                row.write_scalar(attribute.semantic_name_count())?;
                write_optional_utf8_ref(
                    row,
                    attribute.semantic_name_count(),
                    if attribute.semantic_name_count() == 0 {
                        None
                    } else {
                        attribute_semantic_names
                    },
                    failure,
                )
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_primitive_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &[PrimitiveLeafRefs],
    appearance_materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
    attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryPrimitiveTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, GeometryPrimitiveTableSchema, |control| {
        for primitive_index in 0..refs.len() {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            let payload_refs = *refs.get(primitive_index).ok_or(failure)?;
            let primitive_id = u32::try_from(primitive_index).map_err(|_| failure)?;
            let topology = encode_geometry_topology(primitive.topology());
            let (polygon_presence, polygon_count) = match primitive.authored_topology() {
                GeometryTopologyAuthoring::Polygons(boundaries) => (
                    PRESENCE_PRESENT,
                    u32::try_from(boundaries.len()).map_err(|_| failure)?,
                ),
                _ => (PRESENCE_ABSENT, 0),
            };
            let (index_presence, index_count) = match primitive.authored_indices() {
                Presence::Absent => (PRESENCE_ABSENT, 0),
                Presence::Present(indices) => (
                    PRESENCE_PRESENT,
                    u32::try_from(indices.len()).map_err(|_| failure)?,
                ),
            };
            let (material_presence, material_index) = match primitive.material() {
                Some(material) => (PRESENCE_PRESENT, material.index()),
                None => (PRESENCE_ABSENT, 0),
            };
            let attribute_range = primitive.attribute_range();
            control.write_row(|row| {
                row.write_scalar(primitive_id)?;
                row.write_scalar(topology)?;
                row.write_scalar(primitive.vertex_count())?;
                row.write_scalar(primitive.corner_count())?;
                row.write_scalar(primitive.face_count())?;
                row.write_scalar(polygon_presence)?;
                row.write_scalar(polygon_count)?;
                match payload_refs.polygon_boundaries {
                    Some(reference) => row.write_sequence_ref(reference.into_store())?,
                    None => row.write_scalar(0_u8)?,
                }
                row.write_scalar(index_presence)?;
                row.write_scalar(index_count)?;
                match payload_refs.indices {
                    Some(reference) => row.write_sequence_ref(reference.into_store())?,
                    None => row.write_scalar(0_u8)?,
                }
                row.write_scalar(material_presence)?;
                row.write_scalar(material_index)?;
                if material_presence == PRESENCE_PRESENT {
                    row.write_record_ref(appearance_materials.ok_or(failure)?.into_store())?;
                } else {
                    row.write_scalar(0_u8)?;
                }
                row.write_scalar(attribute_range.offset())?;
                row.write_scalar(attribute_range.count())?;
                if attribute_range.count() == 0 {
                    row.write_scalar(0_u8)?;
                } else {
                    row.write_record_ref(attributes.ok_or(failure)?.into_store())?;
                }
                Ok(())
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_logical_mesh_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    logical_mesh_names: Option<CanonicalBlockRef<GeometryLogicalMeshNameSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryLogicalMeshTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = batch.input();
    let logical_meshes = input.logical_meshes();
    lane.try_retain_records(plan, GeometryLogicalMeshTableSchema, |control| {
        let primitive_table = primitives.ok_or(failure)?;
        let mut name_offset = 0_u32;
        for (logical_mesh_index, logical_mesh) in logical_meshes.iter().copied().enumerate() {
            let logical_mesh_id = u32::try_from(logical_mesh_index).map_err(|_| failure)?;
            let range = logical_mesh.primitives();
            let (name_presence, name_count, name_reference) = match logical_mesh.name() {
                Presence::Absent => (PRESENCE_ABSENT, 0, None),
                Presence::Present(name) => {
                    let count = u32::try_from(name.len()).map_err(|_| failure)?;
                    (
                        PRESENCE_PRESENT,
                        count,
                        if count == 0 { None } else { logical_mesh_names },
                    )
                }
            };
            control.write_row(|row| {
                row.write_scalar(logical_mesh_id)?;
                row.write_scalar(range.offset())?;
                row.write_scalar(range.count())?;
                row.write_record_ref(primitive_table.into_store())?;
                row.write_scalar(name_presence)?;
                row.write_scalar(name_offset)?;
                row.write_scalar(name_count)?;
                write_optional_utf8_ref(row, name_count, name_reference, failure)
            })?;
            name_offset = name_offset.checked_add(name_count).ok_or(failure)?;
        }
        if name_offset != batch.logical_mesh_name_byte_count() {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_mesh_instance_table<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    node_table: CanonicalBlockRef<super::super::super::schema::NodeSceneNodeTableSchema>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryMeshInstanceTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = batch.input();
    let instances = input.mesh_instances();
    let node_mapping = nodes.canonical_index_by_local();
    lane.try_retain_records(plan, GeometryMeshInstanceTableSchema, |control| {
        let logical_mesh_table = logical_meshes.ok_or(failure)?;
        for (instance_index, instance) in instances.iter().copied().enumerate() {
            let instance_id = u32::try_from(instance_index).map_err(|_| failure)?;
            let owner_local = usize::try_from(instance.owner().index()).map_err(|_| failure)?;
            let owner_node = *node_mapping.get(owner_local).ok_or(failure)?;
            control.write_row(|row| {
                row.write_scalar(instance_id)?;
                row.write_scalar(owner_node)?;
                row.write_record_ref(node_table.into_store())?;
                row.write_scalar(instance.logical_mesh().index())?;
                row.write_record_ref(logical_mesh_table.into_store())
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_geometry_directory<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &GeometryMaterializationPlans,
    refs: GeometryTableRefs,
    payloads: GeometryPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SceneGeometryDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plans.directory, SceneGeometryDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(SCENE_GEOMETRY_SCHEMA_REVISION)?;
            write_optional_table_ref(row, plans.logical_mesh_count, refs.logical_meshes, failure)?;
            write_optional_table_ref(row, plans.primitive_count, refs.primitives, failure)?;
            write_optional_table_ref(row, plans.attribute_count, refs.attributes, failure)?;
            write_optional_table_ref(row, plans.mesh_instance_count, refs.mesh_instances, failure)?;
            row.write_scalar(plans.logical_mesh_name_byte_count)?;
            write_optional_utf8_ref(
                row,
                plans.logical_mesh_name_byte_count,
                payloads.logical_mesh_names,
                failure,
            )?;
            row.write_scalar(plans.attribute_semantic_name_byte_count)?;
            write_optional_utf8_ref(
                row,
                plans.attribute_semantic_name_byte_count,
                payloads.attribute_semantic_names,
                failure,
            )
        })
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_optional_utf8_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    if count == 0 {
        if reference.is_some() {
            return Err(failure.into());
        }
        row.write_scalar(0_u8)
    } else {
        row.write_utf8_ref(reference.ok_or(failure)?.into_store())
    }
}

fn write_optional_table_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    row.write_scalar(count)?;
    if count == 0 {
        if reference.is_some() {
            return Err(failure.into());
        }
        row.write_scalar(0_u8)
    } else {
        row.write_record_ref(reference.ok_or(failure)?.into_store())
    }
}
