use super::*;
use crate::canonical_scene::builder::assert_generation_mismatch_for_test;
use crate::canonical_validation::validate_canonical_scene;
use crate::entity::{Camera, CameraProjection, Light, LightEmission, LightKind};
use crate::identity::{CameraId, LightId, SceneEntityKind};
use crate::math::Color;
use crate::validation_control::SceneValidationLimits;

fn camera(provenance: Provenance) -> Camera {
    Camera::try_new(
        None,
        CameraProjection::perspective(1.0, None, 0.1, None)
            .expect("camera projection should be valid"),
        provenance,
        [],
    )
    .expect("camera fixture should be valid")
}

fn light(provenance: Provenance) -> Light {
    Light::try_new(
        None,
        LightKind::Directional,
        LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 1.0).expect("light emission"),
        provenance,
        [],
    )
    .expect("light fixture should be valid")
}

fn push_nonzero_attachments(
    builder: &mut CanonicalSceneBuilder,
    provenance: Provenance,
) -> (CameraId, LightId) {
    builder
        .try_push_camera(camera(provenance))
        .expect("dummy camera should prepare index zero");
    let camera = builder
        .try_push_camera(camera(provenance))
        .expect("selected camera should prepare index one");
    builder
        .try_push_light(light(provenance))
        .expect("dummy light should prepare index zero");
    let light = builder
        .try_push_light(light(provenance))
        .expect("selected light should prepare index one");
    (camera, light)
}

fn prepare_attached_root_with_plan<'owner>(
    builder: &'owner mut CanonicalSceneBuilder,
    plan: CanonicalNodeBatchPlan,
    provenance: Provenance,
    camera: Option<CameraId>,
    light: Option<LightId>,
) -> Result<PreparedCanonicalNodeBatch<'owner>, CanonicalSceneBuildError> {
    builder.prepare_node_batch(plan, |batch| {
        let root = batch.slot(0)?;
        let children_plan = batch.node_children_plan(0)?;
        let children = batch.prepare_node_children(children_plan, [])?;
        let descriptor = CanonicalNodeDescriptor::new(
            None,
            AuthoredTransform::identity_trs(),
            children,
            camera,
            light,
            provenance.into(),
            Vec::new(),
        );
        batch.initialize_node(root, descriptor)?;
        batch.set_root(root)
    })
}

fn prepare_attached_root<'owner>(
    builder: &'owner mut CanonicalSceneBuilder,
    provenance: Provenance,
    camera: Option<CameraId>,
    light: Option<LightId>,
) -> Result<PreparedCanonicalNodeBatch<'owner>, CanonicalSceneBuildError> {
    let plan = builder.node_batch_plan(1)?;
    prepare_attached_root_with_plan(builder, plan, provenance, camera, light)
}

fn assert_pristine_node_target(builder: &CanonicalSceneBuilder) {
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);
    assert!(builder.data.root.is_none());
}

#[test]
fn node_attachments_lower_nonzero_qualified_ids_and_roundtrip_views() {
    let (mut builder, provenance) = scene_builder();
    let generation = builder.generation();
    let plan = builder
        .node_batch_plan(1)
        .expect("node batch plan should precede attachment insertion");
    let (camera, light) = push_nonzero_attachments(&mut builder, provenance);

    assert_eq!(camera.generation(), generation);
    assert_eq!(camera.index(), 1);
    assert_eq!(light.generation(), generation);
    assert_eq!(light.index(), 1);

    let committed =
        prepare_attached_root_with_plan(&mut builder, plan, provenance, Some(camera), Some(light))
            .expect("live prepare-time attachment counts should admit the node")
            .commit();
    let root = committed.root();

    assert_eq!(
        builder.data.nodes[0].camera_index(),
        Some(CameraIndex::from_u32(1))
    );
    assert_eq!(
        builder.data.nodes[0].light_index(),
        Some(LightIndex::from_u32(1))
    );

    let scene = validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("qualified attachments should survive central validation");
    let root = scene
        .node(root)
        .expect("committed root should remain addressable");
    assert_eq!(root.camera(), Some(camera));
    assert_eq!(root.light(), Some(light));
}

#[test]
fn node_attachments_reject_foreign_generations_with_colliding_indices_without_publication() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let (foreign_camera, foreign_light) =
        push_nonzero_attachments(&mut foreign, foreign_provenance);

    let (mut builder, provenance) = scene_builder();
    let (local_camera, local_light) = push_nonzero_attachments(&mut builder, provenance);
    assert_eq!(foreign_camera.index(), local_camera.index());
    assert_eq!(foreign_light.index(), local_light.index());

    let error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(foreign_camera),
        Some(local_light),
    )
    .expect_err("foreign camera generation must be rejected");
    assert_generation_mismatch_for_test(error, SceneEntityKind::Camera);
    assert_pristine_node_target(&builder);

    let error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(local_camera),
        Some(foreign_light),
    )
    .expect_err("foreign light generation must be rejected");
    assert_generation_mismatch_for_test(error, SceneEntityKind::Light);
    assert_pristine_node_target(&builder);

    let error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(foreign_camera),
        Some(foreign_light),
    )
    .expect_err("camera generation must precede light generation");
    assert_generation_mismatch_for_test(error, SceneEntityKind::Camera);
    assert_pristine_node_target(&builder);
}

#[test]
fn node_attachment_ranges_are_exact_and_all_generations_precede_ranges() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_light = foreign
        .try_push_light(light(foreign_provenance))
        .expect("foreign light fixture should be valid");

    let (mut builder, provenance) = scene_builder();
    let local_camera = builder
        .try_push_camera(camera(provenance))
        .expect("local camera fixture should be valid");
    let local_light = builder
        .try_push_light(light(provenance))
        .expect("local light fixture should be valid");
    let generation = builder.generation();
    let camera_count = builder.data.cameras.len();
    let light_count = builder.data.lights.len();
    let first_missing_camera = CameraId::new(
        generation,
        u32::try_from(camera_count).expect("camera count should fit canonical identity"),
    );
    let first_missing_light = LightId::new(
        generation,
        u32::try_from(light_count).expect("light count should fit canonical identity"),
    );

    let camera_error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(first_missing_camera),
        Some(local_light),
    )
    .expect_err("the first missing camera must fail before publication");
    assert_eq!(
        camera_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Camera,
            length: camera_count,
        })
    );
    assert_pristine_node_target(&builder);

    let light_error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(local_camera),
        Some(first_missing_light),
    )
    .expect_err("the first missing light must fail before publication");
    assert_eq!(
        light_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Light,
            length: light_count,
        })
    );
    assert_pristine_node_target(&builder);

    let mixed_error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(first_missing_camera),
        Some(foreign_light),
    )
    .expect_err("every generation must be checked before any range");
    assert_generation_mismatch_for_test(mixed_error, SceneEntityKind::Light);
    assert_pristine_node_target(&builder);

    let range_order_error = prepare_attached_root(
        &mut builder,
        provenance,
        Some(first_missing_camera),
        Some(first_missing_light),
    )
    .expect_err("camera range should precede light range");
    assert_eq!(
        range_order_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Camera,
            length: camera_count,
        })
    );
    assert_pristine_node_target(&builder);
}

#[test]
fn dropping_prepared_attached_node_batch_leaves_builder_retryable() {
    let (mut builder, provenance) = scene_builder();
    let (camera, light) = push_nonzero_attachments(&mut builder, provenance);

    let prepared = prepare_attached_root(&mut builder, provenance, Some(camera), Some(light))
        .expect("valid attached node should prepare");
    drop(prepared);
    assert_pristine_node_target(&builder);

    let committed = prepare_attached_root(&mut builder, provenance, Some(camera), Some(light))
        .expect("dropping preparation should leave the builder retryable")
        .commit();
    assert_eq!(committed.root().index(), 0);
    assert_eq!(
        builder.data.nodes[0].camera_index(),
        Some(CameraIndex::from_u32(1))
    );
    assert_eq!(
        builder.data.nodes[0].light_index(),
        Some(LightIndex::from_u32(1))
    );
}

#[test]
fn partial_attached_batch_failure_drops_staging_and_leaves_builder_retryable() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_light = foreign
        .try_push_light(light(foreign_provenance))
        .expect("foreign light fixture should be valid");

    let (mut builder, provenance) = scene_builder();
    let (camera, light) = push_nonzero_attachments(&mut builder, provenance);
    let plan = builder
        .node_batch_plan(2)
        .expect("two-node batch should have a capacity plan");
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            let child = batch.slot(1)?;

            let root_children_plan = batch.node_children_plan(1)?;
            let root_children = batch.prepare_node_children(root_children_plan, [child])?;
            let root_descriptor = CanonicalNodeDescriptor::new(
                None,
                AuthoredTransform::identity_trs(),
                root_children,
                None,
                None,
                provenance.into(),
                Vec::new(),
            );
            batch.initialize_node(root, root_descriptor)?;

            let child_children_plan = batch.node_children_plan(0)?;
            let child_children = batch.prepare_node_children(child_children_plan, [])?;
            let child_descriptor = CanonicalNodeDescriptor::new(
                None,
                AuthoredTransform::identity_trs(),
                child_children,
                Some(camera),
                Some(foreign_light),
                provenance.into(),
                Vec::new(),
            );
            batch.initialize_node(child, child_descriptor)?;
            batch.set_root(root)
        })
        .expect_err("the second node's foreign light must reject the whole batch");
    assert_generation_mismatch_for_test(error, SceneEntityKind::Light);
    assert_pristine_node_target(&builder);

    let committed = prepare_attached_root(&mut builder, provenance, Some(camera), Some(light))
        .expect("a failed partial batch must leave the builder retryable")
        .commit();
    assert_eq!(committed.root().index(), 0);
}
