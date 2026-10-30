mod profile;

use baozi_operation::__retained_store_integration::activation::CanonicalLane;
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, StoreRuntimeError,
};

use super::super::animation::{AnimationPayloadRefs, RetainedAnimationRefs};
use super::super::appearance::{
    AppearancePayloadRefs, AppearanceTableRefs, RetainedAppearanceRefs,
};
use super::super::deformation::{
    DeformationPayloadRefs, DeformationTableRefs, RetainedDeformationRefs,
};
use super::super::geometry::map_build;
use super::super::materialization::RetainedSpatialRefs;
use super::super::profile::MetadataLeafRefs;
use super::super::{NodeSceneMaterializedRefs, SceneStorage};
use super::leaf_plan::{NodeSceneInheritedLeafPlan, inherit_optional_leaf};
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditProgram};
use super::required_record_view;
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
use crate::canonical_scene::store::schema::profile::ProfileDirectorySchema;
use crate::canonical_scene::store::schema::spatial::{
    SpatialCameraTableSchema, SpatialDirectorySchema, SpatialLightTableSchema,
    SpatialTangentBasisSchemaNameSchema, SpatialTangentEncodingSchemaNameSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema,
};
use crate::canonical_scene::store::typed_storage::{NodeGeometryStorage, build::NodeStoragePlan};

pub(super) struct RewrittenNodeSceneGraph {
    pub(super) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(super) refs: NodeSceneMaterializedRefs,
    pub(super) tables: NodeGeometryStorage,
}

#[derive(Clone, Copy)]
struct InheritedNodeSceneLeafRefs {
    tangent_basis_schema_name: Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>>,
    tangent_encoding_schema_name: Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>>,
    appearance: AppearancePayloadRefs,
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

    let (nodes, geometry) = match &pending.program {
        NodeSceneEditProgram::NodeTransform(rewrite) => {
            let source_nodes = pending.input.read_nodes();
            let plan =
                NodeStoragePlan::for_storage(source_nodes).map_err(StoreRuntimeError::Plan)?;
            let nodes = lane.try_retain_storage(
                plan.capacities(),
                plan.work().map_err(StoreRuntimeError::Plan)?,
                plan.name_bytes() as u64,
                |control| {
                    plan.rewrite(
                        source_nodes,
                        rewrite.target().canonical_row.as_usize(),
                        rewrite.replacement(),
                        control,
                    )
                    .map_err(map_build)
                },
            )?;
            (
                nodes,
                lane.try_share_storage(&pending.input.tables.geometry)?,
            )
        }
        NodeSceneEditProgram::TriangulatePrimitive(plan) => {
            let nodes = lane.try_share_storage(&pending.input.tables.nodes)?;
            let geometry = lane.try_retain_storage(
                plan.capacities(),
                plan.work(),
                plan.name_bytes() as u64,
                |mut control| plan.materialize(&mut control).map_err(map_build),
            )?;
            (nodes, geometry)
        }
    };
    let profile_leaves = profile::rewrite_lineage_leaves(lane, pending)?;
    let inherited = inherit_unchanged_evidence_leaves(lane, pending, inherited_leaf_plan)?;
    let spatial = rewrite_spatial(lane, pending, inherited)?;
    let appearance = rewrite_appearance(lane, pending, inherited.appearance)?;
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

    let directory = rewrite_directory_with_shape(
        lane,
        pending,
        NodeSceneDirectoryRefs {
            spatial: spatial.directory,
            appearance: appearance.directory,
            deformation: deformation.directory,
            animation: animation.directory,
            profile: profile.directory,
        },
    )?;

    Ok(RewrittenNodeSceneGraph {
        directory,
        tables: NodeGeometryStorage { nodes, geometry },
        refs: NodeSceneMaterializedRefs {
            directory,
            spatial,
            appearance,
            deformation,
            animation,
            profile,
        },
    })
}

#[derive(Clone, Copy)]
struct NodeSceneDirectoryRefs {
    spatial: CanonicalBlockRef<SpatialDirectorySchema>,
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
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
                row.write_scalar(pending.counts.canonical_node_rows.as_u32())?;
                row.write_scalar(pending.counts.source_root_rows.as_u32())?;
                row.write_scalar(pending.counts.logical_meshes.as_u32())?;
                row.write_scalar(pending.counts.primitives.as_u32())?;
                row.write_scalar(pending.counts.attributes.as_u32())?;
                row.write_scalar(pending.counts.mesh_instances.as_u32())?;
                row.write_record_ref(refs.spatial.into_store())?;
                row.write_record_ref(refs.appearance.into_store())?;
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
        clip_names: inherit_optional_leaf(
            lane,
            pending.input,
            proof.animation_clip_names(),
            &mut inherited,
        )?,
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
        tangent_basis_schema_name,
        tangent_encoding_schema_name,
        appearance,
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
    input: &SceneStorage,
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
    input: &SceneStorage,
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
