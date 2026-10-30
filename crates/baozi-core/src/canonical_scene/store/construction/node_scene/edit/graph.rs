use baozi_operation::__retained_store_integration::activation::CanonicalLane;
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, ShareableStoreKind,
    StoreRuntimeError,
};

use super::super::animation::{AnimationPayloadRefs, RetainedAnimationRefs};
use super::super::appearance::{
    AppearancePayloadRefs, AppearanceTableRefs, RetainedAppearanceRefs,
};
use super::super::geometry::map_build;
use super::super::{NodeSceneMaterializedRefs, SceneStorage};
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditProgram};
use super::profile::retain_edited_profile;
use super::required_record_view;
use crate::canonical_scene::store::schema::animation::{
    AnimationChannelTableSchema, AnimationClipTableSchema, AnimationDirectorySchema,
};
use crate::canonical_scene::store::schema::appearance::{
    AppearanceBindingTableSchema, AppearanceDirectorySchema, AppearanceImageTableSchema,
    AppearanceMaterialTableSchema, AppearanceSamplerTableSchema, AppearanceTextureTableSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema,
};
use crate::canonical_scene::store::typed_storage::{SceneTables, build::NodeStoragePlan};

pub(super) struct RewrittenNodeSceneGraph {
    pub(super) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(super) refs: NodeSceneMaterializedRefs,
    pub(super) tables: SceneTables,
}

#[derive(Clone, Copy)]
struct InheritedNodeSceneLeafRefs {
    appearance: AppearancePayloadRefs,
    animation: AnimationPayloadRefs,
}

pub(super) fn try_rewrite_node_scene<'lane, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
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
        NodeSceneEditProgram::GenerateNormals { geometry: plan, .. } => {
            let nodes = lane.try_share_storage(&pending.input.tables.nodes)?;
            let geometry = lane.try_retain_storage(
                plan.capacities(),
                plan.work(),
                plan.name_bytes() as u64,
                |control| plan.materialize(control).map_err(map_build),
            )?;
            (nodes, geometry)
        }
        NodeSceneEditProgram::ConvertSpace {
            nodes,
            geometry,
            conversion,
        } => {
            let nodes = if conversion.changes_coordinates() {
                lane.try_retain_storage(
                    nodes.capacities(),
                    nodes.work(),
                    nodes.name_bytes() as u64,
                    |control| nodes.materialize(control).map_err(map_build),
                )?
            } else {
                lane.try_share_storage(&pending.input.tables.nodes)?
            };
            let geometry = if conversion.is_identity() {
                lane.try_share_storage(&pending.input.tables.geometry)?
            } else {
                lane.try_retain_storage(
                    geometry.capacities(),
                    geometry.work(),
                    geometry.name_bytes() as u64,
                    |control| geometry.materialize(control).map_err(map_build),
                )?
            };
            (nodes, geometry)
        }
    };
    let profile = retain_edited_profile(
        lane,
        pending.input,
        pending.counts.profile,
        &pending.operation,
    )?;
    let inherited = inherit_unchanged_evidence_leaves(lane, pending)?;
    let spatial = if let NodeSceneEditProgram::ConvertSpace { conversion, .. } = &pending.program {
        let source = pending.input.tables.spatial.as_ref();
        let plan = crate::canonical_scene::store::typed_storage::spatial::build::SpatialStoragePlan::for_storage(source)
            .map_err(StoreRuntimeError::Plan)?;
        lane.try_retain_storage(
            plan.capacities(),
            plan.work().map_err(StoreRuntimeError::Plan)?,
            plan.name_bytes() as u64,
            |control| {
                plan.rewrite_space(source, conversion.target(), control)
                    .map_err(map_build)
            },
        )?
    } else {
        lane.try_share_storage(&pending.input.tables.spatial)?
    };
    let appearance = rewrite_appearance(lane, pending, inherited.appearance)?;
    let deformation = lane.try_share_storage(&pending.input.tables.deformation)?;
    let animation = rewrite_animation(lane, pending, inherited.animation)?;
    let directory = rewrite_directory_with_shape(
        lane,
        pending,
        NodeSceneDirectoryRefs {
            appearance: appearance.directory,
            animation: animation.directory,
        },
    )?;

    Ok(RewrittenNodeSceneGraph {
        directory,
        tables: SceneTables {
            nodes,
            spatial,
            geometry,
            deformation,
            profile,
        },
        refs: NodeSceneMaterializedRefs {
            directory,
            appearance,
            animation,
        },
    })
}

#[derive(Clone, Copy)]
struct NodeSceneDirectoryRefs {
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
    animation: CanonicalBlockRef<AnimationDirectorySchema>,
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
                row.write_record_ref(refs.appearance.into_store())?;
                row.write_record_ref(refs.animation.into_store())
            })
        },
    )
    .map(CanonicalBlockRef::from_store)
}

fn inherit_unchanged_evidence_leaves<'lane, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
) -> Result<InheritedNodeSceneLeafRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let proof = pending.input.proof;
    let mut inherited = 0_usize;

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

    if inherited != pending.counts.inherited_leaf_count.as_usize() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    Ok(InheritedNodeSceneLeafRefs {
        appearance,
        animation,
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

fn inherit_optional_leaf<'lane, O, PP, U, Schema>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    input: &SceneStorage,
    source: Option<CanonicalBlockRef<Schema>>,
    inherited: &mut usize,
) -> Result<Option<CanonicalBlockRef<Schema>>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Schema: CanonicalSchema,
    Schema::StoreKind: ShareableStoreKind,
{
    let Some(source) = source else {
        return Ok(None);
    };
    let mapped = lane.try_share_canonical(&input.canonical, source.into_store())?;
    *inherited = inherited
        .checked_add(1)
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    Ok(Some(CanonicalBlockRef::from_store(mapped)))
}
