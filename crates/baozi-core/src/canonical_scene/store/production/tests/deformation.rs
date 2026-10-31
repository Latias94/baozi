use std::cell::Cell;
use std::rc::Rc;

use crate::deform::{MorphDeltaSemantic, SkinMatrixCorrection, SkinMatrixSpace, SkinMatrixSpaces};
use crate::math::Mat4;

use super::super::plan::deformation::{
    DeformationAuthoringFailure, DeformationAuthoringLimit, DeformationAuthoringSemanticError,
    validate_deformation_batch_with_control,
};
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::{
    AuthoringRange, DeformationAuthoringBatch, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LogicalMorphAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, NodeLocalId, Presence,
    PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring, SkinAuthoring,
};
use super::{SceneValidationLimits, TestControl, TestControlError, TestReservation, trs, validate};

fn validate_single_skin<'input>(
    skins: &'input [SkinAuthoring<'input>],
    control: &mut TestControl,
) -> super::super::ValidatedDeformationBatch<'input, super::TestReservation> {
    validate_skin_batch(skins, SceneValidationLimits::default(), control)
        .expect("the single-skin deformation fixture must validate")
}

fn validate_skin_batch<'input>(
    skins: &'input [SkinAuthoring<'input>],
    limits: SceneValidationLimits,
    control: &mut TestControl,
) -> Result<
    super::super::ValidatedDeformationBatch<'input, TestReservation>,
    DeformationAuthoringFailure<TestControlError>,
> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup_control = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the deformation fixture node must validate");
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the deformation fixture");
    validate_deformation_batch_with_control(
        DeformationAuthoringBatch::new(skins, &[], &[], &[], &[]),
        &validated_nodes,
        &geometry,
        limits,
        control,
    )
}

#[test]
fn empty_deformation_charges_its_real_header_before_scratch_or_payload_work() {
    use crate::canonical_scene::store::typed_storage::deformation::DeformationStorage;
    let expected = std::mem::size_of::<DeformationStorage>() as u64;
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let mut control = TestControl::default();
    let batch = validate_skin_batch(&[], limits, &mut control).unwrap();
    assert_eq!(batch.canonical_bytes(), expected);
    assert_eq!(batch.semantic_cell_count(), 0);
    drop(batch);
    assert_eq!(control.live.get(), 0);

    let mut control = TestControl::default();
    assert!(matches!(
        validate_skin_batch(&[], limits.with_max_canonical_bytes(expected - 1), &mut control),
        Err(DeformationAuthoringFailure::Semantic(
            DeformationAuthoringSemanticError::LimitExceeded {
                limit: DeformationAuthoringLimit::CanonicalBytes, actual, maximum,
            }
        )) if actual == expected && maximum == expected - 1
    ));
    assert_eq!(control.reserves, 0);
    assert_eq!(control.work, 0);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn skin_footprint_counts_used_matrices_and_inline_correction_exactly_once() {
    use crate::canonical_scene::store::typed_storage::StoredNodeIndex;
    use crate::canonical_scene::store::typed_storage::deformation::{
        DeformationStorage, StoredSkin,
    };
    let joints = [NodeLocalId::new(0)];
    let mut matrix = Mat4::IDENTITY;
    matrix.cols[0][0] = -2.0;
    matrix.cols[1][0] = 0.25;
    matrix.cols[2][2] = 0.0;
    matrix.cols[1][3] = -0.0;
    let matrices = [matrix];
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        matrix,
    )
    .unwrap();
    let name = "骨架";
    let expected = (std::mem::size_of::<DeformationStorage>()
        + std::mem::size_of::<StoredSkin>()
        + std::mem::size_of::<StoredNodeIndex>()
        + std::mem::size_of::<Mat4>()
        + name.len()) as u64;
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let mut previous_work = None;
    for spaces in [
        SkinMatrixSpaces::canonical(),
        SkinMatrixSpaces::canonical().with_correction(correction),
    ] {
        let skins = [SkinAuthoring::new(
            Presence::Present(name),
            &joints,
            &matrices,
            Presence::Absent,
            spaces,
        )];
        let mut control = TestControl::default();
        let batch = validate_skin_batch(&skins, limits, &mut control).unwrap();
        assert_eq!(batch.canonical_bytes(), expected);
        assert_eq!(
            batch.f32_scalar_count(),
            if spaces.correction().is_some() {
                32
            } else {
                16
            }
        );
        assert_eq!(batch.u32_scalar_count(), 1);
        assert_eq!(
            batch.semantic_cell_count(),
            batch.f32_scalar_count() + 1 + name.len() as u64
        );
        assert_eq!(batch.validation_work(), control.work);
        if let Some(previous) = previous_work {
            assert_eq!(batch.validation_work(), previous + 16);
        }
        previous_work = Some(batch.validation_work());
        drop(batch);
        assert_eq!(control.live.get(), 0);
        assert!(matches!(
            validate_skin_batch(&skins, limits.with_max_canonical_bytes(expected - 1), &mut control),
            Err(DeformationAuthoringFailure::Semantic(
                DeformationAuthoringSemanticError::LimitExceeded {
                    limit: DeformationAuthoringLimit::CanonicalBytes, actual, maximum,
                }
            )) if actual == expected && maximum == expected - 1
        ));
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn top_level_scratch_is_admitted_without_traversing_unvalidated_skins() {
    let matrices = [Mat4 {
        cols: [[f32::NAN; 4]; 4],
    }];
    let joints = [NodeLocalId::new(0)];
    let skins = [SkinAuthoring::new(
        Presence::Absent,
        &joints,
        &matrices,
        Presence::Absent,
        SkinMatrixSpaces::canonical(),
    )];
    let mut control = TestControl::default();
    assert!(matches!(
        validate_skin_batch(
            &skins,
            SceneValidationLimits::default().with_max_scratch_bytes(0),
            &mut control
        ),
        Err(DeformationAuthoringFailure::Semantic(
            DeformationAuthoringSemanticError::LimitExceeded {
                limit: DeformationAuthoringLimit::ScratchBytes,
                ..
            }
        ))
    ));
    assert_eq!(control.work, 0);
    assert_eq!(control.reserves, 0);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn authoring_rejects_finite_projective_inverse_binds() {
    let joints = [NodeLocalId::new(0)];
    for (column, value) in [(0, 0.5), (3, 2.0)] {
        let mut matrix = Mat4::IDENTITY;
        matrix.cols[column][3] = value;
        let matrices = [matrix];
        let skins = [SkinAuthoring::new(
            Presence::Absent,
            &joints,
            &matrices,
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        )];
        let mut control = TestControl::default();
        assert!(matches!(
            validate_skin_batch(&skins, SceneValidationLimits::default(), &mut control),
            Err(DeformationAuthoringFailure::Semantic(
                DeformationAuthoringSemanticError::NonAffineInverseBind { skin: 0, joint: 0 }
            ))
        ));
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn nested_descriptors_and_uniqueness_growth_release_at_each_control_failure() {
    use crate::canonical_scene::store::typed_storage::deformation::{
        DeformationStorage, StoredLogicalMorph, StoredMorphDefinition, StoredMorphDelta,
        StoredMorphTarget, StoredPrimitiveDeformation,
    };
    let mut setup = TestControl::default();
    let nodes = validate(
        Presence::Present(&[]),
        &[],
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 0),
    )];
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &[], &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let definitions = [0, 1, 2].map(|identity| {
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(identity), Presence::Absent)
    });
    let logical = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, 3),
        &[0.0; 3],
    )];
    let deltas = [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ]
    .map(|semantic| {
        MorphDeltaAuthoring::new(semantic, MorphDeltaPayloadAuthoring::Dense { values: &[] })
    });
    let targets = [PrimitiveMorphTargetAuthoring::new(&deltas); 3];
    let deformation = [PrimitiveDeformationAuthoring::new(
        Presence::Absent,
        &targets,
    )];
    let input = DeformationAuthoringBatch::new(&[], &definitions, &logical, &deformation, &[]);
    let expected = (std::mem::size_of::<DeformationStorage>()
        + std::mem::size_of::<StoredLogicalMorph>()
        + 3 * std::mem::size_of::<StoredMorphDefinition>()
        + std::mem::size_of::<StoredPrimitiveDeformation>()
        + 3 * std::mem::size_of::<StoredMorphTarget>()
        + 9 * std::mem::size_of::<StoredMorphDelta>()
        + 3 * std::mem::size_of::<f32>()) as u64;
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let mut baseline = TestControl::default();
    let batch =
        validate_deformation_batch_with_control(input, &nodes, &geometry, limits, &mut baseline)
            .unwrap();
    assert_eq!(batch.canonical_bytes(), expected);
    assert_eq!(batch.maximum_uniqueness_capacity(), 3);
    assert_eq!(batch.morph_target_count(), 3);
    assert_eq!(batch.morph_delta_count(), 9);
    assert_eq!(batch.entity_count(), 15);
    assert_eq!(batch.semantic_cell_count(), 3);
    assert_eq!(batch.validation_work(), baseline.work);
    assert_eq!(batch.scratch_bytes(), baseline.current_bytes.get());
    assert!(
        baseline.resizes > 2,
        "nested buffers must grow after initial admission"
    );
    drop(batch);
    assert_eq!(baseline.current_bytes.get(), 0);

    for allowed_work in 0..baseline.work {
        let mut control = TestControl {
            reject_work_after: Some(allowed_work),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_deformation_batch_with_control(input, &nodes, &geometry, limits, &mut control),
            Err(DeformationAuthoringFailure::Control(
                TestControlError::WorkRejected
            ))
        ));
        assert_eq!(control.live.get(), 0, "work boundary {allowed_work}");
        assert_eq!(
            control.current_bytes.get(),
            0,
            "work boundary {allowed_work}"
        );
    }
    for resize in 1..=baseline.resizes {
        let mut control = TestControl {
            reject_resize_on: Some(resize),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_deformation_batch_with_control(input, &nodes, &geometry, limits, &mut control),
            Err(DeformationAuthoringFailure::Control(
                TestControlError::ResizeRejected
            ))
        ));
        assert_eq!(control.live.get(), 0, "resize boundary {resize}");
        assert_eq!(control.current_bytes.get(), 0, "resize boundary {resize}");
    }
    for poll in 1..=baseline.polls {
        let mut control = TestControl {
            cancel_on_poll: Some(poll),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_deformation_batch_with_control(input, &nodes, &geometry, limits, &mut control),
            Err(DeformationAuthoringFailure::Control(
                TestControlError::Cancelled
            ))
        ));
        assert_eq!(control.live.get(), 0, "poll boundary {poll}");
        assert_eq!(control.current_bytes.get(), 0, "poll boundary {poll}");
    }
    let mut control = TestControl::default();
    assert!(matches!(
        validate_deformation_batch_with_control(
            input, &nodes, &geometry, limits.with_max_canonical_bytes(expected - 1), &mut control,
        ),
        Err(DeformationAuthoringFailure::Semantic(
            DeformationAuthoringSemanticError::LimitExceeded {
                limit: DeformationAuthoringLimit::CanonicalBytes, actual, maximum,
            }
        )) if actual == expected && maximum == expected - 1
    ));
    assert_eq!(control.current_bytes.get(), 0);
}

#[test]
fn compiled_deformation_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let joints = vec![NodeLocalId::new(0)];
        let inverse_bind_matrices = vec![Mat4::IDENTITY];
        let skins = vec![SkinAuthoring::new(
            Presence::Absent,
            &joints,
            &inverse_bind_matrices,
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        )];

        validate_single_skin(&skins, &mut control).into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, [0; 6]);
    // Skin joint-count plans are transient, but the proof keeps its operation token alive.
    assert_eq!(control.current_bytes.get(), 0);
    assert_eq!(control.live.get(), 1);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn compiled_deformation_storage_drops_before_its_reservation() {
    let joints = [NodeLocalId::new(0)];
    let inverse_bind_matrices = [Mat4::IDENTITY];
    let skins = [SkinAuthoring::new(
        Presence::Absent,
        &joints,
        &inverse_bind_matrices,
        Presence::Absent,
        SkinMatrixSpaces::canonical(),
    )];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_single_skin(&skins, &mut control);

    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);

    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn borrowed_deformation_names_preserve_empty_unicode_and_bounded_errors() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 0),
    )];
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &[], &[]),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let logical = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, 1),
        &[0.0],
    )];
    let targets = [PrimitiveMorphTargetAuthoring::new(&[])];
    let deformation = [PrimitiveDeformationAuthoring::new(
        Presence::Absent,
        &targets,
    )];
    let long = "x".repeat(crate::entity::MAX_ENTITY_NAME_BYTES + 1);
    for name in ["", "骨架 α", "bad\nname", long.as_str()] {
        for skin_name in [true, false] {
            let skins = [SkinAuthoring::new(
                if skin_name {
                    Presence::Present(name)
                } else {
                    Presence::Absent
                },
                &[],
                &[],
                Presence::Absent,
                SkinMatrixSpaces::canonical(),
            )];
            let definitions = [MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(0),
                if skin_name {
                    Presence::Absent
                } else {
                    Presence::Present(name)
                },
            )];
            let mut control = TestControl::default();
            let result = validate_deformation_batch_with_control(
                DeformationAuthoringBatch::new(&skins, &definitions, &logical, &deformation, &[]),
                &validated_nodes,
                &geometry,
                SceneValidationLimits::default(),
                &mut control,
            );
            if let Err(expected) = crate::entity::validate_entity_name_str(name) {
                match result {
                    Err(DeformationAuthoringFailure::Semantic(
                        DeformationAuthoringSemanticError::InvalidSkinName { skin: 0, source },
                    )) if skin_name => assert_eq!(source, expected),
                    Err(DeformationAuthoringFailure::Semantic(
                        DeformationAuthoringSemanticError::InvalidMorphTargetName {
                            logical_mesh: 0,
                            target: 0,
                            source,
                        },
                    )) if !skin_name => assert_eq!(source, expected),
                    _ => panic!("borrowed name must retain its bounded domain error"),
                }
            } else {
                drop(result.expect("empty and Unicode names must remain accepted"));
            }
            assert_eq!(control.live.get(), 0);
        }
    }
}
