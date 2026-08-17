use baozi_operation::{
    CapacityPlan, RetainedStoreReservation, ShareableStoreKind, StoreRuntimeError,
};

use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_PRIMITIVE_FIELD_COUNT, GeometryF32ValuesSchema,
    GeometryI32ValuesSchema, GeometryPolygonBoundariesSchema, GeometryPrimitiveIndicesSchema,
    GeometrySparseF32DefaultsSchema, GeometrySparseI32DefaultsSchema, GeometrySparseIndicesSchema,
    GeometrySparseU32DefaultsSchema, GeometryU32ValuesSchema, geometry_attribute_field,
    geometry_primitive_field,
};
use crate::canonical_scene::store::schema::profile::{
    PROFILE_METADATA_DIRECTORY_FIELD_COUNT, PROFILE_SEGMENT_FIELD_COUNT,
    ProfileLineageDigestSchema, ProfileLineageEvidenceSchema, ProfileLineageOperationSchema,
    ProfileLineageParentSchema, ProfileLineageProvenanceSchema, ProfileLineageResourceSchema,
    ProfileLineageSourceSchema, ProfileLineageTextSchema, ProfileLineageWideSchema,
    ProfileMetadataBoolSchema, ProfileMetadataBytesSchema, ProfileMetadataF32Schema,
    ProfileMetadataF64Schema, ProfileMetadataI64Schema, ProfileMetadataTextSchema,
    ProfileMetadataU64Schema, profile_metadata_directory_field, profile_segment_field,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};

use super::super::{
    NodeSceneWorkMeter, SceneConstructionOwner, SceneStoreAssembly, SceneStoreKeys,
    ValidatedNodeScene,
};
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditPlanError, NodeSceneEvidenceLeafPlan};
use super::required_record_view;

#[derive(Clone, Copy)]
enum NodeSceneInheritedLeaf {
    GeometryPolygonBoundaries(CanonicalBlockRef<GeometryPolygonBoundariesSchema>),
    GeometryPrimitiveIndices(CanonicalBlockRef<GeometryPrimitiveIndicesSchema>),
    GeometryF32Values(CanonicalBlockRef<GeometryF32ValuesSchema>),
    GeometryU32Values(CanonicalBlockRef<GeometryU32ValuesSchema>),
    GeometryI32Values(CanonicalBlockRef<GeometryI32ValuesSchema>),
    GeometrySparseIndices(CanonicalBlockRef<GeometrySparseIndicesSchema>),
    GeometrySparseF32Defaults(CanonicalBlockRef<GeometrySparseF32DefaultsSchema>),
    GeometrySparseU32Defaults(CanonicalBlockRef<GeometrySparseU32DefaultsSchema>),
    GeometrySparseI32Defaults(CanonicalBlockRef<GeometrySparseI32DefaultsSchema>),
    ProfileResources(CanonicalBlockRef<ProfileLineageResourceSchema>),
    ProfileSources(CanonicalBlockRef<ProfileLineageSourceSchema>),
    ProfileProvenance(CanonicalBlockRef<ProfileLineageProvenanceSchema>),
    ProfileOperations(CanonicalBlockRef<ProfileLineageOperationSchema>),
    ProfileEvidence(CanonicalBlockRef<ProfileLineageEvidenceSchema>),
    ProfileParents(CanonicalBlockRef<ProfileLineageParentSchema>),
    ProfileWide(CanonicalBlockRef<ProfileLineageWideSchema>),
    ProfileText(CanonicalBlockRef<ProfileLineageTextSchema>),
    ProfileDigests(CanonicalBlockRef<ProfileLineageDigestSchema>),
    MetadataText(CanonicalBlockRef<ProfileMetadataTextSchema>),
    MetadataBytes(CanonicalBlockRef<ProfileMetadataBytesSchema>),
    MetadataBool(CanonicalBlockRef<ProfileMetadataBoolSchema>),
    MetadataI64(CanonicalBlockRef<ProfileMetadataI64Schema>),
    MetadataU64(CanonicalBlockRef<ProfileMetadataU64Schema>),
    MetadataF64(CanonicalBlockRef<ProfileMetadataF64Schema>),
    MetadataF32(CanonicalBlockRef<ProfileMetadataF32Schema>),
}

/// Allocation-free shape and capacity proof for one inherited-leaf source scan.
#[derive(Clone, Copy)]
pub(super) struct NodeSceneInheritedLeafScanPlan {
    capacity: CapacityPlan,
    envelope_bytes: u64,
    structural_upper: usize,
    inspection_work: usize,
}

impl NodeSceneInheritedLeafScanPlan {
    pub(super) fn try_new(
        input: &ValidatedNodeScene,
        primitive_rows: usize,
        attribute_rows: usize,
        source_segments: usize,
        evidence: NodeSceneEvidenceLeafPlan,
    ) -> Result<Self, NodeSceneEditPlanError> {
        Self::try_from_counts(
            direct_inherited_leaf_count(input),
            evidence.inherited.as_usize(),
            primitive_rows,
            attribute_rows,
            source_segments,
        )
    }

    fn try_from_counts(
        direct_inherited: usize,
        inherited: usize,
        primitive_rows: usize,
        attribute_rows: usize,
        source_segments: usize,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let structural_upper = primitive_rows
            .checked_mul(2)
            .and_then(|count| {
                attribute_rows
                    .checked_mul(3)
                    .and_then(|attributes| count.checked_add(attributes))
            })
            .and_then(|count| {
                source_segments
                    .checked_mul(9)
                    .and_then(|segments| count.checked_add(segments))
            })
            .and_then(|count| count.checked_add(7))
            .ok_or(NodeSceneEditPlanError::SizeOverflow {
                quantity: "inherited shareable leaf structural upper bound",
            })?;
        let exact_inspection_work = primitive_rows
            .checked_add(attribute_rows)
            .and_then(|count| count.checked_add(source_segments))
            .and_then(|count| count.checked_add(1))
            .ok_or(NodeSceneEditPlanError::SizeOverflow {
                quantity: "inherited shareable leaf inspection work",
            })?;
        let expected_dynamic = inherited.checked_sub(direct_inherited).ok_or(
            NodeSceneEditPlanError::InvalidSource {
                reason: "direct inherited leaves exceed the sealed inherited leaf set",
            },
        )?;
        if expected_dynamic > structural_upper {
            return Err(NodeSceneEditPlanError::InvalidSource {
                reason: "dynamic inherited leaves exceed their source-table structural upper bound",
            });
        }
        let capacity = CapacityPlan::for_type::<NodeSceneInheritedLeaf>(0, 0, expected_dynamic)
            .map_err(|_| NodeSceneEditPlanError::SizeOverflow {
                quantity: "inherited shareable leaf allocation",
            })?;
        let envelope_bytes =
            capacity
                .envelope_delta_bytes()
                .map_err(|_| NodeSceneEditPlanError::SizeOverflow {
                    quantity: "inherited shareable leaf allocation envelope",
                })?;

        Ok(Self {
            capacity,
            envelope_bytes,
            structural_upper,
            inspection_work: exact_inspection_work,
        })
    }

    pub(super) const fn capacity(self) -> CapacityPlan {
        self.capacity
    }

    pub(super) const fn envelope_bytes(self) -> u64 {
        self.envelope_bytes
    }

    pub(super) const fn expected_dynamic(self) -> usize {
        self.capacity.desired_capacity()
    }

    pub(super) const fn structural_upper(self) -> usize {
        self.structural_upper
    }

    pub(super) const fn inspection_work(self) -> usize {
        self.inspection_work
    }
}

/// Exact dynamic shareable leaves discovered by one metered source inspection pass.
///
/// The vector is bounded by the sealed inherited-leaf count and drops before the reservation that
/// accounts for its allocator capacity. Materialization consumes this list without rescanning the
/// source tables or rediscovering physical shape.
pub(super) struct NodeSceneInheritedLeafPlan<R> {
    leaves: Vec<NodeSceneInheritedLeaf>,
    _reservation: R,
}

impl NodeSceneInheritedLeafPlan<RetainedStoreReservation<SceneConstructionOwner>> {
    pub(super) fn try_collect(
        assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
        pending: &BoundPendingNodeSceneEdit<'_>,
        keys: SceneStoreKeys,
        work: &mut NodeSceneWorkMeter,
    ) -> Result<Self, StoreRuntimeError> {
        let scan = pending.counts.inherited_leaf_scan;
        let expected_dynamic = scan.expected_dynamic();
        let capacity = scan.capacity();
        let mut checkpoint = assembly.try_control_checkpoint()?;
        let mut reservation =
            checkpoint.reserve(keys.phase, keys.dimension, scan.envelope_bytes())?;
        checkpoint.validate_live_owner(&reservation)?;
        capacity
            .verify_preflight(0, 0)
            .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
        let mut leaves = Vec::new();
        leaves
            .try_reserve_exact(capacity.allocator_additional())
            .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
        let actual = capacity
            .finish(leaves.len(), leaves.capacity())
            .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
        checkpoint.resize(&mut reservation, actual)?;
        drop(checkpoint);

        let mut collector = InheritedLeafCollector {
            leaves: &mut leaves,
            expected_dynamic,
            inspected: 0,
        };
        collect_geometry_leaves(assembly, pending, keys, work, &mut collector)?;
        collect_profile_leaves(assembly, pending, keys, work, &mut collector)?;
        if collector.leaves.len() != expected_dynamic
            || collector.leaves.len() > scan.structural_upper()
            || collector.inspected != scan.inspection_work()
        {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }

        Ok(Self {
            leaves,
            _reservation: reservation,
        })
    }
}

impl<R> NodeSceneInheritedLeafPlan<R> {
    pub(super) fn inherit(
        &self,
        assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
        input: &ValidatedNodeScene,
        inherited: &mut usize,
    ) -> Result<(), StoreRuntimeError> {
        for leaf in &self.leaves {
            leaf.inherit(assembly, input, inherited)?;
        }
        Ok(())
    }
}

impl NodeSceneInheritedLeaf {
    fn inherit(
        self,
        assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
        input: &ValidatedNodeScene,
        inherited: &mut usize,
    ) -> Result<(), StoreRuntimeError> {
        match self {
            Self::GeometryPolygonBoundaries(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometryPrimitiveIndices(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometryF32Values(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometryU32Values(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometryI32Values(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometrySparseIndices(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometrySparseF32Defaults(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometrySparseU32Defaults(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::GeometrySparseI32Defaults(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileResources(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileSources(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileProvenance(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileOperations(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileEvidence(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileParents(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileWide(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileText(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::ProfileDigests(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataText(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataBytes(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataBool(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataI64(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataU64(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataF64(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
            Self::MetadataF32(reference) => {
                inherit_optional_leaf(assembly, input, Some(reference), inherited)
            }
        }
    }
}

pub(super) fn inherit_optional_leaf<Schema>(
    assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
    input: &ValidatedNodeScene,
    source: Option<CanonicalBlockRef<Schema>>,
    inherited: &mut usize,
) -> Result<(), StoreRuntimeError>
where
    Schema: CanonicalSchema,
    Schema::StoreKind: ShareableStoreKind,
{
    let Some(source) = source else {
        return Ok(());
    };
    let _ = assembly.try_share_canonical(&input.canonical, source.into_store())?;
    *inherited = inherited
        .checked_add(1)
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    Ok(())
}

struct InheritedLeafCollector<'a> {
    leaves: &'a mut Vec<NodeSceneInheritedLeaf>,
    expected_dynamic: usize,
    inspected: usize,
}

impl InheritedLeafCollector<'_> {
    fn push(&mut self, leaf: Option<NodeSceneInheritedLeaf>) -> Result<(), StoreRuntimeError> {
        let Some(leaf) = leaf else {
            return Ok(());
        };
        if self.leaves.len() >= self.expected_dynamic {
            return Err(StoreRuntimeError::InvalidSourceSlot);
        }
        self.leaves.push(leaf);
        Ok(())
    }

    fn observe(
        &mut self,
        assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
        keys: SceneStoreKeys,
        work: &mut NodeSceneWorkMeter,
    ) -> Result<(), StoreRuntimeError> {
        let mut checkpoint = assembly.try_control_checkpoint()?;
        work.observe_checkpoint(&mut checkpoint, keys, 1)?;
        self.inspected = self
            .inspected
            .checked_add(1)
            .ok_or(StoreRuntimeError::AccountingOverflow)?;
        Ok(())
    }
}

fn collect_geometry_leaves(
    assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    collector: &mut InheritedLeafCollector<'_>,
) -> Result<(), StoreRuntimeError> {
    match pending.input.proof.primitives() {
        None if pending.counts.primitives.as_usize() == 0 => {}
        Some(reference) => {
            let primitives = required_record_view(pending.input, reference)?;
            if primitives.rows() != pending.counts.primitives.as_usize()
                || primitives.fields() != GEOMETRY_PRIMITIVE_FIELD_COUNT
            {
                return Err(StoreRuntimeError::InvalidSourceSlot);
            }
            for row in 0..primitives.rows() {
                collector.observe(assembly, keys, work)?;
                collector.push(
                    primitives
                        .sequence_ref::<u32, GeometryPolygonBoundariesSchema>(
                            row,
                            geometry_primitive_field::POLYGON_BOUNDARIES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometryPolygonBoundaries),
                )?;
                collector.push(
                    primitives
                        .sequence_ref::<u32, GeometryPrimitiveIndicesSchema>(
                            row,
                            geometry_primitive_field::INDICES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometryPrimitiveIndices),
                )?;
            }
        }
        None => return Err(StoreRuntimeError::InvalidSourceSlot),
    }

    match pending.input.proof.attributes() {
        None if pending.counts.attributes.as_usize() == 0 => {}
        Some(reference) => {
            let attributes = required_record_view(pending.input, reference)?;
            if attributes.rows() != pending.counts.attributes.as_usize()
                || attributes.fields() != GEOMETRY_ATTRIBUTE_FIELD_COUNT
            {
                return Err(StoreRuntimeError::InvalidSourceSlot);
            }
            for row in 0..attributes.rows() {
                collector.observe(assembly, keys, work)?;
                collector.push(
                    attributes
                        .sequence_ref::<f32, GeometryF32ValuesSchema>(
                            row,
                            geometry_attribute_field::VALUES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometryF32Values),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<u32, GeometryU32ValuesSchema>(
                            row,
                            geometry_attribute_field::VALUES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometryU32Values),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<i32, GeometryI32ValuesSchema>(
                            row,
                            geometry_attribute_field::VALUES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometryI32Values),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<u32, GeometrySparseIndicesSchema>(
                            row,
                            geometry_attribute_field::SPARSE_INDICES,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometrySparseIndices),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<f32, GeometrySparseF32DefaultsSchema>(
                            row,
                            geometry_attribute_field::SPARSE_DEFAULT,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometrySparseF32Defaults),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<u32, GeometrySparseU32DefaultsSchema>(
                            row,
                            geometry_attribute_field::SPARSE_DEFAULT,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometrySparseU32Defaults),
                )?;
                collector.push(
                    attributes
                        .sequence_ref::<i32, GeometrySparseI32DefaultsSchema>(
                            row,
                            geometry_attribute_field::SPARSE_DEFAULT,
                        )
                        .map(CanonicalBlockRef::from_store)
                        .map(NodeSceneInheritedLeaf::GeometrySparseI32Defaults),
                )?;
            }
        }
        None => return Err(StoreRuntimeError::InvalidSourceSlot),
    }
    Ok(())
}

fn collect_profile_leaves(
    assembly: &mut SceneStoreAssembly<baozi_operation::CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    collector: &mut InheritedLeafCollector<'_>,
) -> Result<(), StoreRuntimeError> {
    let segments = required_record_view(pending.input, pending.input.proof.profile_segments())?;
    if segments.rows() != pending.counts.profile.source_segment_count.as_usize()
        || segments.fields() != PROFILE_SEGMENT_FIELD_COUNT
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    for row in 0..segments.rows() {
        collector.observe(assembly, keys, work)?;
        collector.push(
            segments
                .sequence_ref::<u32, ProfileLineageResourceSchema>(
                    row,
                    profile_segment_field::RESOURCES,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileResources),
        )?;
        collector.push(
            segments
                .sequence_ref::<u32, ProfileLineageSourceSchema>(
                    row,
                    profile_segment_field::SOURCES,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileSources),
        )?;
        collector.push(
            segments
                .sequence_ref::<u32, ProfileLineageProvenanceSchema>(
                    row,
                    profile_segment_field::PROVENANCE_NODES,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileProvenance),
        )?;
        collector.push(
            segments
                .sequence_ref::<u32, ProfileLineageOperationSchema>(
                    row,
                    profile_segment_field::OPERATIONS,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileOperations),
        )?;
        collector.push(
            segments
                .sequence_ref::<u32, ProfileLineageEvidenceSchema>(
                    row,
                    profile_segment_field::EVIDENCE,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileEvidence),
        )?;
        collector.push(
            segments
                .sequence_ref::<u64, ProfileLineageParentSchema>(
                    row,
                    profile_segment_field::PARENTS,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileParents),
        )?;
        collector.push(
            segments
                .sequence_ref::<u64, ProfileLineageWideSchema>(
                    row,
                    profile_segment_field::WIDE_VALUES,
                )
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileWide),
        )?;
        collector.push(
            segments
                .utf8_ref::<ProfileLineageTextSchema>(row, profile_segment_field::TEXTS)
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileText),
        )?;
        collector.push(
            segments
                .bytes_ref::<ProfileLineageDigestSchema>(row, profile_segment_field::DIGESTS)
                .map(CanonicalBlockRef::from_store)
                .map(NodeSceneInheritedLeaf::ProfileDigests),
        )?;
    }

    let metadata = required_record_view(
        pending.input,
        pending.input.proof.profile_metadata_directory(),
    )?;
    if metadata.rows() != 1 || metadata.fields() != PROFILE_METADATA_DIRECTORY_FIELD_COUNT {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    collector.observe(assembly, keys, work)?;
    collector.push(
        metadata
            .utf8_ref::<ProfileMetadataTextSchema>(0, profile_metadata_directory_field::TEXTS)
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataText),
    )?;
    collector.push(
        metadata
            .bytes_ref::<ProfileMetadataBytesSchema>(0, profile_metadata_directory_field::BYTES)
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataBytes),
    )?;
    collector.push(
        metadata
            .sequence_ref::<u8, ProfileMetadataBoolSchema>(
                0,
                profile_metadata_directory_field::BOOL_VALUES,
            )
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataBool),
    )?;
    collector.push(
        metadata
            .sequence_ref::<i64, ProfileMetadataI64Schema>(
                0,
                profile_metadata_directory_field::I64_VALUES,
            )
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataI64),
    )?;
    collector.push(
        metadata
            .sequence_ref::<u64, ProfileMetadataU64Schema>(
                0,
                profile_metadata_directory_field::U64_VALUES,
            )
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataU64),
    )?;
    collector.push(
        metadata
            .sequence_ref::<f64, ProfileMetadataF64Schema>(
                0,
                profile_metadata_directory_field::F64_VALUES,
            )
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataF64),
    )?;
    collector.push(
        metadata
            .sequence_ref::<f32, ProfileMetadataF32Schema>(
                0,
                profile_metadata_directory_field::F32_VALUES,
            )
            .map(CanonicalBlockRef::from_store)
            .map(NodeSceneInheritedLeaf::MetadataF32),
    )?;
    Ok(())
}

fn direct_inherited_leaf_count(input: &ValidatedNodeScene) -> usize {
    let proof = input.proof;
    [
        proof.node_names().is_some(),
        proof.tangent_basis_schema_name().is_some(),
        proof.tangent_encoding_schema_name().is_some(),
        proof.appearance_image_payloads().is_some(),
        proof.appearance_media_types().is_some(),
        proof.appearance_source_keys().is_some(),
        proof.appearance_binding_semantic_scalars().is_some(),
        proof.appearance_material_factor_scalars().is_some(),
        proof.appearance_material_names().is_some(),
        proof.logical_mesh_names().is_some(),
        proof.attribute_semantic_names().is_some(),
        proof.deformation_joint_nodes().is_some(),
        proof.deformation_inverse_bind_matrices().is_some(),
        proof.deformation_skin_correction_matrices().is_some(),
        proof.deformation_skin_names().is_some(),
        proof.deformation_influence_row_offsets().is_some(),
        proof.deformation_influence_joints().is_some(),
        proof.deformation_influence_weights().is_some(),
        proof.deformation_influence_source_orders().is_some(),
        proof.deformation_morph_defaults().is_some(),
        proof.deformation_morph_names().is_some(),
        proof.deformation_morph_values().is_some(),
        proof.deformation_morph_sparse_indices().is_some(),
        proof.deformation_instance_morph_weights().is_some(),
        proof.animation_times().is_some(),
        proof.animation_values().is_some(),
        proof.animation_in_tangents().is_some(),
        proof.animation_out_tangents().is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count()
}

#[cfg(test)]
mod tests {
    use super::super::plan::NodeSceneEditPlanError;
    use super::NodeSceneInheritedLeafScanPlan;

    #[test]
    fn inherited_leaf_scan_plan_seals_the_structural_upper_and_exact_work() {
        let plan = NodeSceneInheritedLeafScanPlan::try_from_counts(4, 14, 2, 3, 1)
            .expect("an exact inherited-leaf scan must seal");

        assert_eq!(plan.expected_dynamic(), 10);
        assert_eq!(plan.structural_upper(), 29);
        assert_eq!(plan.inspection_work(), 7);
        assert_eq!(plan.capacity().desired_capacity(), 10);
        assert!(matches!(
            NodeSceneInheritedLeafScanPlan::try_from_counts(4, 34, 2, 3, 1),
            Err(NodeSceneEditPlanError::InvalidSource { .. })
        ));
    }

    #[test]
    fn inherited_leaf_scan_plan_checks_zero_shape_and_arithmetic_overflow() {
        let empty = NodeSceneInheritedLeafScanPlan::try_from_counts(0, 0, 0, 0, 0)
            .expect("metadata inspection remains mandatory for an empty dynamic shape");

        assert_eq!(empty.expected_dynamic(), 0);
        assert_eq!(empty.structural_upper(), 7);
        assert_eq!(empty.inspection_work(), 1);
        assert!(matches!(
            NodeSceneInheritedLeafScanPlan::try_from_counts(0, 0, usize::MAX, 0, 0),
            Err(NodeSceneEditPlanError::SizeOverflow { .. })
        ));
    }
}
