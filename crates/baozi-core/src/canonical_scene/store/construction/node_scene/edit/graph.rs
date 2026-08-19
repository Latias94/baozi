mod profile;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalBuildControl, CanonicalLane,
};
use baozi_operation::{
    CanonicalInheritanceSource, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock,
    RecordPlan, SequencePlan, SequenceWriter, StoreBuildError, StoreControlledError,
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
    SpatialTangentBasisSchemaNameSchema, SpatialTangentEncodingSchemaNameSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_TRANSFORM_MATRIX,
    NODE_SCENE_TRANSFORM_TRS, NodeSceneAuthoredTransformSchema, NodeSceneNodeNameSchema,
    NodeSceneNodeTableSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT,
    SCENE_SCHEMA_REVISION, SceneDirectorySchema, SceneGeometryDirectorySchema,
    node_scene_node_field,
};
use crate::entity::{AuthoredTransform, AuthoredTransformKind};

use super::super::animation::{AnimationPayloadRefs, RetainedAnimationRefs};
use super::super::appearance::{
    AppearancePayloadRefs, AppearanceTableRefs, RetainedAppearanceRefs,
};
use super::super::deformation::{
    DeformationPayloadRefs, DeformationTableRefs, RetainedDeformationRefs,
};
use super::super::geometry::{GeometryPayloadRefs, GeometryTableRefs, RetainedGeometryRefs};
use super::super::materialization::RetainedSpatialRefs;
use super::super::profile::MetadataLeafRefs;
use super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneMaterializedRefs, ValidatedNodeScene,
};
use super::leaf_plan::{NodeSceneInheritedLeafPlan, inherit_optional_leaf};
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditProgram, NodeTransformRewrite};
use super::required_record_view;

pub(super) struct RewrittenNodeSceneGraph {
    pub(super) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(super) refs: NodeSceneMaterializedRefs,
}

#[derive(Clone, Copy)]
struct InheritedNodeSceneLeafRefs {
    node_names: Option<CanonicalBlockRef<NodeSceneNodeNameSchema>>,
    tangent_basis_schema_name: Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>>,
    tangent_encoding_schema_name: Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>>,
    appearance: AppearancePayloadRefs,
    geometry: GeometryPayloadRefs,
    deformation: DeformationPayloadRefs,
    animation: AnimationPayloadRefs,
    metadata: MetadataLeafRefs,
}

pub(super) fn try_rewrite_node_scene<'lane, 'scope, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    inherited_leaf_plan: &NodeSceneInheritedLeafPlan<'scope, O>,
) -> Result<RewrittenNodeSceneGraph, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_bind_canonical_source(&pending.input.canonical)?;

    let node_transform = match &pending.program {
        NodeSceneEditProgram::NodeTransform(rewrite) => *rewrite,
    };
    let transforms = rewrite_transforms(lane, pending, node_transform)?;
    let profile_leaves = profile::rewrite_lineage_leaves(lane, pending)?;
    let inherited = inherit_unchanged_evidence_leaves(lane, pending, inherited_leaf_plan)?;

    let proof = pending.input.proof;
    let spatial = rewrite_spatial(lane, pending, inherited)?;
    let references = rewrite_optional_record(
        lane,
        pending.input,
        proof.references(),
        pending.counts.source_reference_rows.as_usize(),
        crate::canonical_scene::store::schema::NodeSceneChildReferenceTableSchema,
    )?;
    let shape = pending.program.shape();
    let nodes = if shape.rewrites_node_table() {
        rewrite_nodes_with_shape(
            lane,
            pending,
            node_transform,
            transforms,
            inherited.node_names,
            spatial,
        )?
    } else {
        rewrite_required_record(
            lane,
            pending.input,
            proof.nodes(),
            pending.counts.canonical_node_rows.as_usize(),
            NodeSceneNodeTableSchema,
        )?
    };

    let appearance = rewrite_appearance(lane, pending, inherited.appearance)?;
    let geometry = rewrite_geometry(lane, pending, inherited.geometry)?;
    let deformation = rewrite_deformation(lane, pending, inherited.deformation)?;
    let animation = rewrite_animation(lane, pending, inherited.animation)?;
    let profile_segments = profile::rewrite_segments(lane, pending, profile_leaves)?;
    let profile = profile::rewrite_profile_records(
        lane,
        pending,
        profile_segments,
        profile_leaves,
        inherited.metadata,
    )?;

    let directory = if shape.rewrites_scene_directory() {
        let references = references.ok_or(StoreRuntimeError::InvalidSourceSlot)?;
        rewrite_directory_with_shape(
            lane,
            pending,
            NodeSceneDirectoryRefs {
                nodes,
                references,
                transforms,
                spatial: spatial.directory,
                appearance: appearance.directory,
                geometry: geometry.directory,
                deformation: deformation.directory,
                animation: animation.directory,
                profile: profile.directory,
            },
        )?
    } else {
        rewrite_required_record(
            lane,
            pending.input,
            proof.directory(),
            1,
            SceneDirectorySchema,
        )?
    };

    Ok(RewrittenNodeSceneGraph {
        directory,
        refs: NodeSceneMaterializedRefs {
            directory,
            nodes,
            references,
            transforms: Some(transforms),
            node_names: inherited.node_names,
            spatial,
            appearance,
            geometry,
            deformation,
            animation,
            profile,
        },
    })
}

fn rewrite_transforms<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    transform: NodeTransformRewrite,
) -> Result<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
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
    lane.try_rewrite_sequence(
        &pending.input.canonical,
        source_ref.into_store(),
        plan,
        NodeSceneAuthoredTransformSchema,
        |control| {
            write_scalar_slice(control, prefix)?;
            write_authored_transform(control, replacement, failure)?;
            write_scalar_slice(control, suffix)
        },
    )
    .map(CanonicalBlockRef::from_store)
}

fn write_scalar_slice<O, PP, U>(
    control: &mut CanonicalBuildControl<'_, '_, '_, O, PP, U, SequenceWriter<f32>>,
    values: &[f32],
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    control.extend_from_slice(values)
}

fn write_authored_transform<O, PP, U>(
    control: &mut CanonicalBuildControl<'_, '_, '_, O, PP, U, SequenceWriter<f32>>,
    transform: AuthoredTransform,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match transform.kind() {
        AuthoredTransformKind::Matrix => {
            let matrix = transform.authored_matrix().ok_or(failure)?;
            for column in matrix.cols {
                write_scalar_slice(control, &column)?;
            }
        }
        AuthoredTransformKind::Trs => {
            let trs = transform.authored_trs().ok_or(failure)?;
            let translation = trs.translation();
            let rotation = trs.rotation();
            let scale = trs.scale();
            write_scalar_slice(
                control,
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
            )?;
        }
    }
    Ok(())
}

fn rewrite_nodes_with_shape<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    transform: NodeTransformRewrite,
    transforms: CanonicalBlockRef<NodeSceneAuthoredTransformSchema>,
    node_names: Option<CanonicalBlockRef<NodeSceneNodeNameSchema>>,
    spatial: RetainedSpatialRefs,
) -> Result<CanonicalBlockRef<NodeSceneNodeTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
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
    lane.try_rewrite_records(
        &pending.input.canonical,
        source_ref.into_store(),
        plan,
        NodeSceneNodeTableSchema,
        |control| {
            for row_index in 0..source.rows() {
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
                    PRESENCE_ABSENT => {
                        (source.scalar::<u8>(row_index, node_scene_node_field::CAMERA) == Some(0))
                            .then_some(None)
                            .ok_or(failure)?
                    }
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
                    PRESENCE_ABSENT => {
                        (source.scalar::<u8>(row_index, node_scene_node_field::LIGHT) == Some(0))
                            .then_some(None)
                            .ok_or(failure)?
                    }
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
                        shifted_offset(source_offset, old_length, target_length).ok_or(failure)?,
                        source_length,
                    )
                } else {
                    (source_kind, source_offset, source_length)
                };
                control.write_row(|row| {
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
                        row.write_utf8_ref(node_names.ok_or(failure)?.into_store())?;
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

fn rewrite_directory_with_shape<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    refs: NodeSceneDirectoryRefs,
) -> Result<CanonicalBlockRef<SceneDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let source_ref = pending.input.proof.directory();
    let source = required_record_view(pending.input, source_ref)?;
    if source.rows() != 1 || source.fields() != SCENE_DIRECTORY_FIELD_COUNT {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let plan = RecordPlan::new(1, SCENE_DIRECTORY_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?;
    lane.try_rewrite_records(
        &pending.input.canonical,
        source_ref.into_store(),
        plan,
        SceneDirectorySchema,
        |control| {
            control.write_row(|row| {
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
        },
    )
    .map(CanonicalBlockRef::from_store)
}

fn inherit_unchanged_evidence_leaves<'lane, 'scope, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    inherited_leaf_plan: &NodeSceneInheritedLeafPlan<'scope, O>,
) -> Result<InheritedNodeSceneLeafRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let proof = pending.input.proof;
    let mut inherited = 0_usize;

    let node_names =
        inherit_optional_leaf(lane, pending.input, proof.node_names(), &mut inherited)?;
    let tangent_basis_schema_name = inherit_optional_leaf(
        lane,
        pending.input,
        proof.tangent_basis_schema_name(),
        &mut inherited,
    )?;
    let tangent_encoding_schema_name = inherit_optional_leaf(
        lane,
        pending.input,
        proof.tangent_encoding_schema_name(),
        &mut inherited,
    )?;

    let appearance = AppearancePayloadRefs {
        image_payloads: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_image_payloads(),
            &mut inherited,
        )?,
        media_types: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_media_types(),
            &mut inherited,
        )?,
        source_keys: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_source_keys(),
            &mut inherited,
        )?,
        binding_semantic_scalars: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_binding_semantic_scalars(),
            &mut inherited,
        )?,
        material_factor_scalars: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_material_factor_scalars(),
            &mut inherited,
        )?,
        material_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.appearance_material_names(),
            &mut inherited,
        )?,
    };

    let geometry = GeometryPayloadRefs {
        logical_mesh_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.logical_mesh_names(),
            &mut inherited,
        )?,
        attribute_semantic_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.attribute_semantic_names(),
            &mut inherited,
        )?,
    };

    let deformation = DeformationPayloadRefs {
        joint_nodes: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_joint_nodes(),
            &mut inherited,
        )?,
        inverse_bind_matrices: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_inverse_bind_matrices(),
            &mut inherited,
        )?,
        skin_correction_matrices: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_skin_correction_matrices(),
            &mut inherited,
        )?,
        skin_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_skin_names(),
            &mut inherited,
        )?,
        influence_row_offsets: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_influence_row_offsets(),
            &mut inherited,
        )?,
        influence_joints: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_influence_joints(),
            &mut inherited,
        )?,
        influence_weights: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_influence_weights(),
            &mut inherited,
        )?,
        influence_source_orders: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_influence_source_orders(),
            &mut inherited,
        )?,
        morph_defaults: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_morph_defaults(),
            &mut inherited,
        )?,
        morph_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_morph_names(),
            &mut inherited,
        )?,
        morph_values: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_morph_values(),
            &mut inherited,
        )?,
        morph_sparse_indices: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_morph_sparse_indices(),
            &mut inherited,
        )?,
        instance_morph_weights: inherit_optional_leaf(
            lane,
            pending.input,
            proof.deformation_instance_morph_weights(),
            &mut inherited,
        )?,
    };

    let animation = AnimationPayloadRefs {
        times: inherit_optional_leaf(lane, pending.input, proof.animation_times(), &mut inherited)?,
        values: inherit_optional_leaf(
            lane,
            pending.input,
            proof.animation_values(),
            &mut inherited,
        )?,
        in_tangents: inherit_optional_leaf(
            lane,
            pending.input,
            proof.animation_in_tangents(),
            &mut inherited,
        )?,
        out_tangents: inherit_optional_leaf(
            lane,
            pending.input,
            proof.animation_out_tangents(),
            &mut inherited,
        )?,
    };

    let metadata = inherited_leaf_plan.inherit(lane, pending.input, &mut inherited)?;
    if inherited != pending.counts.evidence_leaves.inherited.as_usize() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    Ok(InheritedNodeSceneLeafRefs {
        node_names,
        tangent_basis_schema_name,
        tangent_encoding_schema_name,
        appearance,
        geometry,
        deformation,
        animation,
        metadata,
    })
}

fn rewrite_spatial<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    inherited: InheritedNodeSceneLeafRefs,
) -> Result<RetainedSpatialRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = pending.input;
    let proof = input.proof;
    let cameras = rewrite_optional_record(
        lane,
        input,
        proof.cameras(),
        pending.counts.cameras.as_usize(),
        SpatialCameraTableSchema,
    )?;
    let lights = rewrite_optional_record(
        lane,
        input,
        proof.lights(),
        pending.counts.lights.as_usize(),
        SpatialLightTableSchema,
    )?;
    let directory = rewrite_required_record(
        lane,
        input,
        proof.spatial_directory(),
        1,
        SpatialDirectorySchema,
    )?;
    Ok(RetainedSpatialRefs {
        directory,
        cameras,
        lights,
        tangent_basis_schema_name: inherited.tangent_basis_schema_name,
        tangent_encoding_schema_name: inherited.tangent_encoding_schema_name,
    })
}

fn rewrite_appearance<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    payloads: AppearancePayloadRefs,
) -> Result<RetainedAppearanceRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = pending.input;
    let proof = input.proof;
    let images = rewrite_optional_record(
        lane,
        input,
        proof.appearance_images(),
        pending.counts.images.as_usize(),
        AppearanceImageTableSchema,
    )?;
    let samplers = rewrite_optional_record(
        lane,
        input,
        proof.appearance_samplers(),
        pending.counts.samplers.as_usize(),
        AppearanceSamplerTableSchema,
    )?;
    let textures = rewrite_optional_record(
        lane,
        input,
        proof.appearance_textures(),
        pending.counts.textures.as_usize(),
        AppearanceTextureTableSchema,
    )?;
    let bindings = rewrite_optional_record(
        lane,
        input,
        proof.appearance_bindings(),
        pending.counts.material_bindings.as_usize(),
        AppearanceBindingTableSchema,
    )?;
    let materials = rewrite_optional_record(
        lane,
        input,
        proof.materials(),
        pending.counts.materials.as_usize(),
        AppearanceMaterialTableSchema,
    )?;
    let directory = rewrite_required_record(
        lane,
        input,
        proof.appearance_directory(),
        1,
        AppearanceDirectorySchema,
    )?;
    Ok(RetainedAppearanceRefs {
        directory,
        tables: AppearanceTableRefs {
            images,
            samplers,
            textures,
            bindings,
            materials,
        },
        payloads,
    })
}

fn rewrite_geometry<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    payloads: GeometryPayloadRefs,
) -> Result<RetainedGeometryRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = pending.input;
    let proof = input.proof;
    let attributes = rewrite_optional_record(
        lane,
        input,
        proof.attributes(),
        pending.counts.attributes.as_usize(),
        GeometryAttributeTableSchema,
    )?;
    let primitives = rewrite_optional_record(
        lane,
        input,
        proof.primitives(),
        pending.counts.primitives.as_usize(),
        GeometryPrimitiveTableSchema,
    )?;
    let logical_meshes = rewrite_optional_record(
        lane,
        input,
        proof.logical_meshes(),
        pending.counts.logical_meshes.as_usize(),
        GeometryLogicalMeshTableSchema,
    )?;
    let mesh_instances = rewrite_optional_record(
        lane,
        input,
        proof.mesh_instances(),
        pending.counts.mesh_instances.as_usize(),
        GeometryMeshInstanceTableSchema,
    )?;
    let directory = rewrite_required_record(
        lane,
        input,
        proof.geometry_directory(),
        1,
        SceneGeometryDirectorySchema,
    )?;
    Ok(RetainedGeometryRefs {
        directory,
        tables: GeometryTableRefs {
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
        },
        payloads,
    })
}

fn rewrite_deformation<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    payloads: DeformationPayloadRefs,
) -> Result<RetainedDeformationRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = pending.input;
    let proof = input.proof;
    let skins = rewrite_optional_record(
        lane,
        input,
        proof.deformation_skins(),
        pending.counts.skins.as_usize(),
        DeformationSkinTableSchema,
    )?;
    let morph_definitions = rewrite_optional_record(
        lane,
        input,
        proof.deformation_morph_definitions(),
        pending.counts.morph_definitions.as_usize(),
        DeformationMorphDefinitionTableSchema,
    )?;
    let morph_deltas = rewrite_optional_record(
        lane,
        input,
        proof.deformation_morph_deltas(),
        pending.counts.morph_deltas.as_usize(),
        DeformationMorphDeltaTableSchema,
    )?;
    let morph_targets = rewrite_optional_record(
        lane,
        input,
        proof.deformation_morph_targets(),
        pending.counts.morph_targets.as_usize(),
        DeformationMorphTargetTableSchema,
    )?;
    let primitives = rewrite_optional_record(
        lane,
        input,
        proof.deformation_primitives(),
        pending.counts.primitive_deformations.as_usize(),
        DeformationPrimitiveTableSchema,
    )?;
    let logical_morphs = rewrite_optional_record(
        lane,
        input,
        proof.deformation_logical_morphs(),
        pending.counts.logical_morphs.as_usize(),
        DeformationLogicalMorphTableSchema,
    )?;
    let mesh_instances = rewrite_optional_record(
        lane,
        input,
        proof.deformation_mesh_instances(),
        pending.counts.mesh_instances.as_usize(),
        DeformationMeshInstanceTableSchema,
    )?;
    let directory = rewrite_required_record(
        lane,
        input,
        proof.deformation_directory(),
        1,
        DeformationDirectorySchema,
    )?;
    Ok(RetainedDeformationRefs {
        directory,
        tables: DeformationTableRefs {
            skins,
            logical_morphs,
            morph_definitions,
            primitives,
            morph_targets,
            morph_deltas,
            mesh_instances,
        },
        payloads,
    })
}

fn rewrite_animation<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
    payloads: AnimationPayloadRefs,
) -> Result<RetainedAnimationRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let input = pending.input;
    let proof = input.proof;
    let channels = rewrite_optional_record(
        lane,
        input,
        proof.animation_channels(),
        pending.counts.animation_channels.as_usize(),
        AnimationChannelTableSchema,
    )?;
    let clips = rewrite_optional_record(
        lane,
        input,
        proof.animation_clips(),
        pending.counts.animation_clips.as_usize(),
        AnimationClipTableSchema,
    )?;
    let directory = rewrite_required_record(
        lane,
        input,
        proof.animation_directory(),
        1,
        AnimationDirectorySchema,
    )?;
    Ok(RetainedAnimationRefs {
        directory,
        clips,
        channels,
        payloads,
    })
}

fn rewrite_optional_record<'lane, O, PP, U, Schema>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    input: &ValidatedNodeScene,
    source: Option<CanonicalBlockRef<Schema>>,
    expected_rows: usize,
    tag: Schema,
) -> Result<Option<CanonicalBlockRef<Schema>>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (source, expected_rows) {
        (None, 0) => Ok(None),
        (Some(source), rows) if rows != 0 => {
            rewrite_required_record(lane, input, source, rows, tag).map(Some)
        }
        _ => Err(StoreRuntimeError::InvalidSourceSlot),
    }
}

fn rewrite_required_record<'lane, O, PP, U, Schema>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    input: &ValidatedNodeScene,
    source: CanonicalBlockRef<Schema>,
    expected_rows: usize,
    tag: Schema,
) -> Result<CanonicalBlockRef<Schema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    let view = required_record_view(input, source)?;
    if view.rows() != expected_rows {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let plan = RecordPlan::new(view.rows(), view.fields()).map_err(StoreRuntimeError::Plan)?;
    lane.try_copy_mapped_records(&input.canonical, source.into_store(), plan, tag)
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
