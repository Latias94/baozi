mod profile;

use baozi_operation::{
    CanonicalInheritanceSource, OutcomeEvidenceRecordRef, RecordBlock, RecordPlan,
    RetainedStoreWorkControl, SequencePlan, SequenceWriter, StoreBuildError, StoreControlledError,
    StoreRuntimeError,
};

use crate::canonical_scene::store::schema::animation::{
    AnimationChannelTableSchema, AnimationClipTableSchema, AnimationDirectorySchema,
};
use crate::canonical_scene::store::schema::appearance::{
    AppearanceBindingTableSchema, AppearanceDirectorySchema, AppearanceImageTableSchema,
    AppearanceMaterialTableSchema, AppearanceSamplerTableSchema, AppearanceTextureTableSchema,
};
use crate::canonical_scene::store::schema::deformation::{
    DeformationDirectorySchema, DeformationLogicalMorphTableSchema,
    DeformationMeshInstanceTableSchema, DeformationMorphDefinitionTableSchema,
    DeformationMorphDeltaTableSchema, DeformationMorphTargetTableSchema,
    DeformationPrimitiveTableSchema, DeformationSkinTableSchema,
};
use crate::canonical_scene::store::schema::geometry::{
    GeometryAttributeTableSchema, GeometryLogicalMeshTableSchema, GeometryMeshInstanceTableSchema,
    GeometryPrimitiveTableSchema,
};
use crate::canonical_scene::store::schema::profile::ProfileDirectorySchema;
use crate::canonical_scene::store::schema::spatial::{
    SpatialCameraTableSchema, SpatialDirectorySchema, SpatialLightTableSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_EVIDENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE, NODE_SCENE_NODE_FIELD_COUNT,
    NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_TRS, NodeSceneAuthoredTransformSchema,
    NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
    SceneGeometryDirectorySchema, node_scene_node_field,
};
use crate::entity::{AuthoredTransform, AuthoredTransformKind};

use super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneWorkMeter, SceneStoreAssembly, SceneStoreKeys,
    ValidatedNodeScene, process_slice_in_work_chunks,
};
use super::leaf_plan::{NodeSceneInheritedLeafPlan, inherit_optional_leaf};
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditProgram, NodeTransformRewrite};
use super::required_record_view;

pub(super) struct RewrittenNodeSceneGraph {
    pub(super) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(super) evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    pub(super) work: NodeSceneWorkMeter,
}

pub(super) enum NodeSceneRewriteError {
    BeforeEvidence(StoreRuntimeError),
    AfterEvidence {
        evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
        cause: StoreRuntimeError,
    },
}

pub(super) fn try_rewrite_node_scene<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    keys: SceneStoreKeys,
) -> Result<RewrittenNodeSceneGraph, NodeSceneRewriteError> {
    let evidence =
        retain_edit_evidence(assembly, pending).map_err(NodeSceneRewriteError::BeforeEvidence)?;
    assembly
        .try_commit_outcome_evidence(keys.dimension, 1)
        .map_err(NodeSceneRewriteError::BeforeEvidence)?;
    let rewritten =
        (|| -> Result<(CanonicalBlockRef<SceneDirectorySchema>, NodeSceneWorkMeter), StoreRuntimeError> {
            assembly.try_bind_canonical_source(&pending.input.canonical, keys.dimension)?;

            let mut work = NodeSceneWorkMeter::new();
            let inherited_leaf_plan =
                NodeSceneInheritedLeafPlan::try_collect(assembly, pending, keys, &mut work)?;
            let node_transform = match &pending.program {
                NodeSceneEditProgram::NodeTransform(rewrite) => *rewrite,
            };
            let transforms =
                rewrite_transforms(assembly, pending, node_transform, keys, &mut work)?;
            let profile_leaves =
                profile::rewrite_lineage_leaves(assembly, pending, keys, &mut work)?;
            inherit_unchanged_evidence_leaves(assembly, pending, &inherited_leaf_plan)?;
            drop(inherited_leaf_plan);

            let proof = pending.input.proof;
            let spatial = rewrite_spatial(assembly, pending, keys)?;
            let references = rewrite_optional_record(
                assembly,
                pending.input,
                proof.references(),
                pending.counts.source_reference_rows.as_usize(),
                crate::canonical_scene::store::schema::NodeSceneChildReferenceTableSchema,
                keys,
            )?;
            let shape = pending.program.shape();
            let nodes = if shape.rewrites_node_table() {
                rewrite_nodes_with_shape(
                    assembly,
                    pending,
                    node_transform,
                    transforms,
                    spatial,
                    keys,
                    &mut work,
                )?
            } else {
                rewrite_required_record(
                    assembly,
                    pending.input,
                    proof.nodes(),
                    pending.counts.canonical_node_rows.as_usize(),
                    NodeSceneNodeTableSchema,
                    keys,
                )?
            };

            rewrite_appearance(assembly, pending, keys)?;
            let appearance = rewrite_required_record(
                assembly,
                pending.input,
                proof.appearance_directory(),
                1,
                AppearanceDirectorySchema,
                keys,
            )?;
            rewrite_geometry(assembly, pending, keys)?;
            let geometry = rewrite_required_record(
                assembly,
                pending.input,
                proof.geometry_directory(),
                1,
                SceneGeometryDirectorySchema,
                keys,
            )?;
            rewrite_deformation(assembly, pending, keys)?;
            let deformation = rewrite_required_record(
                assembly,
                pending.input,
                proof.deformation_directory(),
                1,
                DeformationDirectorySchema,
                keys,
            )?;
            rewrite_animation(assembly, pending, keys)?;
            let animation = rewrite_required_record(
                assembly,
                pending.input,
                proof.animation_directory(),
                1,
                AnimationDirectorySchema,
                keys,
            )?;
            let profile_segments =
                profile::rewrite_segments(assembly, pending, profile_leaves, keys, &mut work)?;
            let profile = profile::rewrite_profile_records(
                assembly,
                pending,
                profile_segments,
                profile_leaves,
                keys,
                &mut work,
            )?;

            let directory = if shape.rewrites_scene_directory() {
                let references = references.ok_or(StoreRuntimeError::InvalidSourceSlot)?;
                rewrite_directory_with_shape(
                    assembly,
                    pending,
                    NodeSceneDirectoryRefs {
                        nodes,
                        references,
                        transforms,
                        spatial: spatial.directory,
                        appearance,
                        geometry,
                        deformation,
                        animation,
                        profile,
                    },
                    keys,
                    &mut work,
                )?
            } else {
                rewrite_required_record(
                    assembly,
                    pending.input,
                    proof.directory(),
                    1,
                    SceneDirectorySchema,
                    keys,
                )?
            };

            Ok((directory, work))
        })();
    match rewritten {
        Ok((directory, work)) => Ok(RewrittenNodeSceneGraph {
            directory,
            evidence,
            work,
        }),
        Err(cause) => Err(NodeSceneRewriteError::AfterEvidence { evidence, cause }),
    }
}

fn retain_edit_evidence(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
) -> Result<OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>, StoreRuntimeError> {
    let counts = pending.counts.evidence;
    assembly.try_retain_outcome_evidence_records(
        RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
        NodeSceneOutcomeEvidenceSchema,
        |writer| {
            writer.write_row(|row| {
                row.write_scalar(SCENE_SCHEMA_REVISION)?;
                row.write_scalar(NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE)?;
                row.write_scalar(counts.canonical_block_count)?;
                row.write_scalar(counts.node_count)?;
                row.write_scalar(counts.camera_count)?;
                row.write_scalar(counts.light_count)?;
                row.write_scalar(counts.image_count)?;
                row.write_scalar(counts.sampler_count)?;
                row.write_scalar(counts.texture_count)?;
                row.write_scalar(counts.material_count)?;
                row.write_scalar(counts.material_binding_count)?;
                row.write_scalar(counts.logical_mesh_count)?;
                row.write_scalar(counts.primitive_count)?;
                row.write_scalar(counts.attribute_count)?;
                row.write_scalar(counts.mesh_instance_count)?;
                row.write_scalar(counts.skin_count)?;
                row.write_scalar(counts.logical_morph_count)?;
                row.write_scalar(counts.morph_definition_count)?;
                row.write_scalar(counts.primitive_deformation_count)?;
                row.write_scalar(counts.morph_target_count)?;
                row.write_scalar(counts.morph_delta_count)?;
                row.write_scalar(counts.animation_clip_count)?;
                row.write_scalar(counts.animation_channel_count)?;
                row.write_scalar(counts.payload_block_count)?;
                row.write_scalar(counts.profile_segment_count)?;
                row.write_scalar(counts.profile_resource_count)?;
                row.write_scalar(counts.profile_source_count)?;
                row.write_scalar(counts.profile_provenance_count)?;
                row.write_scalar(counts.profile_operation_count)?;
                row.write_scalar(counts.profile_parent_count)?;
                row.write_scalar(counts.profile_owner_count)?;
                row.write_scalar(counts.profile_attachment_count)?;
                row.write_scalar(counts.profile_metadata_entry_count)?;
                row.write_scalar(counts.shareable_leaf_block_count)
            })
        },
    )
}

fn rewrite_transforms(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    transform: NodeTransformRewrite,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>, StoreRuntimeError> {
    let target = transform.target();
    let source_ref = pending
        .input
        .proof
        .transforms()
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let source = pending
        .input
        .canonical
        .sequence(source_ref.into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let source_values = source.as_slice();
    if source_values.len() != pending.counts.source_transform_scalars.as_usize() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let start = target.transform_offset.as_usize();
    let end = start
        .checked_add(target.old_transform_scalars.as_usize())
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    let prefix = source_values
        .get(..start)
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let suffix = source_values
        .get(end..)
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let replacement = transform.replacement();
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    let plan = SequencePlan::<f32>::new(pending.counts.output_transform_scalars.as_usize())
        .map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_rewrite_canonical_sequence_controlled(
            &pending.input.canonical,
            source_ref.into_store(),
            plan,
            keys.dimension,
            NodeSceneAuthoredTransformSchema,
            |writer, control| {
                write_scalar_slice(writer, control, work, prefix, failure)?;
                write_authored_transform(writer, control, work, replacement, failure)?;
                write_scalar_slice(writer, control, work, suffix, failure)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn write_scalar_slice(
    writer: &mut SequenceWriter<f32>,
    control: &mut RetainedStoreWorkControl<
        '_,
        super::super::super::SceneConstructionOwner,
        super::super::super::SceneConstructionPhase,
        super::super::super::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    values: &[f32],
    failure: StoreBuildError,
) -> Result<(), StoreControlledError> {
    process_slice_in_work_chunks(values, control, work, failure, |chunk| {
        writer.extend_from_slice(chunk)
    })
}

fn write_authored_transform(
    writer: &mut SequenceWriter<f32>,
    control: &mut RetainedStoreWorkControl<
        '_,
        super::super::super::SceneConstructionOwner,
        super::super::super::SceneConstructionPhase,
        super::super::super::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    transform: AuthoredTransform,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError> {
    match transform.kind() {
        AuthoredTransformKind::Matrix => {
            let matrix = transform.authored_matrix().ok_or(failure)?;
            for column in matrix.cols {
                write_scalar_slice(writer, control, work, &column, failure)?;
            }
        }
        AuthoredTransformKind::Trs => {
            let trs = transform.authored_trs().ok_or(failure)?;
            let translation = trs.translation();
            let rotation = trs.rotation();
            let scale = trs.scale();
            write_scalar_slice(
                writer,
                control,
                work,
                &[
                    translation.x,
                    translation.y,
                    translation.z,
                    rotation.x,
                    rotation.y,
                    rotation.z,
                    rotation.w,
                    scale.x,
                    scale.y,
                    scale.z,
                ],
                failure,
            )?;
        }
    }
    Ok(())
}

fn rewrite_nodes_with_shape(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    transform: NodeTransformRewrite,
    transforms: CanonicalBlockRef<NodeSceneAuthoredTransformSchema>,
    spatial: RewrittenSpatialRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneNodeTableSchema>, StoreRuntimeError> {
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    let target = transform.target();
    let source_ref = pending.input.proof.nodes();
    let source = required_record_view(pending.input, source_ref)?;
    if source.rows() != pending.counts.canonical_node_rows.as_usize()
        || source.fields() != NODE_SCENE_NODE_FIELD_COUNT
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let target_row = target.canonical_row.as_usize();
    let target_kind = transform_kind(transform.replacement());
    let target_length = target.new_transform_scalars.as_u32();
    let old_length = target.old_transform_scalars.as_u32();
    let plan = RecordPlan::new(source.rows(), source.fields()).map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_rewrite_canonical_records_with_controlled(
            &pending.input.canonical,
            source_ref.into_store(),
            plan,
            keys.dimension,
            NodeSceneNodeTableSchema,
            |writer, control| {
                for row_index in 0..source.rows() {
                    work.observe(control, 1)?;
                    let node = source
                        .scalar::<u32>(row_index, node_scene_node_field::NODE)
                        .ok_or(failure)?;
                    let presence = source
                        .scalar::<u8>(row_index, node_scene_node_field::CHILDREN_PRESENCE)
                        .ok_or(failure)?;
                    let child_offset = source
                        .scalar::<u32>(row_index, node_scene_node_field::CHILD_REFERENCE_OFFSET)
                        .ok_or(failure)?;
                    let child_count = source
                        .scalar::<u32>(row_index, node_scene_node_field::CHILD_REFERENCE_COUNT)
                        .ok_or(failure)?;
                    let source_kind = source
                        .scalar::<u8>(row_index, node_scene_node_field::TRANSFORM_KIND)
                        .ok_or(failure)?;
                    let source_offset = source
                        .scalar::<u32>(row_index, node_scene_node_field::TRANSFORM_OFFSET)
                        .ok_or(failure)?;
                    let source_length = source
                        .scalar::<u32>(row_index, node_scene_node_field::TRANSFORM_LENGTH)
                        .ok_or(failure)?;
                    let name_presence = source
                        .scalar::<u8>(row_index, node_scene_node_field::NAME_PRESENCE)
                        .ok_or(failure)?;
                    let name_offset = source
                        .scalar::<u32>(row_index, node_scene_node_field::NAME_OFFSET)
                        .ok_or(failure)?;
                    let name_count = source
                        .scalar::<u32>(row_index, node_scene_node_field::NAME_COUNT)
                        .ok_or(failure)?;
                    let camera_presence = source
                        .scalar::<u8>(row_index, node_scene_node_field::CAMERA_PRESENCE)
                        .ok_or(failure)?;
                    let camera = match camera_presence {
                        PRESENCE_ABSENT => (source
                            .scalar::<u8>(row_index, node_scene_node_field::CAMERA)
                            == Some(0))
                        .then_some(None)
                        .ok_or(failure)?,
                        PRESENCE_PRESENT => Some(
                            source
                                .scalar::<u32>(row_index, node_scene_node_field::CAMERA)
                                .ok_or(failure)?,
                        ),
                        _ => return Err(StoreControlledError::from(failure)),
                    };
                    let light_presence = source
                        .scalar::<u8>(row_index, node_scene_node_field::LIGHT_PRESENCE)
                        .ok_or(failure)?;
                    let light = match light_presence {
                        PRESENCE_ABSENT => (source
                            .scalar::<u8>(row_index, node_scene_node_field::LIGHT)
                            == Some(0))
                        .then_some(None)
                        .ok_or(failure)?,
                        PRESENCE_PRESENT => Some(
                            source
                                .scalar::<u32>(row_index, node_scene_node_field::LIGHT)
                                .ok_or(failure)?,
                        ),
                        _ => return Err(StoreControlledError::from(failure)),
                    };
                    let (kind, offset, length) = if row_index == target_row {
                        (target_kind, source_offset, target_length)
                    } else if row_index > target_row {
                        (
                            source_kind,
                            shifted_offset(source_offset, old_length, target_length)
                                .ok_or(failure)?,
                            source_length,
                        )
                    } else {
                        (source_kind, source_offset, source_length)
                    };
                    writer.write_row(|row| {
                        row.write_scalar(node)?;
                        row.write_scalar(presence)?;
                        row.write_scalar(child_offset)?;
                        row.write_scalar(child_count)?;
                        row.write_scalar(kind)?;
                        row.write_scalar(offset)?;
                        row.write_scalar(length)?;
                        if row_index == 0 {
                            row.write_scalar(0_u8)?;
                        } else {
                            row.write_sequence_ref(transforms.into_store())?;
                        }
                        row.write_scalar(name_presence)?;
                        row.write_scalar(name_offset)?;
                        row.write_scalar(name_count)?;
                        if name_count == 0 {
                            row.write_scalar(0_u8)?;
                        } else {
                            row.write_utf8_ref(
                                pending
                                    .input
                                    .proof
                                    .node_names()
                                    .ok_or(failure)?
                                    .into_store(),
                            )?;
                        }
                        row.write_scalar(camera_presence)?;
                        match camera {
                            None => {
                                row.write_scalar(0_u8)?;
                                row.write_scalar(0_u8)?;
                            }
                            Some(camera) => {
                                row.write_scalar(camera)?;
                                row.write_record_ref(spatial.cameras.ok_or(failure)?.into_store())?
                            }
                        }
                        row.write_scalar(light_presence)?;
                        match light {
                            None => {
                                row.write_scalar(0_u8)?;
                                row.write_scalar(0_u8)
                            }
                            Some(light) => {
                                row.write_scalar(light)?;
                                row.write_record_ref(spatial.lights.ok_or(failure)?.into_store())
                            }
                        }
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[derive(Clone, Copy)]
struct NodeSceneDirectoryRefs {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: CanonicalBlockRef<
        crate::canonical_scene::store::schema::NodeSceneChildReferenceTableSchema,
    >,
    transforms: CanonicalBlockRef<NodeSceneAuthoredTransformSchema>,
    spatial: CanonicalBlockRef<SpatialDirectorySchema>,
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
    geometry: CanonicalBlockRef<SceneGeometryDirectorySchema>,
    deformation: CanonicalBlockRef<DeformationDirectorySchema>,
    animation: CanonicalBlockRef<AnimationDirectorySchema>,
    profile: CanonicalBlockRef<ProfileDirectorySchema>,
}

fn rewrite_directory_with_shape(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    refs: NodeSceneDirectoryRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<SceneDirectorySchema>, StoreRuntimeError> {
    let source_ref = pending.input.proof.directory();
    let source = required_record_view(pending.input, source_ref)?;
    if source.rows() != 1 || source.fields() != SCENE_DIRECTORY_FIELD_COUNT {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let plan = RecordPlan::new(1, SCENE_DIRECTORY_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_rewrite_canonical_records_with_controlled(
            &pending.input.canonical,
            source_ref.into_store(),
            plan,
            keys.dimension,
            SceneDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(SCENE_SCHEMA_REVISION)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(pending.counts.canonical_node_rows.as_u32())?;
                        row.write_record_ref(refs.nodes.into_store())?;
                        row.write_scalar(PRESENCE_PRESENT)?;
                        row.write_scalar(pending.counts.source_reference_rows.as_u32())?;
                        row.write_record_ref(refs.references.into_store())?;
                        row.write_scalar(PRESENCE_PRESENT)?;
                        row.write_scalar(pending.counts.output_transform_scalars.as_u32())?;
                        row.write_sequence_ref(refs.transforms.into_store())?;
                        row.write_scalar(pending.counts.source_root_rows.as_u32())?;
                        row.write_record_ref(refs.spatial.into_store())?;
                        row.write_record_ref(refs.appearance.into_store())?;
                        row.write_record_ref(refs.geometry.into_store())?;
                        row.write_record_ref(refs.deformation.into_store())?;
                        row.write_record_ref(refs.animation.into_store())?;
                        row.write_record_ref(refs.profile.into_store())
                    })
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn inherit_unchanged_evidence_leaves<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    inherited_leaf_plan: &NodeSceneInheritedLeafPlan<
        baozi_operation::RetainedStoreReservation<super::super::SceneConstructionOwner>,
    >,
) -> Result<(), StoreRuntimeError> {
    let proof = pending.input.proof;
    let mut inherited = 0_usize;

    inherit_optional_leaf(assembly, pending.input, proof.node_names(), &mut inherited)?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.tangent_basis_schema_name(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.tangent_encoding_schema_name(),
        &mut inherited,
    )?;

    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_image_payloads(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_media_types(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_source_keys(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_binding_semantic_scalars(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_material_factor_scalars(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.appearance_material_names(),
        &mut inherited,
    )?;

    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.logical_mesh_names(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.attribute_semantic_names(),
        &mut inherited,
    )?;

    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_joint_nodes(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_inverse_bind_matrices(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_skin_correction_matrices(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_skin_names(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_influence_row_offsets(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_influence_joints(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_influence_weights(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_influence_source_orders(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_morph_defaults(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_morph_names(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_morph_values(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_morph_sparse_indices(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.deformation_instance_morph_weights(),
        &mut inherited,
    )?;

    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.animation_times(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.animation_values(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.animation_in_tangents(),
        &mut inherited,
    )?;
    inherit_optional_leaf(
        assembly,
        pending.input,
        proof.animation_out_tangents(),
        &mut inherited,
    )?;

    inherited_leaf_plan.inherit(assembly, pending.input, &mut inherited)?;
    if inherited != pending.counts.evidence_leaves.inherited.as_usize() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RewrittenSpatialRefs {
    directory: CanonicalBlockRef<SpatialDirectorySchema>,
    cameras: Option<CanonicalBlockRef<SpatialCameraTableSchema>>,
    lights: Option<CanonicalBlockRef<SpatialLightTableSchema>>,
}

fn rewrite_spatial(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
) -> Result<RewrittenSpatialRefs, StoreRuntimeError> {
    let input = pending.input;
    let proof = input.proof;
    let cameras = rewrite_optional_record(
        assembly,
        input,
        proof.cameras(),
        pending.counts.cameras.as_usize(),
        SpatialCameraTableSchema,
        keys,
    )?;
    let lights = rewrite_optional_record(
        assembly,
        input,
        proof.lights(),
        pending.counts.lights.as_usize(),
        SpatialLightTableSchema,
        keys,
    )?;
    let directory = rewrite_required_record(
        assembly,
        input,
        proof.spatial_directory(),
        1,
        SpatialDirectorySchema,
        keys,
    )?;
    Ok(RewrittenSpatialRefs {
        directory,
        cameras,
        lights,
    })
}

fn rewrite_appearance(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
) -> Result<(), StoreRuntimeError> {
    let input = pending.input;
    let proof = input.proof;
    rewrite_optional_record(
        assembly,
        input,
        proof.appearance_images(),
        pending.counts.images.as_usize(),
        AppearanceImageTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.appearance_samplers(),
        pending.counts.samplers.as_usize(),
        AppearanceSamplerTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.appearance_textures(),
        pending.counts.textures.as_usize(),
        AppearanceTextureTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.appearance_bindings(),
        pending.counts.material_bindings.as_usize(),
        AppearanceBindingTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.materials(),
        pending.counts.materials.as_usize(),
        AppearanceMaterialTableSchema,
        keys,
    )?;
    Ok(())
}

fn rewrite_geometry(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
) -> Result<(), StoreRuntimeError> {
    let input = pending.input;
    let proof = input.proof;
    rewrite_optional_record(
        assembly,
        input,
        proof.attributes(),
        pending.counts.attributes.as_usize(),
        GeometryAttributeTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.primitives(),
        pending.counts.primitives.as_usize(),
        GeometryPrimitiveTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.logical_meshes(),
        pending.counts.logical_meshes.as_usize(),
        GeometryLogicalMeshTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.mesh_instances(),
        pending.counts.mesh_instances.as_usize(),
        GeometryMeshInstanceTableSchema,
        keys,
    )?;
    Ok(())
}

fn rewrite_deformation(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
) -> Result<(), StoreRuntimeError> {
    let input = pending.input;
    let proof = input.proof;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_skins(),
        pending.counts.skins.as_usize(),
        DeformationSkinTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_morph_definitions(),
        pending.counts.morph_definitions.as_usize(),
        DeformationMorphDefinitionTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_morph_deltas(),
        pending.counts.morph_deltas.as_usize(),
        DeformationMorphDeltaTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_morph_targets(),
        pending.counts.morph_targets.as_usize(),
        DeformationMorphTargetTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_primitives(),
        pending.counts.primitive_deformations.as_usize(),
        DeformationPrimitiveTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_logical_morphs(),
        pending.counts.logical_morphs.as_usize(),
        DeformationLogicalMorphTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.deformation_mesh_instances(),
        pending.counts.mesh_instances.as_usize(),
        DeformationMeshInstanceTableSchema,
        keys,
    )?;
    Ok(())
}

fn rewrite_animation(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    keys: SceneStoreKeys,
) -> Result<(), StoreRuntimeError> {
    let input = pending.input;
    let proof = input.proof;
    rewrite_optional_record(
        assembly,
        input,
        proof.animation_channels(),
        pending.counts.animation_channels.as_usize(),
        AnimationChannelTableSchema,
        keys,
    )?;
    rewrite_optional_record(
        assembly,
        input,
        proof.animation_clips(),
        pending.counts.animation_clips.as_usize(),
        AnimationClipTableSchema,
        keys,
    )?;
    Ok(())
}

fn rewrite_optional_record<Schema>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    input: &ValidatedNodeScene,
    source: Option<CanonicalBlockRef<Schema>>,
    expected_rows: usize,
    tag: Schema,
    keys: SceneStoreKeys,
) -> Result<Option<CanonicalBlockRef<Schema>>, StoreRuntimeError>
where
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (source, expected_rows) {
        (None, 0) => Ok(None),
        (Some(source), rows) if rows != 0 => {
            rewrite_required_record(assembly, input, source, rows, tag, keys).map(Some)
        }
        _ => Err(StoreRuntimeError::InvalidSourceSlot),
    }
}

fn rewrite_required_record<Schema>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    input: &ValidatedNodeScene,
    source: CanonicalBlockRef<Schema>,
    expected_rows: usize,
    tag: Schema,
    keys: SceneStoreKeys,
) -> Result<CanonicalBlockRef<Schema>, StoreRuntimeError>
where
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    let view = required_record_view(input, source)?;
    if view.rows() != expected_rows {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let plan = RecordPlan::new(view.rows(), view.fields()).map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_rewrite_canonical_records_controlled(
            &input.canonical,
            source.into_store(),
            plan,
            keys.dimension,
            tag,
        )
        .map(CanonicalBlockRef::from_store)
}

fn transform_kind(transform: AuthoredTransform) -> u8 {
    match transform.kind() {
        AuthoredTransformKind::Matrix => NODE_SCENE_TRANSFORM_MATRIX,
        AuthoredTransformKind::Trs => NODE_SCENE_TRANSFORM_TRS,
    }
}

fn shifted_offset(source: u32, old_length: u32, new_length: u32) -> Option<u32> {
    if new_length >= old_length {
        source.checked_add(new_length - old_length)
    } else {
        source.checked_sub(old_length - new_length)
    }
}
