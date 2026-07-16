use super::*;
use crate::canonical_scene::builder::node_batch::CanonicalNodeDescriptor;

fn base_color_binding(
    texture: TextureId,
    provenance: Provenance,
) -> CanonicalTextureBindingDescriptor {
    CanonicalTextureBindingDescriptor::try_new(
        texture,
        TextureRole::BaseColor,
        TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
        ColorInterpretation::SrgbColor,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        TextureBlendOperation::Replace,
        None,
        provenance,
        [],
    )
    .expect("base-color binding fixture")
}

fn fully_populated_scene() -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("fully populated edit fixture builder");

    let camera = builder
        .try_push_camera(
            Camera::try_new(
                None,
                CameraProjection::perspective(1.0, None, 0.1, None).expect("camera projection"),
                provenance,
                [],
            )
            .expect("camera fixture"),
        )
        .expect("camera storage");
    let light = builder
        .try_push_light(
            Light::try_new(
                None,
                LightKind::Directional,
                Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                1.0,
                provenance,
                [],
            )
            .expect("light fixture"),
        )
        .expect("light storage");
    let node_plan = builder
        .node_batch_plan(1)
        .expect("fully populated root batch plan");
    let committed_nodes = builder
        .prepare_node_batch(node_plan, |batch| {
            let root = batch.slot(0)?;
            let children_plan = batch.node_children_plan(0)?;
            let children = batch.prepare_node_children(children_plan, [])?;
            let descriptor = CanonicalNodeDescriptor::new(
                None,
                AuthoredTransform::identity_trs(),
                children,
                Some(camera),
                Some(light),
                provenance.into(),
                Vec::new(),
            );
            batch.initialize_node(root, descriptor)?;
            batch.set_root(root)
        })
        .expect("fully populated root batch should prepare")
        .commit();
    let root = NodeIndex::from_handle(builder.generation(), committed_nodes.root())
        .expect("committed root should belong to the fixture generation");

    let image = builder
        .try_push_image(
            Image::try_encoded([1, 2, 3], None, None, None, None, provenance)
                .expect("image fixture"),
        )
        .expect("image storage");
    let sampler = builder
        .try_push_sampler(Sampler::canonical_default(provenance))
        .expect("sampler storage");
    let texture = builder
        .try_push_texture(CanonicalTextureDescriptor::new(image, sampler, provenance))
        .expect("texture storage");
    let material_id = builder
        .try_push_material(
            CanonicalMaterialDescriptor::try_new(
                None,
                ShadingModel::Unlit,
                AlphaMode::Opaque,
                false,
                MaterialFactors::empty(),
                [base_color_binding(texture, provenance)],
                provenance,
                [],
            )
            .expect("material fixture"),
        )
        .expect("material storage");
    let material = MaterialIndex::from_handle(builder.generation(), material_id)
        .expect("fixture material should belong to the builder generation");
    let primitive = MeshPrimitive::try_new(
        3,
        Topology::Points,
        IndexStream::non_indexed(),
        Some(material),
        vec![positions(provenance)],
        None,
        Vec::new(),
        1,
        PrimitiveBounds::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::DefaultWeights,
        ),
        provenance,
    )
    .expect("fully populated primitive");
    let logical_mesh = builder
        .try_push_logical_mesh(
            LogicalMesh::try_new(
                None,
                vec![primitive],
                vec![
                    MorphTargetDefinition::try_new(None, provenance)
                        .expect("morph target definition"),
                ],
                vec![0.0],
                LogicalMeshBounds::absent(
                    DerivedAbsenceReason::NotProvided,
                    MorphBoundsPolicy::DefaultWeights,
                ),
                provenance,
            )
            .expect("fully populated logical mesh"),
        )
        .expect("logical mesh storage");
    let skin = builder
        .try_push_skin(
            Skin::try_new(
                None,
                [root],
                [Mat4::IDENTITY],
                Some(root),
                SkinMatrixSpaces::canonical(),
                provenance,
            )
            .expect("skin fixture"),
        )
        .expect("skin storage");
    builder
        .try_push_mesh_instance(
            MeshInstance::try_new(
                root,
                logical_mesh,
                Some(skin),
                1,
                None,
                Vec::new(),
                provenance,
            )
            .expect("mesh instance fixture"),
        )
        .expect("mesh instance storage");
    builder
        .try_push_animation_clip(
            AnimationClip::try_new(
                0.0,
                AnimationTimeDomain::Seconds,
                [],
                provenance,
                &AnimationLimits::canonical(),
            )
            .expect("animation clip fixture"),
        )
        .expect("animation clip storage");

    validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("fully populated edit fixture should validate")
}

#[test]
fn changed_transform_publishes_fresh_generation_complete_remaps_and_invalidation() {
    let input = scene_with_valid_bounds();
    let input_generation = input.generation();
    let root = input.root().expect("fixture root");

    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform(root, translated(), SceneValidationLimits::default())
        .expect("changed transform should prepare an edit")
    else {
        panic!("a changed transform must not be classified as unchanged")
    };

    assert_eq!(
        input.node(root).unwrap().authored_transform(),
        AuthoredTransform::identity_trs(),
        "preparation must not mutate the input proof-bearing scene"
    );
    assert!(
        input.data().nodes[0].shares_payload_with(&edit.candidate.replacement),
        "a transform-only edit must share every non-transform node attachment"
    );

    let publication = edit
        .commit()
        .expect("validated edit should commit")
        .into_unreserved_publication();
    let output = publication.scene();
    let output_generation = output.generation();
    assert_ne!(output_generation, input_generation);
    assert_eq!(
        output
            .node(root.rebind(output_generation))
            .unwrap()
            .authored_transform(),
        translated()
    );
    assert_eq!(
        output.scene_bounds().derived().reason(),
        Some(DerivedAbsenceReason::Invalidated)
    );
    assert_eq!(
        publication.invalidations().scene_bounds(),
        Some(DerivedAbsenceReason::Invalidated)
    );

    let node_remap = publication.remaps().nodes();
    assert_eq!(node_remap.input_generation(), input_generation);
    assert_eq!(node_remap.output_generation(), output_generation);
    assert_eq!(node_remap.change(), RemapChange::ExplicitlyUnchanged);
    assert_eq!(
        node_remap.try_rebind_unchanged(root).unwrap(),
        Some(root.rebind(output_generation))
    );

    let empty_material_remap = publication.remaps().materials();
    assert_eq!(empty_material_remap.input_generation(), input_generation);
    assert_eq!(empty_material_remap.output_generation(), output_generation);
    assert_eq!(
        empty_material_remap.change(),
        RemapChange::ExplicitlyUnchanged
    );
}

#[test]
fn non_root_edit_changes_only_the_selected_node_and_shares_all_payloads() {
    let input = scene_with_three_nodes();
    let input_nodes = input.nodes().map(|(id, _)| id).collect::<Vec<_>>();
    let [root, child, sibling] = input_nodes.as_slice() else {
        panic!("multi-node fixture must contain exactly three nodes")
    };
    let before = [*root, *child, *sibling]
        .map(|id| input.node(id).expect("fixture node").authored_transform());

    let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
        &input,
        *child,
        translated(),
        SceneValidationLimits::default(),
    )
    .expect("non-root transform should prepare an edit") else {
        panic!("non-root transform must change")
    };

    let publication = edit
        .commit()
        .expect("non-root edit should commit")
        .into_unreserved_publication();
    let output = publication.scene();
    let generation = output.generation();
    assert!(
        output
            .node(root.rebind(generation))
            .expect("rebound root")
            .authored_transform()
            .has_same_authored_bits(before[0])
    );
    assert!(
        output
            .node(child.rebind(generation))
            .expect("rebound child")
            .authored_transform()
            .has_same_authored_bits(translated())
    );
    assert!(
        output
            .node(sibling.rebind(generation))
            .expect("rebound sibling")
            .authored_transform()
            .has_same_authored_bits(before[2])
    );
    for (id, expected) in [*root, *child, *sibling].into_iter().zip(before) {
        assert!(
            input
                .node(id)
                .expect("input node remains available")
                .authored_transform()
                .has_same_authored_bits(expected),
            "preparation and commit must leave the input generation unchanged"
        );
    }
    for index in 0..3 {
        assert!(
            input.data().nodes[index].shares_payload_with(&output.data().nodes[index]),
            "transform edits must share node payload {index}"
        );
    }
}

#[test]
fn matrix_and_trs_representation_changes_publish_the_requested_authored_form() {
    let identity_matrix =
        AuthoredTransform::try_matrix(Mat4::IDENTITY).expect("identity matrix transform");
    for (input_transform, requested, expected_kind) in [
        (
            AuthoredTransform::identity_trs(),
            identity_matrix,
            AuthoredTransformKind::Matrix,
        ),
        (
            identity_matrix,
            AuthoredTransform::identity_trs(),
            AuthoredTransformKind::Trs,
        ),
    ] {
        let input = scene_with_valid_bounds_and_transform(input_transform);
        let root = input.root().expect("fixture root");
        let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
            &input,
            root,
            requested,
            SceneValidationLimits::default(),
        )
        .expect("cross-representation edit should begin") else {
            panic!("equal evaluated matrices with different authored forms must change")
        };
        let publication = edit
            .commit()
            .expect("cross-representation edit should commit")
            .into_unreserved_publication();
        let output = publication.scene();
        let authored = output
            .node(root.rebind(output.generation()))
            .expect("rebound root")
            .authored_transform();

        assert_eq!(authored.kind(), expected_kind);
        assert!(authored.has_same_authored_bits(requested));
        assert!(
            input
                .node(root)
                .expect("input root remains available")
                .authored_transform()
                .has_same_authored_bits(input_transform)
        );
    }
}

#[test]
fn unchanged_transform_does_not_open_an_edit_or_mint_publication_state() {
    let input = scene_with_valid_bounds();
    let root = input.root().expect("fixture root");

    let result = CanonicalNodeTransformEdit::try_begin(
        &input,
        root,
        AuthoredTransform::identity_trs(),
        SceneValidationLimits::default(),
    )
    .expect("unchanged transform should classify without failure");

    assert!(matches!(result, CanonicalNodeTransformEditStart::Unchanged));
    assert_eq!(input.generation(), root.generation());
    assert!(input.scene_bounds().derived().is_valid());
}

#[test]
fn signed_zero_authored_bits_open_an_edit_and_survive_publication() {
    let input = scene_with_valid_bounds();
    let root = input.root().expect("fixture root");
    let transform = AuthoredTransform::try_trs(
        Vec3::new(-0.0, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("signed-zero transform");

    let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
        &input,
        root,
        transform,
        SceneValidationLimits::default(),
    )
    .expect("different authored bits should prepare an edit") else {
        panic!("signed-zero authored bits must not classify as unchanged")
    };
    let publication = edit
        .commit()
        .expect("signed-zero edit should validate")
        .into_unreserved_publication();
    let output = publication.scene();
    let translation = output
        .node(root.rebind(output.generation()))
        .expect("rebound root")
        .authored_transform()
        .authored_trs()
        .expect("TRS authored form")
        .translation();

    assert_eq!(translation.x.to_bits(), (-0.0_f32).to_bits());
}

#[test]
fn node_transform_rebinds_unaffected_local_bounds_without_copying_payloads() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().expect("fixture root");
    let (_, input_mesh) = input.logical_meshes().next().expect("fixture mesh");
    let (_, input_primitive) = input_mesh.primitives().next().expect("fixture primitive");
    let input_mesh_bounds = input_mesh
        .bounds()
        .derived()
        .value()
        .expect("valid input logical bounds");
    let input_primitive_bounds = input_primitive
        .bounds()
        .derived()
        .value()
        .expect("valid input primitive bounds");
    let input_footprint = input.validation_stats().footprint();

    let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
        &input,
        root,
        translated(),
        SceneValidationLimits::default(),
    )
    .expect("changed transform should prepare an edit") else {
        panic!("a changed transform must open an edit")
    };
    let publication = edit
        .commit()
        .expect("validated edit should commit")
        .into_unreserved_publication();
    let output = publication.scene();
    let output_generation = output.generation();
    let (_, output_mesh) = output.logical_meshes().next().expect("fixture mesh");
    let (_, output_primitive) = output_mesh.primitives().next().expect("fixture primitive");

    assert_eq!(
        output_mesh.bounds().derived().generation(),
        Some(output_generation)
    );
    assert_eq!(
        output_primitive.bounds().derived().generation(),
        Some(output_generation)
    );
    assert_eq!(
        output_mesh.bounds().derived().input_stamp(),
        input_mesh.bounds().derived().input_stamp()
    );
    assert_eq!(
        output_primitive.bounds().derived().input_stamp(),
        input_primitive.bounds().derived().input_stamp()
    );
    assert_eq!(
        output_mesh.bounds().derived().producer(),
        input_mesh.bounds().derived().producer()
    );
    assert_eq!(
        output_primitive.bounds().derived().producer(),
        input_primitive.bounds().derived().producer()
    );
    assert!(std::ptr::eq(
        input_mesh_bounds,
        output_mesh.bounds().derived().value().unwrap()
    ));
    assert!(std::ptr::eq(
        input_primitive_bounds,
        output_primitive.bounds().derived().value().unwrap()
    ));
    assert_eq!(
        output.scene_bounds().derived().reason(),
        Some(DerivedAbsenceReason::Invalidated)
    );
    assert!(
        input
            .data()
            .provenance_arena
            .ptr_eq(&output.data().provenance_arena)
    );
    assert!(input.data().provenance.ptr_eq(&output.data().provenance));
    assert!(
        input
            .data()
            .extension_schemas
            .ptr_eq(&output.data().extension_schemas)
    );
    assert!(input.data().metadata.ptr_eq(&output.data().metadata));
    assert_eq!(input.validation_stats().footprint(), input_footprint);
    assert_eq!(
        input_mesh.bounds().derived().generation(),
        Some(input.generation())
    );
    assert_eq!(
        input_primitive.bounds().derived().generation(),
        Some(input.generation())
    );
}

#[test]
fn validated_geometry_rebind_charges_wrappers_without_rehashing_payloads() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    assert_eq!(
        control.clone_work, 1,
        "begin may clone only the affected node"
    );

    let committed = edit.commit_with(&mut control).unwrap();
    assert_eq!(
        control.clone_work, 8,
        "commit planning and generation rebind must charge retained wrappers without rehashing payloads"
    );
    drop(committed);
}

#[test]
fn begin_cost_and_candidate_storage_scale_with_the_affected_delta_only() {
    let small_input = scene_with_valid_bounds();
    let small_root = small_input.root().expect("small fixture root");
    let mut small_control = TestControl::new();
    let small_edit = CanonicalNodeTransformEdit::try_begin_with(
        &small_input,
        small_root,
        translated(),
        SceneValidationLimits::default(),
        &mut small_control,
    )
    .expect("small transform edit should begin");
    assert!(matches!(
        small_edit,
        CanonicalNodeTransformEditStart::Changed(_)
    ));

    let large_input = scene_with_many_vertices(512);
    let large_root = large_input.root().expect("large fixture root");
    let mut large_control = TestControl::new();
    let large_edit = CanonicalNodeTransformEdit::try_begin_with(
        &large_input,
        large_root,
        translated(),
        SceneValidationLimits::default(),
        &mut large_control,
    )
    .expect("large transform edit should begin");
    assert!(matches!(
        large_edit,
        CanonicalNodeTransformEditStart::Changed(_)
    ));

    assert_eq!(small_control.total_work, large_control.total_work);
    assert_eq!(small_control.clone_work, large_control.clone_work);
    assert_eq!(small_control.candidate_reservation_bytes(), Some(0));
    assert_eq!(large_control.candidate_reservation_bytes(), Some(0));
    assert_eq!(small_control.candidate_resize_count(), 0);
    assert_eq!(large_control.candidate_resize_count(), 0);
}

#[test]
fn begin_enforces_the_affected_delta_work_budget() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().expect("fixture root");
    let mut baseline_control = TestControl::new();
    let baseline = CanonicalNodeTransformEdit::try_begin_with(
        &input,
        root,
        translated(),
        SceneValidationLimits::default(),
        &mut baseline_control,
    )
    .expect("baseline edit should begin");
    let exact_work = baseline_control.total_work;
    assert_eq!(exact_work, 1);
    assert_eq!(baseline_control.clone_work, exact_work);
    drop(baseline);

    let mut exact_control = TestControl::new();
    let exact = CanonicalNodeTransformEdit::try_begin_with(
        &input,
        root,
        translated(),
        SceneValidationLimits::default().with_max_work(exact_work),
        &mut exact_control,
    )
    .expect("the exact affected-work boundary must be inclusive");
    assert_eq!(exact_control.total_work, exact_work);
    drop(exact);

    let mut one_short_control = TestControl::new();
    let error = CanonicalNodeTransformEdit::try_begin_with(
        &input,
        root,
        translated(),
        SceneValidationLimits::default().with_max_work(exact_work - 1),
        &mut one_short_control,
    )
    .err()
    .expect("one unit below affected work must fail");
    let (actual, maximum) = match error {
        CanonicalEditFailure::Edit(CanonicalEditError::WorkLimitExceeded {
            actual,
            maximum,
            ..
        }) => (actual, maximum),
        other => panic!("unexpected affected-work failure: {other:?}"),
    };
    assert_eq!(actual, exact_work);
    assert_eq!(maximum, exact_work - 1);
    assert_eq!(one_short_control.total_work, 0);
    assert_eq!(one_short_control.reservation_drop_count(), 0);
}

#[test]
fn transaction_enforces_one_cumulative_work_budget_across_all_phases() {
    let input = scene_with_three_primitives_and_absent_scene_bounds();
    let root = input.root().expect("fixture root");

    let mut baseline_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut baseline_control,
        )
        .expect("baseline edit should begin")
    else {
        panic!("fixture transform must change")
    };
    let baseline = edit
        .commit_with(&mut baseline_control)
        .expect("baseline edit should commit");
    let total_work = baseline_control.total_work;
    assert!(total_work > 1);
    assert_eq!(
        baseline.publication.scene.validation_stats().work_units(),
        total_work,
        "validation stats must include begin, planning, materialization, and validation work"
    );
    drop(baseline);

    let exact_limits = SceneValidationLimits::default().with_max_work(total_work);
    let mut exact_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(root, translated(), exact_limits, &mut exact_control)
        .expect("the exact cumulative boundary must admit begin")
    else {
        panic!("fixture transform must change")
    };
    let exact = edit
        .commit_with(&mut exact_control)
        .expect("the exact cumulative boundary must be inclusive");
    assert_eq!(exact_control.total_work, total_work);
    assert_eq!(
        exact.publication.scene.validation_stats().work_units(),
        total_work
    );
    drop(exact);

    let below_limits = SceneValidationLimits::default().with_max_work(total_work - 1);
    let mut below_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) = input
        .try_edit_node_transform_with(root, translated(), below_limits, &mut below_control)
        .expect("one below the cumulative total must still admit the sparse begin phase")
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(below_control.shared_drop_state());
    let error = match edit.commit_with(&mut below_control) {
        Ok(_) => panic!("one below the cumulative total must fail transactionally"),
        Err(error) => error,
    };
    match error {
        CanonicalEditFailure::Edit(CanonicalEditError::WorkLimitExceeded { maximum, .. })
        | CanonicalEditFailure::Edit(CanonicalEditError::Validation(
            CanonicalValidationError::WorkLimitExceeded { maximum, .. },
        )) => assert_eq!(maximum, total_work - 1),
        other => panic!("unexpected cumulative work failure: {other}"),
    }
    assert!(below_control.total_work < total_work);
    assert_eq!(below_control.transition_count, 0);
    assert_eq!(below_control.live_scratch_reservations(), 0);
    assert!(below_control.reservation_drop_count() >= 1);
    assert_eq!(below_control.rollback_observed_state(), 0b111);
}

fn assert_unchanged_subject<S: RemapSpace>(
    remap: &CanonicalSubjectRemap<S>,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
) {
    assert_eq!(remap.input_generation(), input_generation);
    assert_eq!(remap.output_generation(), output_generation);
    assert_eq!(remap.change(), RemapChange::ExplicitlyUnchanged);
}

fn assert_all_subjects_unchanged(
    remaps: &CanonicalRemapSet,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
) {
    macro_rules! assert_unchanged {
        ($remap:expr) => {
            assert_unchanged_subject($remap, input_generation, output_generation)
        };
    }

    assert_unchanged!(remaps.nodes());
    assert_unchanged!(remaps.mesh_instances());
    assert_unchanged!(remaps.logical_meshes());
    assert_unchanged!(remaps.primitives());
    assert_unchanged!(remaps.morph_targets());
    assert_unchanged!(remaps.materials());
    assert_unchanged!(remaps.images());
    assert_unchanged!(remaps.samplers());
    assert_unchanged!(remaps.textures());
    assert_unchanged!(remaps.skins());
    assert_unchanged!(remaps.animation_clips());
    assert_unchanged!(remaps.cameras());
    assert_unchanged!(remaps.lights());
    assert_unchanged!(remaps.vertices());
    assert_unchanged!(remaps.corners());
    assert_unchanged!(remaps.faces());
    assert_unchanged!(remaps.instances());
    assert_eq!(remaps.retained_allocation_bytes(), 0);
}

#[test]
fn complete_remap_set_covers_every_entity_and_domain_even_when_empty() {
    let input = scene_with_valid_local_and_scene_bounds();
    let input_generation = input.generation();
    let root = input.root().unwrap();
    let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
        &input,
        root,
        translated(),
        SceneValidationLimits::default(),
    )
    .unwrap() else {
        panic!("fixture transform must change")
    };
    let publication = edit.commit().unwrap().into_unreserved_publication();
    let output_generation = publication.scene().generation();
    let remaps = publication.remaps();
    assert_all_subjects_unchanged(remaps, input_generation, output_generation);
}

#[test]
fn populated_scene_uses_compact_unchanged_markers_for_every_subject() {
    let input = fully_populated_scene();
    let input_generation = input.generation();
    let input_texture = input
        .materials()
        .next()
        .expect("fully populated fixture material")
        .1
        .texture_bindings()
        .next()
        .expect("fully populated fixture binding")
        .texture();
    assert_eq!(input_texture.generation(), input_generation);
    let root = input.root().expect("fully populated fixture root");
    let input_root = input.node(root).expect("fully populated input root view");
    let input_camera = input_root.camera().expect("fully populated input camera");
    let input_light = input_root.light().expect("fully populated input light");
    assert_eq!(input_camera.generation(), input_generation);
    assert_eq!(input_light.generation(), input_generation);
    let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
        &input,
        root,
        translated(),
        SceneValidationLimits::default(),
    )
    .expect("fully populated transform edit should begin") else {
        panic!("fixture transform must change")
    };
    let publication = edit
        .commit()
        .expect("fully populated transform edit should commit")
        .into_unreserved_publication();
    let output = publication.scene();
    let output_generation = output.generation();
    let output_texture = output
        .materials()
        .next()
        .expect("published fixture material")
        .1
        .texture_bindings()
        .next()
        .expect("published fixture binding")
        .texture();
    assert_eq!(output_texture.generation(), output_generation);
    assert_eq!(output_texture.index(), input_texture.index());
    let output_root = output
        .node(output.root().expect("published fully populated root"))
        .expect("published fully populated root view");
    let output_camera = output_root.camera().expect("published camera attachment");
    let output_light = output_root.light().expect("published light attachment");
    assert_eq!(output_camera.generation(), output_generation);
    assert_eq!(output_camera.index(), input_camera.index());
    assert_eq!(output_light.generation(), output_generation);
    assert_eq!(output_light.index(), input_light.index());
    let remaps = publication.remaps();
    assert_all_subjects_unchanged(remaps, input_generation, output_generation);
}
