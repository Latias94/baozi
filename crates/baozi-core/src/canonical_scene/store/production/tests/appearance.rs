//! Source authoring boundaries for typed appearance construction.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, MaterialFactorGroup, MaterialFactors,
    MaterialSemantics, ShadingModel, TextureBindingSemantics, TextureBlendOperation,
    TextureChannel, TextureChannelSwizzle, TextureRole, TextureSourceKey, TextureStrength,
    TextureTransform,
};
use crate::canonical_scene::store::typed_storage::appearance::build::AppearanceStoragePlan;
use crate::canonical_scene::store::typed_storage::appearance::{
    AppearancePayloads, AppearanceRows, AppearanceStorage, StoredBinding, StoredImage,
    StoredMaterial, StoredTexture,
};
use crate::extension::ExtensionMediaType;
use crate::image::{MAX_ENCODED_IMAGE_BYTES, SamplerState};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationWork};

use super::super::plan::appearance::{
    AppearanceAuthoringFailure, AppearanceAuthoringLimit, AppearanceAuthoringSemanticError,
    appearance_shape_scratch_envelope, validate_appearance_batch_with_control,
    validate_encoded_payload_limit,
};
use super::super::{
    AppearanceAuthoringBatch, AuthoringRange, ImageAuthoring, ImageLocalId, MaterialAuthoring,
    MaterialBindingAuthoring, Presence, ResourceIdentityLocalId, ResourceSnapshotAuthoring,
    ResourceSnapshotAuthoringTable, SamplerAuthoring, SamplerLocalId, TextureAuthoring,
    TextureLocalId,
};
use super::{SceneValidationLimits, TestControl, TestControlError};

fn with_input<T>(
    action: impl FnOnce(AppearanceAuthoringBatch<'_>, ResourceSnapshotAuthoringTable<'_>) -> T,
) -> T {
    // Equal bytes are two source identities, resolved from different authorities.
    let encoded = vec![0_u8, 255, 2];
    let resource = vec![0_u8, 255, 2];
    let media = ExtensionMediaType::new("image/png").unwrap();
    let images = [
        ImageAuthoring::encoded(&encoded, Presence::Present(&media)),
        ImageAuthoring::session_resource(ResourceIdentityLocalId::new(0), Presence::Absent),
    ];
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(&resource)];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [
        TextureAuthoring::new(ImageLocalId::new(1), SamplerLocalId::new(0)),
        TextureAuthoring::new(ImageLocalId::new(0), SamplerLocalId::new(0)),
    ];
    let roles = [
        TextureRole::BaseColor,
        TextureRole::Normal,
        TextureRole::BaseColor,
    ];
    let uv_sets = [0, 1, u16::MAX];
    let channels = [
        TextureChannelSwizzle::RGBA,
        TextureChannelSwizzle::RGB,
        TextureChannelSwizzle::RGBA,
    ];
    let colors = [
        ColorInterpretation::SrgbColor,
        ColorInterpretation::Data,
        ColorInterpretation::SrgbColor,
    ];
    let key = TextureSourceKey::try_new("src:α").unwrap();
    let bindings: [_; 3] = std::array::from_fn(|index| {
        MaterialBindingAuthoring::new(
            TextureLocalId::new([0, 1, 0][index]),
            TextureBindingSemantics::new(
                roles[index],
                uv_sets[index],
                channels[index],
                colors[index],
                TextureTransform::IDENTITY,
                TextureStrength::try_new(if index == 1 { -0.0 } else { 1.0 }).unwrap(),
                TextureBlendOperation::Replace,
                if index == 1 { None } else { Some(&key) },
            ),
        )
    });
    let alphas = [
        AlphaMode::Opaque,
        AlphaMode::Blend,
        AlphaMode::mask(-0.0).unwrap(),
    ];
    let name = String::from("材");
    let materials = [
        MaterialAuthoring::new(
            AuthoringRange::new(0, 2),
            MaterialSemantics::new(
                ShadingModel::Unlit,
                alphas[0],
                false,
                MaterialFactors::empty(),
            ),
        )
        .with_name(&name),
        MaterialAuthoring::new(
            AuthoringRange::new(2, 0),
            MaterialSemantics::new(
                ShadingModel::Unlit,
                alphas[1],
                true,
                MaterialFactors::empty(),
            ),
        )
        .with_name(""),
        MaterialAuthoring::new(
            AuthoringRange::new(2, 1),
            MaterialSemantics::new(
                ShadingModel::Unlit,
                alphas[2],
                false,
                MaterialFactors::empty(),
            ),
        ),
    ];
    action(
        AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials),
        ResourceSnapshotAuthoringTable::new(1, &snapshots),
    )
}

#[test]
fn authoring_seals_actual_typed_footprint_and_borrows_original_semantics() {
    with_input(|input, snapshots| {
        let mut control = TestControl::default();
        let mut result = None;
        let allocation = allocation_counter::measure(|| {
            result = Some(
                validate_appearance_batch_with_control(
                    input,
                    snapshots,
                    SceneValidationLimits::default(),
                    &mut control,
                )
                .unwrap(),
            );
        });
        let batch = result.unwrap();
        assert_eq!(
            allocation.count_total, 1,
            "only resolved image slices allocate scratch"
        );
        assert_eq!(batch.row_counts(), [2, 1, 2, 3, 3]);
        assert_eq!(batch.payload_counts(), [6, 24]);
        assert_eq!(batch.image_payload_byte_count(), 6);
        assert_eq!(batch.text_byte_count(), 24);
        assert_eq!(batch.entity_count(), 11);
        assert_eq!(batch.edge_count(), 10);
        assert_eq!(batch.reference_count(), 10);
        // 6 image bytes + 24 UTF-8 bytes + 3*6 binding floats + 3*11 present
        // material factor floats + one authored Mask cutoff. No absent-group padding.
        assert_eq!(batch.semantic_cell_count(), 82);
        // Rows + text bytes + one owner check per binding + edge/reference
        // actions + final ownership closure. Opaque image contents require no scan.
        assert_eq!(batch.validation_work(), 11 + 24 + 3 + 20 + 1);
        assert_eq!(control.work, batch.validation_work());
        assert_eq!(control.traversal_work(), 4);

        let expected = (std::mem::size_of::<AppearanceStorage>()
            + std::mem::size_of::<AppearanceRows>()
            + std::mem::size_of::<AppearancePayloads>()
            + 2 * std::mem::size_of::<StoredImage>()
            + std::mem::size_of::<SamplerState>()
            + 2 * std::mem::size_of::<StoredTexture>()
            + 3 * std::mem::size_of::<StoredBinding>()
            + 3 * std::mem::size_of::<StoredMaterial>()
            + 6
            + 24) as u64;
        let plan = AppearanceStoragePlan::new(&batch).unwrap();
        assert_eq!(batch.canonical_bytes(), expected);
        assert_eq!(plan.canonical_bytes().unwrap(), expected);
        assert_eq!(plan.row_counts(), batch.row_counts());
        assert_eq!(plan.payload_counts(), batch.payload_counts());
        assert_eq!(plan.name_bytes(), 24);
        assert_eq!(plan.work().unwrap(), 49);
        assert_eq!(plan.rows_capacities().unwrap().len(), 5);
        assert_eq!(plan.payload_capacities().unwrap().len(), 2);
        assert_eq!(batch.scratch_bytes(), control.current_bytes.get());
        assert!(batch.scratch_bytes() <= appearance_shape_scratch_envelope(input).unwrap());

        assert_eq!(batch.input().images().as_ptr(), input.images().as_ptr());
        assert_eq!(batch.samplers().as_ptr(), input.samplers().as_ptr());
        assert_eq!(batch.textures().as_ptr(), input.textures().as_ptr());
        assert_eq!(batch.bindings().as_ptr(), input.bindings().as_ptr());
        assert!(
            batch
                .bindings()
                .iter()
                .map(|binding| binding.semantics().uv_set())
                .eq([0, 1, u16::MAX])
        );
        assert_eq!(batch.materials().as_ptr(), input.materials().as_ptr());
        assert_eq!(
            batch.bindings()[1].semantics().strength().get().to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(
            batch.materials()[2]
                .semantics()
                .alpha_mode()
                .mask_cutoff()
                .unwrap()
                .to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(batch.materials()[0].name(), Presence::Present("材"));
        assert_eq!(batch.materials()[1].name(), Presence::Present(""));
        assert_eq!(batch.materials()[2].name(), Presence::Absent);

        let first = batch.image(0).unwrap();
        let second = batch.image(1).unwrap();
        assert_eq!(batch.images().count(), 2);
        assert!(batch.image(2).is_none());
        assert_eq!(first.payload(), second.payload());
        assert_ne!(first.payload().as_ptr(), second.payload().as_ptr());
        let Presence::Present(resource_bytes) = snapshots.snapshots()[0].bytes() else {
            panic!("fixture resource must be snapshotted");
        };
        assert_eq!(second.payload().as_ptr(), resource_bytes.as_ptr());
        let Presence::Present(media) = first.media_type() else {
            panic!("fixture encoded image must have a media type");
        };
        assert_eq!(media.as_str(), "image/png");
        assert_eq!(second.media_type(), Presence::Absent);
        assert_eq!(
            batch.bindings()[0]
                .semantics()
                .source_key()
                .unwrap()
                .as_str(),
            "src:α"
        );
        assert_eq!(
            batch.bindings()[2]
                .semantics()
                .source_key()
                .unwrap()
                .as_str(),
            "src:α"
        );
        drop(batch);
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    });
}

#[test]
fn exact_typed_limits_accept_and_each_one_less_boundary_releases_scratch() {
    with_input(|input, snapshots| {
        let bytes = {
            let mut control = TestControl::default();
            validate_appearance_batch_with_control(
                input,
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            )
            .unwrap()
            .canonical_bytes()
        };
        let scratch = appearance_shape_scratch_envelope(input).unwrap();
        let exact = SceneValidationLimits::default()
            .with_max_entities(11)
            .with_max_edges(10)
            .with_max_references(10)
            .with_max_payload_cells(82)
            .with_max_canonical_bytes(bytes)
            .with_max_work(59)
            .with_max_scratch_bytes(scratch);
        let mut control = TestControl::default();
        let batch =
            validate_appearance_batch_with_control(input, snapshots, exact, &mut control).unwrap();
        assert_eq!(batch.validation_work(), 59);
        drop(batch);

        for (limit, limits) in [
            (
                AppearanceAuthoringLimit::Entities,
                exact.with_max_entities(10),
            ),
            (AppearanceAuthoringLimit::Edges, exact.with_max_edges(9)),
            (
                AppearanceAuthoringLimit::References,
                exact.with_max_references(9),
            ),
            (
                AppearanceAuthoringLimit::SemanticCells,
                exact.with_max_payload_cells(81),
            ),
            (
                AppearanceAuthoringLimit::CanonicalBytes,
                exact.with_max_canonical_bytes(bytes - 1),
            ),
            (AppearanceAuthoringLimit::Work, exact.with_max_work(58)),
            (
                AppearanceAuthoringLimit::ScratchBytes,
                exact.with_max_scratch_bytes(scratch - 1),
            ),
        ] {
            let mut control = TestControl::default();
            let result =
                validate_appearance_batch_with_control(input, snapshots, limits, &mut control);
            assert!(
                matches!(result,
                Err(AppearanceAuthoringFailure::Semantic(AppearanceAuthoringSemanticError::LimitExceeded { limit: actual, .. }))
                if actual == limit),
                "wrong boundary failure for {limit:?}"
            );
            assert_eq!(control.live.get(), 0);
            assert_eq!(control.current_bytes.get(), 0);
        }
    });
}

#[test]
fn empty_appearance_counts_the_typed_header_without_allocating_projection_scratch() {
    let input = AppearanceAuthoringBatch::empty();
    let bytes = (std::mem::size_of::<AppearanceStorage>()
        + std::mem::size_of::<AppearanceRows>()
        + std::mem::size_of::<AppearancePayloads>()) as u64;
    assert_eq!(appearance_shape_scratch_envelope(input).unwrap(), 0);
    let limits = SceneValidationLimits::default()
        .with_max_canonical_bytes(bytes)
        .with_max_scratch_bytes(0)
        .with_max_entities(0)
        .with_max_payload_cells(0)
        .with_max_edges(0)
        .with_max_references(0)
        .with_max_work(1);
    let mut control = TestControl::default();
    let allocation = allocation_counter::measure(|| {
        let batch = validate_appearance_batch_with_control(
            input,
            ResourceSnapshotAuthoringTable::empty(),
            limits,
            &mut control,
        )
        .unwrap();
        assert_eq!(batch.row_counts(), [0; 5]);
        assert_eq!(batch.payload_counts(), [0; 2]);
        assert_eq!(batch.canonical_bytes(), bytes);
        assert_eq!(batch.semantic_cell_count(), 0);
        assert_eq!(batch.validation_work(), 1);
        assert_eq!(batch.scratch_bytes(), 0);
        assert!(batch.image(0).is_none());
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(control.live.get(), 0);
    let result = validate_appearance_batch_with_control(
        input,
        ResourceSnapshotAuthoringTable::empty(),
        limits.with_max_canonical_bytes(bytes - 1),
        &mut control,
    );
    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::LimitExceeded {
                limit: AppearanceAuthoringLimit::CanonicalBytes,
                ..
            }
        ))
    ));
}

#[test]
fn shape_and_initial_admission_do_not_scan_or_resolve_images() {
    let images = [ImageAuthoring::session_resource(
        ResourceIdentityLocalId::new(u32::MAX),
        Presence::Absent,
    )];
    let input = AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]);
    assert!(appearance_shape_scratch_envelope(input).is_ok());
    let mut cancelled = TestControl {
        cancel_on_poll: Some(1),
        ..TestControl::default()
    };
    let result = validate_appearance_batch_with_control(
        input,
        ResourceSnapshotAuthoringTable::empty(),
        SceneValidationLimits::default(),
        &mut cancelled,
    );
    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(cancelled.work, 0);
    assert_eq!(cancelled.reserves, 0);

    let mut control = TestControl::default();
    let result = validate_appearance_batch_with_control(
        input,
        ResourceSnapshotAuthoringTable::empty(),
        SceneValidationLimits::default().with_max_entities(0),
        &mut control,
    );
    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::LimitExceeded {
                limit: AppearanceAuthoringLimit::Entities,
                ..
            }
        ))
    ));
    assert_eq!(control.work, 0);
    assert_eq!(control.reserves, 0);
}

#[test]
fn session_resource_table_and_image_authority_fail_at_the_source_boundary() {
    let bytes = [1_u8];
    let absent = [ResourceSnapshotAuthoring::absent()];
    let present = [ResourceSnapshotAuthoring::snapshotted(&bytes)];
    let extra = [
        ResourceSnapshotAuthoring::snapshotted(&bytes),
        ResourceSnapshotAuthoring::absent(),
    ];
    for (resource, table, expected_work) in [
        (0, ResourceSnapshotAuthoringTable::new(1, &absent), 1),
        (0, ResourceSnapshotAuthoringTable::new(1, &[]), 1),
        (1, ResourceSnapshotAuthoringTable::new(1, &present), 1),
        (0, ResourceSnapshotAuthoringTable::new(1, &extra), 0),
    ] {
        let images = [ImageAuthoring::session_resource(
            ResourceIdentityLocalId::new(resource),
            Presence::Absent,
        )];
        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
            table,
            SceneValidationLimits::default(),
            &mut control,
        );
        match result {
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                    image: 0,
                    resource: 0,
                },
            )) => assert_eq!(resource, 0),
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::ImageResourceOutOfRange {
                    image: 0,
                    resource: 1,
                    resource_count: 1,
                },
            )) => assert_eq!(resource, 1),
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::ResourceSnapshotCardinalityMismatch {
                    snapshot_count: 2,
                    resource_count: 1,
                },
            )) => assert_eq!(expected_work, 0),
            other => panic!("wrong resource authority outcome: {other:?}"),
        }
        assert_eq!(control.work, expected_work);
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
    // No image uses this profile resource, so a missing snapshot table is legal.
    let images = [ImageAuthoring::encoded(&bytes, Presence::Absent)];
    let mut control = TestControl::default();
    let result = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &[]),
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(result.is_ok());
    drop(result);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn encoded_and_resolved_empty_images_remain_invalid_and_size_maximum_is_inclusive() {
    let empty = [];
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(&empty)];
    for image in [
        ImageAuthoring::encoded(&empty, Presence::Absent),
        ImageAuthoring::session_resource(ResourceIdentityLocalId::new(0), Presence::Absent),
    ] {
        let images = [image];
        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
            ResourceSnapshotAuthoringTable::new(1, &snapshots),
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::EmptyEncodedPayload { image: 0 }
            ))
        ));
        assert_eq!(control.work, 1);
        assert_eq!(control.live.get(), 0);
    }
    assert!(validate_encoded_payload_limit(7, MAX_ENCODED_IMAGE_BYTES).is_ok());
    assert!(
        matches!(validate_encoded_payload_limit(7, MAX_ENCODED_IMAGE_BYTES + 1),
        Err(AppearanceAuthoringSemanticError::EncodedPayloadTooLarge { image: 7, actual, maximum })
        if actual == MAX_ENCODED_IMAGE_BYTES + 1 && maximum == MAX_ENCODED_IMAGE_BYTES)
    );
}

#[test]
fn every_completed_action_and_each_poll_cancellation_releases_resolved_image_scratch() {
    with_input(|input, snapshots| {
        let mut complete = TestControl::default();
        let batch = validate_appearance_batch_with_control(
            input,
            snapshots,
            SceneValidationLimits::default(),
            &mut complete,
        )
        .unwrap();
        let work = batch.validation_work();
        drop(batch);
        for prefix in 0..work {
            let mut control = TestControl {
                reject_work_after: Some(prefix),
                ..TestControl::default()
            };
            let allocation = allocation_counter::measure(|| {
                assert!(matches!(
                    validate_appearance_batch_with_control(
                        input,
                        snapshots,
                        SceneValidationLimits::default(),
                        &mut control
                    ),
                    Err(AppearanceAuthoringFailure::Control(
                        TestControlError::WorkRejected
                    ))
                ));
            });
            assert_eq!(control.work, prefix);
            assert_eq!(control.live.get(), 0);
            assert_eq!(control.current_bytes.get(), 0);
            assert_eq!(allocation.count_current, 0);
            assert_eq!(allocation.bytes_current, 0);
        }
        for poll in 1..=complete.polls {
            let mut control = TestControl {
                cancel_on_poll: Some(poll),
                ..TestControl::default()
            };
            let result = validate_appearance_batch_with_control(
                input,
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            );
            assert!(matches!(
                result,
                Err(AppearanceAuthoringFailure::Control(
                    TestControlError::Cancelled
                ))
            ));
            assert_eq!(control.polls, poll);
            assert_eq!(control.live.get(), 0);
            assert_eq!(control.current_bytes.get(), 0);
        }
    });
}

#[test]
fn encoded_image_admission_counts_bytes_without_scheduling_content_validation() {
    for size in [1, 1024 * 1024] {
        let payload = vec![0xff; size];
        let images = [ImageAuthoring::encoded(&payload, Presence::Absent)];
        let mut control = TestControl::default();
        let batch = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
            ResourceSnapshotAuthoringTable::empty(),
            SceneValidationLimits::default().with_max_work(2),
            &mut control,
        )
        .expect("opaque image bytes need only the image-row and completion actions");
        assert_eq!(batch.validation_work(), 2);
        assert_eq!(control.work, 2);
        assert_eq!(batch.image_payload_byte_count(), size as u64);
        assert_eq!(batch.semantic_cell_count(), size as u64);
        assert_eq!(batch.image(0).unwrap().payload().as_ptr(), payload.as_ptr());
        drop(batch);
        assert_eq!(control.current_bytes.get(), 0);
    }
}

#[test]
fn periodic_image_row_polls_stop_at_the_exact_prefix() {
    let images = vec![
        ImageAuthoring::encoded(&[9], Presence::Absent);
        (2 * SCENE_VALIDATION_POLL_INTERVAL + 3) as usize
    ];
    for poll in [2, 3] {
        let mut control = TestControl {
            cancel_on_poll: Some(poll),
            ..TestControl::default()
        };
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
            ResourceSnapshotAuthoringTable::empty(),
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Control(
                TestControlError::Cancelled
            ))
        ));
        assert_eq!(control.work, (poll - 1) * SCENE_VALIDATION_POLL_INTERVAL);
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
    }
}

#[test]
fn resize_failure_releases_the_only_scratch_allocation_and_reservation() {
    with_input(|input, snapshots| {
        let mut control = TestControl {
            reject_resize_on: Some(1),
            ..TestControl::default()
        };
        let allocation = allocation_counter::measure(|| {
            let result = validate_appearance_batch_with_control(
                input,
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            );
            assert!(matches!(
                result,
                Err(AppearanceAuthoringFailure::Control(
                    TestControlError::ResizeRejected
                ))
            ));
        });
        assert_eq!(control.work, 0);
        assert_eq!(control.live.get(), 0);
        assert_eq!(control.current_bytes.get(), 0);
        assert_eq!(allocation.count_current, 0);
        assert_eq!(allocation.bytes_current, 0);
    });
}

#[test]
fn resolved_image_storage_drops_before_reservation_on_normal_drop_and_unwind() {
    with_input(|input, snapshots| {
        for unwind in [false, true] {
            let stage = Rc::new(Cell::new(0));
            let mut control = TestControl {
                reservation_drop_stage: Some(Rc::clone(&stage)),
                ..TestControl::default()
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                let mut batch = validate_appearance_batch_with_control(
                    input,
                    snapshots,
                    SceneValidationLimits::default(),
                    &mut control,
                )
                .unwrap();
                batch.install_storage_drop_probe(Rc::clone(&stage));
                if unwind {
                    std::panic::resume_unwind(Box::new(()));
                }
                drop(batch);
            }));
            assert_eq!(result.is_err(), unwind);
            assert_eq!(stage.get(), 2);
            assert_eq!(control.live.get(), 0);
            assert_eq!(control.current_bytes.get(), 0);
        }
    });
}

#[test]
fn material_binding_ranges_reject_out_of_bounds_gaps_overlap_and_unowned_rows() {
    with_input(|input, snapshots| {
        for range in [AuthoringRange::new(1, 0), AuthoringRange::new(0, u32::MAX)] {
            let materials = [MaterialAuthoring::new(
                range,
                input.materials()[0].semantics(),
            )];
            let mut control = TestControl::default();
            let result = validate_appearance_batch_with_control(
                AppearanceAuthoringBatch::new(
                    input.images(),
                    input.samplers(),
                    input.textures(),
                    input.bindings(),
                    &materials,
                ),
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            );
            assert!(matches!(
                result,
                Err(AppearanceAuthoringFailure::Semantic(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOrder { material: 0, .. }
                        | AppearanceAuthoringSemanticError::MaterialBindingRangeOutOfBounds {
                            material: 0,
                            ..
                        }
                ))
            ));
            assert_eq!(control.live.get(), 0);
        }
        let materials = [
            input.materials()[0],
            MaterialAuthoring::new(AuthoringRange::new(1, 1), input.materials()[1].semantics()),
        ];
        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(
                input.images(),
                input.samplers(),
                input.textures(),
                input.bindings(),
                &materials,
            ),
            snapshots,
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::MaterialBindingRangeOrder {
                    material: 1,
                    expected: 2,
                    actual: 1
                }
            ))
        ));
        assert_eq!(control.live.get(), 0);

        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(
                input.images(),
                input.samplers(),
                input.textures(),
                input.bindings(),
                &input.materials()[..2],
            ),
            snapshots,
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::UnownedBindings { unowned: 1 }
            ))
        ));
        assert_eq!(control.last_work(), Some(SceneValidationWork::Traversal));
        assert_eq!(control.live.get(), 0);
    });
}

#[test]
fn duplicate_binding_roles_stop_at_second_owner_check_with_no_structural_prepass() {
    with_input(|input, snapshots| {
        let bindings = [input.bindings()[0]; 3];
        let materials = [MaterialAuthoring::new(
            AuthoringRange::new(0, 3),
            input.materials()[0].semantics(),
        )];
        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(
                input.images(),
                input.samplers(),
                input.textures(),
                &bindings,
                &materials,
            ),
            snapshots,
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::InvalidMaterialBindingSemantics {
                    material: 0,
                    source: AppearanceBuildError::DuplicateBindingAuthority {
                        first_index: 0,
                        duplicate_index: 1
                    }
                }
            ))
        ));
        assert_eq!(control.traversal_work(), 2);
        assert_eq!(control.live.get(), 0);
    });
}

#[test]
fn closed_roles_preserve_metallic_authority_conflicts() {
    with_input(|input, snapshots| {
        let roles = [TextureRole::MetallicRoughness, TextureRole::Metallic];
        let channels = [
            TextureChannelSwizzle::try_new(&[TextureChannel::Green, TextureChannel::Blue]).unwrap(),
            TextureChannelSwizzle::RED,
        ];
        let bindings: [_; 2] = std::array::from_fn(|index| {
            MaterialBindingAuthoring::new(
                TextureLocalId::new(0),
                TextureBindingSemantics::new(
                    roles[index],
                    0,
                    channels[index],
                    ColorInterpretation::Data,
                    TextureTransform::IDENTITY,
                    TextureStrength::ONE,
                    TextureBlendOperation::Replace,
                    None,
                ),
            )
        });
        let materials = [MaterialAuthoring::new(
            AuthoringRange::new(0, 2),
            input.materials()[0].semantics(),
        )];
        let mut control = TestControl::default();
        let result = validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(
                input.images(),
                input.samplers(),
                input.textures(),
                &bindings,
                &materials,
            ),
            snapshots,
            SceneValidationLimits::default(),
            &mut control,
        );
        assert!(matches!(
            result,
            Err(AppearanceAuthoringFailure::Semantic(
                AppearanceAuthoringSemanticError::InvalidMaterialBindingSemantics {
                    material: 0,
                    source: AppearanceBuildError::ConflictingBindingRoles {
                        first_index: 0,
                        first_role: TextureRole::MetallicRoughness,
                        duplicate_index: 1,
                        duplicate_role: TextureRole::Metallic,
                    }
                }
            ))
        ));
        assert_eq!(control.traversal_work(), 2);
        assert_eq!(control.live.get(), 0);
    });
}

#[test]
fn material_factor_requirements_and_name_validation_remain_authoring_errors() {
    with_input(|input, snapshots| {
        for name_failure in [false, true] {
            let material = if name_failure {
                input.materials()[0].with_name("invalid\nname")
            } else {
                MaterialAuthoring::new(
                    AuthoringRange::new(0, 2),
                    MaterialSemantics::new(
                        ShadingModel::PbrMetallicRoughness,
                        AlphaMode::Opaque,
                        false,
                        MaterialFactors::empty(),
                    ),
                )
            };
            let materials = [material, input.materials()[1], input.materials()[2]];
            let mut control = TestControl::default();
            let result = validate_appearance_batch_with_control(
                AppearanceAuthoringBatch::new(
                    input.images(),
                    input.samplers(),
                    input.textures(),
                    input.bindings(),
                    &materials,
                ),
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            );
            if name_failure {
                assert!(matches!(
                    result,
                    Err(AppearanceAuthoringFailure::Semantic(
                        AppearanceAuthoringSemanticError::InvalidMaterialName { material: 0, .. }
                    ))
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(AppearanceAuthoringFailure::Semantic(
                        AppearanceAuthoringSemanticError::InvalidMaterialSemantics {
                            material: 0,
                            source: AppearanceBuildError::MissingFactorGroup {
                                shading: ShadingModel::PbrMetallicRoughness,
                                required: MaterialFactorGroup::PbrMetallicRoughness,
                            }
                        }
                    ))
                ));
            }
            assert_eq!(control.live.get(), 0);
        }
    });
}

#[test]
fn texture_image_sampler_and_binding_texture_indices_keep_typed_source_locations() {
    with_input(|input, snapshots| {
        for invalid in 0..3 {
            let textures = [TextureAuthoring::new(
                ImageLocalId::new(if invalid == 0 { 2 } else { 0 }),
                SamplerLocalId::new(if invalid == 1 { 1 } else { 0 }),
            )];
            let bindings = [MaterialBindingAuthoring::new(
                TextureLocalId::new(if invalid == 2 { 1 } else { 0 }),
                input.bindings()[0].semantics(),
            )];
            let materials = [MaterialAuthoring::new(
                AuthoringRange::new(0, 1),
                input.materials()[0].semantics(),
            )];
            let mut control = TestControl::default();
            let result = validate_appearance_batch_with_control(
                AppearanceAuthoringBatch::new(
                    input.images(),
                    input.samplers(),
                    &textures,
                    &bindings,
                    &materials,
                ),
                snapshots,
                SceneValidationLimits::default(),
                &mut control,
            );
            match (invalid, result) {
                (
                    0,
                    Err(AppearanceAuthoringFailure::Semantic(
                        AppearanceAuthoringSemanticError::TextureImageOutOfRange {
                            texture: 0,
                            image: 2,
                            image_count: 2,
                        },
                    )),
                )
                | (
                    1,
                    Err(AppearanceAuthoringFailure::Semantic(
                        AppearanceAuthoringSemanticError::TextureSamplerOutOfRange {
                            texture: 0,
                            sampler: 1,
                            sampler_count: 1,
                        },
                    )),
                )
                | (
                    2,
                    Err(AppearanceAuthoringFailure::Semantic(
                        AppearanceAuthoringSemanticError::BindingTextureOutOfRange {
                            binding: 0,
                            texture: 1,
                            texture_count: 1,
                        },
                    )),
                ) => {}
                other => panic!("wrong source index outcome: {other:?}"),
            }
            assert_eq!(control.live.get(), 0);
        }
    });
}
