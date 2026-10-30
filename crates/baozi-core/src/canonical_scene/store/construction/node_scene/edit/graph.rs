use baozi_operation::__retained_store_integration::activation::CanonicalLane;
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, RecordPlan, StoreRuntimeError};

use super::super::NodeSceneMaterializedRefs;
use super::super::geometry::map_build;
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditProgram};
use super::profile::retain_edited_profile;
use super::required_record_view;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
};
use crate::canonical_scene::store::typed_storage::{SceneTables, build::NodeStoragePlan};

pub(super) struct RewrittenNodeSceneGraph {
    pub(super) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(super) refs: NodeSceneMaterializedRefs,
    pub(super) tables: SceneTables,
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
        NodeSceneEditProgram::ConvertSpace(plan) => {
            let conversion = plan.conversion();
            let nodes = plan.nodes();
            let geometry = plan.geometry();
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
    let spatial = if let NodeSceneEditProgram::ConvertSpace(plan) = &pending.program {
        let spatial = plan.spatial();
        lane.try_retain_storage(
            spatial.capacities(),
            spatial.work(),
            spatial.name_bytes() as u64,
            |control| spatial.materialize(control).map_err(map_build),
        )?
    } else {
        lane.try_share_storage(&pending.input.tables.spatial)?
    };
    let appearance_rows = if let NodeSceneEditProgram::ConvertSpace(plan) = &pending.program
        && plan.appearance().changes_storage()
    {
        let appearance = plan.appearance();
        lane.try_retain_storage(
            appearance.capacities().map_err(StoreRuntimeError::Plan)?,
            appearance.work(),
            appearance.name_bytes() as u64,
            |control| appearance.materialize(control).map_err(map_build),
        )?
    } else {
        lane.try_share_storage(&pending.input.tables.appearance.rows)?
    };
    let appearance = crate::canonical_scene::store::typed_storage::appearance::AppearanceStorage {
        rows: appearance_rows,
        payloads: lane.try_share_storage(&pending.input.tables.appearance.payloads)?,
    };
    let deformation = if let NodeSceneEditProgram::ConvertSpace(plan) = &pending.program
        && plan.deformation().changes_storage()
    {
        let deformation = plan.deformation();
        lane.try_retain_storage(
            deformation.capacities().map_err(StoreRuntimeError::Plan)?,
            deformation.work(),
            deformation.name_bytes() as u64,
            |control| deformation.materialize(control).map_err(map_build),
        )?
    } else {
        lane.try_share_storage(&pending.input.tables.deformation)?
    };
    let animation = if let NodeSceneEditProgram::ConvertSpace(plan) = &pending.program
        && plan.animation().changes_storage()
    {
        let animation = plan.animation();
        lane.try_retain_storage(
            animation.capacities().map_err(StoreRuntimeError::Plan)?,
            animation.work(),
            animation.name_bytes() as u64,
            |control| animation.materialize(control).map_err(map_build),
        )?
    } else {
        lane.try_share_storage(&pending.input.tables.animation)?
    };
    let directory = rewrite_directory_with_shape(lane, pending)?;

    Ok(RewrittenNodeSceneGraph {
        directory,
        tables: SceneTables {
            nodes,
            spatial,
            geometry,
            deformation,
            animation,
            appearance,
            profile,
        },
        refs: NodeSceneMaterializedRefs { directory },
    })
}

fn rewrite_directory_with_shape<'lane, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'_>,
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
                row.write_scalar(pending.counts.mesh_instances.as_u32())
            })
        },
    )
    .map(CanonicalBlockRef::from_store)
}
