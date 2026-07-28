use baozi_operation::{
    CapacityPlan, RecordPlan, RetainedStoreReservation, ScalarSequence, SequencePlan,
    StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryAttributePayloadKind, GeometryAttributeScalarKind,
    GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch, ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    GEOMETRY_MATERIAL_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT, GEOMETRY_NORMALIZATION_NONE,
    GEOMETRY_PAYLOAD_DENSE, GEOMETRY_PAYLOAD_SPARSE, GEOMETRY_PRIMITIVE_FIELD_COUNT,
    GEOMETRY_SCALAR_F32, GEOMETRY_SCALAR_U32, GeometryAttributeTableSchema,
    GeometryF32ValuesSchema, GeometryLogicalMeshTableSchema, GeometryMaterialTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPolygonBoundariesSchema,
    GeometryPrimitiveIndicesSchema, GeometryPrimitiveTableSchema, GeometrySparseF32DefaultsSchema,
    GeometrySparseIndicesSchema, GeometrySparseU32DefaultsSchema, GeometryU32ValuesSchema,
    encode_geometry_domain, encode_geometry_semantic, encode_geometry_topology,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT, SCENE_GEOMETRY_SCHEMA_REVISION,
    SceneGeometryDirectorySchema,
};

use super::super::{
    SceneConstructionOwner, SceneStoreAssembly, SceneStoreControlCheckpoint, SceneStoreKeys,
};
use super::{NodeSceneBuildFaults, NodeSceneWorkMeter};

pub(super) struct GeometryMaterializationPlans {
    materials: Option<RecordPlan>,
    logical_meshes: Option<RecordPlan>,
    primitives: Option<RecordPlan>,
    attributes: Option<RecordPlan>,
    mesh_instances: Option<RecordPlan>,
    directory: RecordPlan,
    material_count: u32,
    logical_mesh_count: u32,
    primitive_count: u32,
    attribute_count: u32,
    mesh_instance_count: u32,
    payload_block_count: u32,
    canonical_block_count: usize,
}

impl GeometryMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedGeometryBatch<'_, R>) -> Result<Self, StorePlanError> {
        let material_count = batch.material_count();
        let logical_mesh_count = batch.logical_mesh_count();
        let primitive_count = batch.primitive_count();
        let attribute_count = batch.attribute_count();
        let mesh_instance_count = batch.mesh_instance_count();
        let materials = record_plan_if_nonempty(material_count, GEOMETRY_MATERIAL_FIELD_COUNT)?;
        let logical_meshes =
            record_plan_if_nonempty(logical_mesh_count, GEOMETRY_LOGICAL_MESH_FIELD_COUNT)?;
        let primitives = record_plan_if_nonempty(primitive_count, GEOMETRY_PRIMITIVE_FIELD_COUNT)?;
        let attributes = record_plan_if_nonempty(attribute_count, GEOMETRY_ATTRIBUTE_FIELD_COUNT)?;
        let mesh_instances =
            record_plan_if_nonempty(mesh_instance_count, GEOMETRY_MESH_INSTANCE_FIELD_COUNT)?;
        let table_block_count = [
            materials.is_some(),
            logical_meshes.is_some(),
            primitives.is_some(),
            attributes.is_some(),
            mesh_instances.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let payload_block_count = batch.payload_block_count();
        let canonical_block_count = 1_usize
            .checked_add(table_block_count)
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()
                    .and_then(|payload| count.checked_add(payload))
            })
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            materials,
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            directory: RecordPlan::new(1, SCENE_GEOMETRY_DIRECTORY_FIELD_COUNT)?,
            material_count,
            logical_mesh_count,
            primitive_count,
            attribute_count,
            mesh_instance_count,
            payload_block_count,
            canonical_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn material_count(&self) -> u32 {
        self.material_count
    }

    pub(super) const fn logical_mesh_count(&self) -> u32 {
        self.logical_mesh_count
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
}

#[derive(Clone, Copy)]
enum AttributeDefaultRef {
    Absent,
    F32(CanonicalBlockRef<GeometrySparseF32DefaultsSchema>),
    U32(CanonicalBlockRef<GeometrySparseU32DefaultsSchema>),
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

pub(super) struct GeometryReferenceScratch {
    primitive_refs: Vec<PrimitiveLeafRefs>,
    attribute_refs: Vec<AttributeLeafRefs>,
    _reservation: RetainedStoreReservation<SceneConstructionOwner>,
}

impl GeometryReferenceScratch {
    pub(super) fn try_new<R>(
        checkpoint: &mut SceneStoreControlCheckpoint<'_>,
        keys: SceneStoreKeys,
        batch: &ValidatedGeometryBatch<'_, R>,
    ) -> Result<Self, StoreRuntimeError> {
        let primitive_plan = CapacityPlan::for_type::<PrimitiveLeafRefs>(
            0,
            0,
            usize::try_from(batch.primitive_count())
                .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?,
        )
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let attribute_plan = CapacityPlan::for_type::<AttributeLeafRefs>(
            0,
            0,
            usize::try_from(batch.attribute_count())
                .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?,
        )
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let envelope = primitive_plan
            .envelope_delta_bytes()
            .ok()
            .and_then(|first| {
                attribute_plan
                    .envelope_delta_bytes()
                    .ok()
                    .and_then(|second| first.checked_add(second))
            })
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;

        checkpoint.poll().map_err(StoreRuntimeError::Control)?;
        let mut reservation = checkpoint.reserve(keys.phase, keys.dimension, envelope)?;
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
            checkpoint,
            keys,
            &mut primitive_refs,
            usize::try_from(batch.primitive_count())
                .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?,
            PrimitiveLeafRefs::EMPTY,
        )?;
        fill_reference_scratch(
            checkpoint,
            keys,
            &mut attribute_refs,
            usize::try_from(batch.attribute_count())
                .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?,
            AttributeLeafRefs::EMPTY,
        )?;
        checkpoint.poll().map_err(StoreRuntimeError::Control)?;
        Ok(Self {
            primitive_refs,
            attribute_refs,
            _reservation: reservation,
        })
    }
}

fn fill_reference_scratch<O, PP, U, T: Copy>(
    checkpoint: &mut baozi_operation::RetainedStoreControlCheckpoint<'_, O, PP, U>,
    keys: SceneStoreKeys,
    values: &mut Vec<T>,
    count: usize,
    value: T,
) -> Result<(), StoreRuntimeError>
where
    O: baozi_operation::OperationOwner,
    PP: baozi_operation::ProgressPhase,
    U: baozi_operation::ProgressUnit,
{
    let poll_interval = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
    for index in 0..count {
        if index.is_multiple_of(poll_interval) {
            checkpoint.poll().map_err(StoreRuntimeError::Control)?;
        }
        checkpoint.charge_observed(keys.dimension, 1)?;
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

#[derive(Clone, Copy)]
struct GeometryTableRefs {
    materials: Option<CanonicalBlockRef<GeometryMaterialTableSchema>>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn retain_geometry<R, NR>(
    assembly: &mut SceneStoreAssembly,
    plans: &GeometryMaterializationPlans,
    mut scratch: GeometryReferenceScratch,
    batch: &ValidatedGeometryBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    node_table: CanonicalBlockRef<super::super::super::schema::NodeSceneNodeTableSchema>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SceneGeometryDirectorySchema>, StoreRuntimeError> {
    let primitive_payload_blocks = retain_primitive_leaves(
        assembly,
        batch,
        &mut scratch.primitive_refs,
        keys,
        work,
        failure,
    )?;
    let attribute_payload_blocks = retain_attribute_leaves(
        assembly,
        batch,
        &mut scratch.attribute_refs,
        keys,
        work,
        faults,
        failure,
    )?;
    let payload_block_count = primitive_payload_blocks
        .checked_add(attribute_payload_blocks)
        .ok_or(StoreRuntimeError::Build(failure))?;
    if payload_block_count != batch.payload_block_count() {
        return Err(StoreRuntimeError::Build(failure));
    }

    let materials = match plans.materials {
        Some(plan) => Some(retain_material_table(
            assembly,
            plan,
            plans.material_count,
            keys,
            work,
        )?),
        None => None,
    };
    let attributes = match plans.attributes {
        Some(plan) => Some(retain_attribute_table(
            assembly,
            plan,
            batch,
            &scratch.attribute_refs,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let primitives = match plans.primitives {
        Some(plan) => Some(retain_primitive_table(
            assembly,
            plan,
            batch,
            &scratch.primitive_refs,
            materials,
            attributes,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let logical_meshes = match plans.logical_meshes {
        Some(plan) => Some(retain_logical_mesh_table(
            assembly, plan, batch, primitives, keys, work, failure,
        )?),
        None => None,
    };
    let mesh_instances = match plans.mesh_instances {
        Some(plan) => Some(retain_mesh_instance_table(
            assembly,
            plan,
            batch,
            nodes,
            node_table,
            logical_meshes,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let refs = GeometryTableRefs {
        materials,
        logical_meshes,
        primitives,
        attributes,
        mesh_instances,
    };
    retain_geometry_directory(assembly, plans, refs, keys, work, failure)
}

fn retain_primitive_leaves<R>(
    assembly: &mut SceneStoreAssembly,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &mut [PrimitiveLeafRefs],
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<u32, StoreRuntimeError> {
    let mut payload_block_count = 0_u32;
    for primitive_index in 0..refs.len() {
        {
            let mut checkpoint = assembly.try_control_checkpoint()?;
            work.observe_checkpoint(&mut checkpoint, keys, 1)?;
        }
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
                assembly,
                boundaries,
                GeometryPolygonBoundariesSchema,
                keys,
                work,
            )?);
            record_payload_block(&mut payload_block_count, failure)?;
        }
        if let Presence::Present(indices) = primitive.authored_indices()
            && !indices.is_empty()
        {
            target.indices = Some(retain_u32_sequence(
                assembly,
                indices,
                GeometryPrimitiveIndicesSchema,
                keys,
                work,
            )?);
            record_payload_block(&mut payload_block_count, failure)?;
        }
    }
    Ok(payload_block_count)
}

fn retain_attribute_leaves<R>(
    assembly: &mut SceneStoreAssembly,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &mut [AttributeLeafRefs],
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<u32, StoreRuntimeError> {
    let mut payload_block_count = 0_u32;
    for attribute_index in 0..refs.len() {
        {
            let mut checkpoint = assembly.try_control_checkpoint()?;
            work.observe_checkpoint(&mut checkpoint, keys, 1)?;
        }
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
                        assembly,
                        source_values,
                        GeometryF32ValuesSchema,
                        keys,
                        work,
                        faults,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                Presence::Present(indices) if !indices.is_empty() => {
                    target.values = AttributeValuesRef::F32(retain_projected_f32_sequence(
                        assembly,
                        source_values,
                        indices,
                        lanes,
                        attribute.canonical_scalar_count(),
                        keys,
                        work,
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
                        assembly,
                        values,
                        GeometryU32ValuesSchema,
                        keys,
                        work,
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
                        assembly,
                        values,
                        GeometryF32ValuesSchema,
                        keys,
                        work,
                        faults,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !indices.is_empty() {
                    target.sparse_indices = Some(retain_u32_sequence(
                        assembly,
                        indices,
                        GeometrySparseIndicesSchema,
                        keys,
                        work,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !default.is_empty() {
                    target.sparse_default = AttributeDefaultRef::F32(retain_f32_sequence(
                        assembly,
                        default,
                        GeometrySparseF32DefaultsSchema,
                        keys,
                        work,
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
                        assembly,
                        values,
                        GeometryU32ValuesSchema,
                        keys,
                        work,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !indices.is_empty() {
                    target.sparse_indices = Some(retain_u32_sequence(
                        assembly,
                        indices,
                        GeometrySparseIndicesSchema,
                        keys,
                        work,
                    )?);
                    record_payload_block(&mut payload_block_count, failure)?;
                }
                if !default.is_empty() {
                    target.sparse_default = AttributeDefaultRef::U32(retain_u32_sequence(
                        assembly,
                        default,
                        GeometrySparseU32DefaultsSchema,
                        keys,
                        work,
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

fn retain_u32_sequence<Tag>(
    assembly: &mut SceneStoreAssembly,
    values: &[u32],
    tag: Tag,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<u32>>,
{
    let plan = SequencePlan::<u32>::new(values.len()).map_err(StoreRuntimeError::Plan)?;
    let work_amount =
        u64::try_from(values.len()).map_err(|_| StoreRuntimeError::AccountingOverflow)?;
    assembly
        .try_retain_canonical_sequence_controlled(plan, keys.dimension, tag, |writer, control| {
            work.observe(control, work_amount)?;
            writer
                .extend_from_slice(values)
                .map_err(StoreControlledError::from)
        })
        .map(CanonicalBlockRef::from_store)
}

fn retain_f32_sequence<Tag>(
    assembly: &mut SceneStoreAssembly,
    values: &[f32],
    tag: Tag,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut NodeSceneBuildFaults,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    let plan = SequencePlan::<f32>::new(values.len()).map_err(StoreRuntimeError::Plan)?;
    let work_amount =
        u64::try_from(values.len()).map_err(|_| StoreRuntimeError::AccountingOverflow)?;
    assembly
        .try_retain_canonical_sequence_controlled(plan, keys.dimension, tag, |writer, control| {
            work.observe(control, work_amount)?;
            for &value in values {
                writer.push(faults.geometry_f32(value))?;
            }
            Ok(())
        })
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_projected_f32_sequence(
    assembly: &mut SceneStoreAssembly,
    source_values: &[f32],
    source_indices: &[u32],
    lanes: u8,
    scalar_count: u64,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut NodeSceneBuildFaults,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryF32ValuesSchema>, StoreRuntimeError> {
    let scalar_count = usize::try_from(scalar_count)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let plan = SequencePlan::<f32>::new(scalar_count).map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            GeometryF32ValuesSchema,
            |writer, control| {
                let lanes_usize = usize::from(lanes);
                for &source_index in source_indices {
                    let start = usize::try_from(source_index)
                        .map_err(|_| failure)?
                        .checked_mul(lanes_usize)
                        .ok_or(failure)?;
                    let end = start.checked_add(lanes_usize).ok_or(failure)?;
                    let values = source_values.get(start..end).ok_or(failure)?;
                    work.observe(control, u64::from(lanes))?;
                    for &value in values {
                        writer.push(faults.geometry_f32(value))?;
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_material_table(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    material_count: u32,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<GeometryMaterialTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            GeometryMaterialTableSchema,
            |writer, control| {
                for material in 0..material_count {
                    work.observe(control, 1)?;
                    writer.write_row(|row| row.write_scalar(material))?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_attribute_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &[AttributeLeafRefs],
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryAttributeTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            GeometryAttributeTableSchema,
            |writer, control| {
                for attribute_index in 0..refs.len() {
                    work.observe(control, 1)?;
                    let attribute = batch.attribute(attribute_index).ok_or(failure)?;
                    let payload_refs = *refs.get(attribute_index).ok_or(failure)?;
                    let attribute_id = u32::try_from(attribute_index).map_err(|_| failure)?;
                    let (semantic_kind, semantic_set) =
                        encode_geometry_semantic(attribute.semantic()).ok_or(failure)?;
                    let domain = encode_geometry_domain(attribute.domain()).ok_or(failure)?;
                    let scalar_kind = match attribute.scalar_kind() {
                        GeometryAttributeScalarKind::F32 => GEOMETRY_SCALAR_F32,
                        GeometryAttributeScalarKind::U32 => GEOMETRY_SCALAR_U32,
                    };
                    let payload_kind = match attribute.payload_kind() {
                        GeometryAttributePayloadKind::Dense => GEOMETRY_PAYLOAD_DENSE,
                        GeometryAttributePayloadKind::Sparse => GEOMETRY_PAYLOAD_SPARSE,
                    };
                    writer.write_row(|row| {
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
                        }
                        Ok(())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_primitive_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    refs: &[PrimitiveLeafRefs],
    materials: Option<CanonicalBlockRef<GeometryMaterialTableSchema>>,
    attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryPrimitiveTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            GeometryPrimitiveTableSchema,
            |writer, control| {
                for primitive_index in 0..refs.len() {
                    work.observe(control, 1)?;
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
                    writer.write_row(|row| {
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
                            row.write_record_ref(materials.ok_or(failure)?.into_store())?;
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
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_logical_mesh_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryLogicalMeshTableSchema>, StoreRuntimeError> {
    let input = batch.input();
    let logical_meshes = input.logical_meshes();
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            GeometryLogicalMeshTableSchema,
            |writer, control| {
                let primitive_table = primitives.ok_or(failure)?;
                for (logical_mesh_index, logical_mesh) in logical_meshes.iter().copied().enumerate()
                {
                    work.observe(control, 1)?;
                    let logical_mesh_id = u32::try_from(logical_mesh_index).map_err(|_| failure)?;
                    let range = logical_mesh.primitives();
                    writer.write_row(|row| {
                        row.write_scalar(logical_mesh_id)?;
                        row.write_scalar(range.offset())?;
                        row.write_scalar(range.count())?;
                        row.write_record_ref(primitive_table.into_store())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_mesh_instance_table<R, NR>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedGeometryBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    node_table: CanonicalBlockRef<super::super::super::schema::NodeSceneNodeTableSchema>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<GeometryMeshInstanceTableSchema>, StoreRuntimeError> {
    let input = batch.input();
    let instances = input.mesh_instances();
    let node_mapping = nodes.canonical_index_by_local();
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            GeometryMeshInstanceTableSchema,
            |writer, control| {
                let logical_mesh_table = logical_meshes.ok_or(failure)?;
                for (instance_index, instance) in instances.iter().copied().enumerate() {
                    work.observe(control, 1)?;
                    let instance_id = u32::try_from(instance_index).map_err(|_| failure)?;
                    let owner_local =
                        usize::try_from(instance.owner().index()).map_err(|_| failure)?;
                    let owner_node = *node_mapping.get(owner_local).ok_or(failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(instance_id)?;
                        row.write_scalar(owner_node)?;
                        row.write_record_ref(node_table.into_store())?;
                        row.write_scalar(instance.logical_mesh().index())?;
                        row.write_record_ref(logical_mesh_table.into_store())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_geometry_directory(
    assembly: &mut SceneStoreAssembly,
    plans: &GeometryMaterializationPlans,
    refs: GeometryTableRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SceneGeometryDirectorySchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plans.directory,
            keys.dimension,
            SceneGeometryDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(SCENE_GEOMETRY_SCHEMA_REVISION)?;
                        write_optional_table_ref(
                            row,
                            plans.material_count,
                            refs.materials,
                            failure,
                        )?;
                        write_optional_table_ref(
                            row,
                            plans.logical_mesh_count,
                            refs.logical_meshes,
                            failure,
                        )?;
                        write_optional_table_ref(
                            row,
                            plans.primitive_count,
                            refs.primitives,
                            failure,
                        )?;
                        write_optional_table_ref(
                            row,
                            plans.attribute_count,
                            refs.attributes,
                            failure,
                        )?;
                        write_optional_table_ref(
                            row,
                            plans.mesh_instance_count,
                            refs.mesh_instances,
                            failure,
                        )?;
                        Ok(())
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn write_optional_table_ref<Tag>(
    row: &mut baozi_operation::CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    row.write_scalar(count)?;
    if count == 0 {
        if reference.is_some() {
            return Err(failure);
        }
        row.write_scalar(0_u8)
    } else {
        row.write_record_ref(reference.ok_or(failure)?.into_store())
    }
}
