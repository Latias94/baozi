use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::canonical_scene::store::production::input::deformation::MorphTargetIdentity;
use crate::canonical_scene::store::typed_storage::deformation::{
    StoredInfluenceRows, StoredLogicalMorph, StoredMeshInstanceDeformation, StoredMorphDefinition,
    StoredMorphDelta, StoredMorphTarget, StoredMorphTargetIndex, StoredPrimitiveDeformation,
    StoredSkinIndex,
};
use crate::canonical_scene::store::typed_storage::{
    StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
};
use crate::deform::SkinMatrixSpace;
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpace};
use crate::space_conversion::TargetSpace;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(u64);

#[derive(Default)]
struct Control {
    completed: u64,
    strings: u64,
    stop: Option<u64>,
    omit: Option<u64>,
    panic: Option<u64>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, Interrupted> {
        if self.stop == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.panic == Some(self.completed) {
            // Avoid global panic-hook/output-capture allocations in the release probe.
            std::panic::resume_unwind(Box::new(Interrupted(self.completed)));
        }
        let result = action();
        self.completed += 1;
        self.strings += u64::from(string);
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(false, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(true, action)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.omit == Some(self.completed) {
            return Ok(None);
        }
        self.execute(false, action).map(Some)
    }
}

fn reflected() -> SpaceConversion {
    // Q(x,y,z) = (-x,-z,y), det(Q) = -1, s = 2.
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveZ,
            Axis::PositiveY,
            0.5,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn units(target_units: f64) -> SpaceConversion {
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            target_units,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn correction(matrix: Mat4) -> SkinMatrixSpaces {
    SkinMatrixSpaces::canonical().with_correction(
        SkinMatrixCorrection::new(
            SkinMatrixSpace::LogicalMeshLocalBind,
            SkinMatrixSpace::MeshInstanceLocal,
            matrix,
        )
        .unwrap(),
    )
}

fn singular() -> Mat4 {
    Mat4 {
        cols: [
            [0.0, -0.0, 0.0, -0.0],
            [2.0, 3.0, 4.0, 0.0],
            [-5.0, 6.0, 7.0, -0.0],
            [8.0, -9.0, 10.0, 1.0],
        ],
    }
}

fn singular_expected() -> Mat4 {
    Mat4 {
        cols: [
            [0.0, 0.0, 0.0, 0.0],
            [-5.0, 7.0, -6.0, 0.0],
            [-2.0, -4.0, 3.0, 0.0],
            [-16.0, -20.0, -18.0, 1.0],
        ],
    }
}

fn mirrored() -> Mat4 {
    Mat4 {
        cols: [
            [-2.0, -0.0, 0.0, -0.0],
            [0.5, 3.0, 0.0, 0.0],
            [0.0, 0.0, 4.0, -0.0],
            [1.0, 2.0, 3.0, 1.0],
        ],
    }
}

fn fixture() -> DeformationStorage {
    let mut correction_matrix = mirrored();
    correction_matrix.cols[3] = [4.0, -0.0, -2.0, 1.0];
    DeformationStorage {
        skins: vec![
            StoredSkin {
                name: Some(TextRange(TableRange::new(0, 4))),
                joints: TableRange::new(0, 1),
                inverse_bind_matrices: TableRange::new(0, 1),
                skeleton_root: Some(StoredNodeIndex(2)),
                matrix_spaces: correction(correction_matrix),
            },
            StoredSkin {
                name: Some(TextRange(TableRange::new(4, 0))),
                joints: TableRange::new(1, 1),
                inverse_bind_matrices: TableRange::new(1, 1),
                skeleton_root: None,
                matrix_spaces: SkinMatrixSpaces::canonical(),
            },
        ],
        logical_morphs: vec![
            StoredLogicalMorph {
                definitions: TableRange::new(0, 2),
                default_weights: TableRange::new(0, 2),
            },
            StoredLogicalMorph {
                definitions: TableRange::new(2, 0),
                default_weights: TableRange::new(2, 0),
            },
        ],
        morph_definitions: vec![
            StoredMorphDefinition {
                logical_mesh: StoredLogicalMeshIndex(0),
                target_ordinal: 0,
                stable_identity: MorphTargetIdentity::new(1),
                name: Some(TextRange(TableRange::new(4, 0))),
            },
            StoredMorphDefinition {
                logical_mesh: StoredLogicalMeshIndex(0),
                target_ordinal: 1,
                stable_identity: MorphTargetIdentity::new(0),
                name: Some(TextRange(TableRange::new(4, 2))),
            },
        ],
        primitives: vec![
            StoredPrimitiveDeformation {
                influences: Some(StoredInfluenceRows {
                    vertex_offsets: TableRange::new(0, 4),
                    influences: TableRange::new(0, 3),
                }),
                morph_targets: TableRange::new(0, 2),
            },
            StoredPrimitiveDeformation {
                influences: Some(StoredInfluenceRows {
                    vertex_offsets: TableRange::new(4, 1),
                    influences: TableRange::new(3, 0),
                }),
                morph_targets: TableRange::new(2, 2),
            },
            StoredPrimitiveDeformation {
                influences: None,
                morph_targets: TableRange::new(4, 0),
            },
        ],
        morph_targets: vec![
            StoredMorphTarget {
                primitive: 0,
                target_ordinal: 0,
                deltas: TableRange::new(0, 3),
            },
            StoredMorphTarget {
                primitive: 0,
                target_ordinal: 1,
                deltas: TableRange::new(3, 1),
            },
            StoredMorphTarget {
                primitive: 1,
                target_ordinal: 0,
                deltas: TableRange::new(4, 1),
            },
            StoredMorphTarget {
                primitive: 1,
                target_ordinal: 1,
                deltas: TableRange::new(5, 0),
            },
        ],
        morph_deltas: vec![
            StoredMorphDelta {
                target: StoredMorphTargetIndex(0),
                semantic: MorphDeltaSemantic::Position,
                payload: StoredMorphDeltaPayload::Dense(TableRange::new(0, 9)),
            },
            StoredMorphDelta {
                target: StoredMorphTargetIndex(0),
                semantic: MorphDeltaSemantic::Normal,
                payload: StoredMorphDeltaPayload::Sparse {
                    logical_count: 3,
                    indices: TableRange::new(0, 1),
                    values: TableRange::new(9, 3),
                },
            },
            StoredMorphDelta {
                target: StoredMorphTargetIndex(0),
                semantic: MorphDeltaSemantic::Tangent,
                payload: StoredMorphDeltaPayload::Dense(TableRange::new(12, 9)),
            },
            StoredMorphDelta {
                target: StoredMorphTargetIndex(1),
                semantic: MorphDeltaSemantic::Position,
                payload: StoredMorphDeltaPayload::Sparse {
                    logical_count: 3,
                    indices: TableRange::new(1, 0),
                    values: TableRange::new(21, 0),
                },
            },
            StoredMorphDelta {
                target: StoredMorphTargetIndex(2),
                semantic: MorphDeltaSemantic::Position,
                payload: StoredMorphDeltaPayload::Dense(TableRange::new(21, 0)),
            },
        ],
        mesh_instances: vec![
            StoredMeshInstanceDeformation {
                skin: Some(StoredSkinIndex(0)),
                morph_weights: Some(TableRange::new(2, 2)),
            },
            StoredMeshInstanceDeformation {
                skin: Some(StoredSkinIndex(1)),
                morph_weights: None,
            },
            StoredMeshInstanceDeformation {
                skin: None,
                morph_weights: Some(TableRange::new(4, 0)),
            },
            StoredMeshInstanceDeformation {
                skin: None,
                morph_weights: None,
            },
        ],
        joints: vec![StoredNodeIndex(3), StoredNodeIndex(1)],
        inverse_bind_matrices: vec![singular(), mirrored()],
        names: b"skin\xCE\xBC".to_vec(),
        influence_vertex_offsets: vec![0, 2, 2, 3, 0],
        influence_joints: vec![0, 0, 0],
        influence_weights: vec![0.25, -0.0, 2.0],
        influence_source_orders: vec![1, 0, 0],
        morph_weights: vec![-0.0, -2.0, 1.25, -0.0],
        morph_values: vec![
            1.0,
            2.0,
            3.0,
            -0.0,
            0.0,
            -2.0,
            f32::from_bits(1),
            -0.0,
            0.0, // Position
            -0.0,
            0.0,
            0.0, // sparse explicit zero Normal
            4.0,
            5.0,
            6.0,
            2.0,
            -3.0,
            0.0,
            -0.0,
            -0.0,
            0.0, // xyz Tangent
        ],
        morph_sparse_indices: vec![1],
    }
}

fn assert_float_bits(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

fn assert_matrix_bits(actual: Mat4, expected: Mat4) {
    assert_eq!(
        actual.cols.map(|column| column.map(f32::to_bits)),
        expected.cols.map(|column| column.map(f32::to_bits)),
    );
}

#[test]
fn reflection_units_and_axis_exchange_preserve_all_seventeen_buffers_and_exact_work() {
    let source = fixture();
    let mut preflight = Control::default();
    let plan = DeformationSpacePlan::try_new(&source, reflected(), &mut preflight).unwrap();
    assert_eq!(preflight.completed, 16); // 2 skins + 2 binds + 5 delta rows + 7 xyz tuples.
    assert_eq!(preflight.strings, 0);
    assert!(plan.changes_storage());
    assert_eq!(plan.work(), 72);
    assert_eq!(plan.correspondence_work(), 72);
    assert_eq!(plan.name_bytes(), 6);
    assert_eq!(
        plan.capacities().unwrap().map(|p| p.desired_capacity()),
        [2, 2, 2, 3, 4, 5, 4, 2, 2, 6, 5, 3, 3, 3, 4, 21, 1],
    );
    let mut build = Control::default();
    let output = plan.materialize(&mut build).unwrap();
    assert_eq!(build.completed, 72);
    assert_eq!(build.strings, 6);
    assert_matrix_bits(output.inverse_bind_matrices[0], singular_expected());
    assert_matrix_bits(
        output.inverse_bind_matrices[1],
        Mat4 {
            cols: [
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, 4.0, -0.0, 0.0],
                [-0.5, -0.0, 3.0, 0.0],
                [-2.0, -6.0, 4.0, 1.0],
            ],
        },
    );
    let correction = output.skins[0].matrix_spaces.correction().unwrap();
    assert_eq!(correction.from(), SkinMatrixSpace::LogicalMeshLocalBind);
    assert_eq!(correction.to(), SkinMatrixSpace::MeshInstanceLocal);
    assert_matrix_bits(
        correction.matrix(),
        Mat4 {
            cols: [
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, 4.0, -0.0, 0.0],
                [-0.5, -0.0, 3.0, 0.0],
                [-8.0, 4.0, -0.0, 1.0],
            ],
        },
    );
    assert!(output.skins[1].matrix_spaces.correction().is_none());
    assert_float_bits(
        &output.morph_values,
        &[
            -2.0,
            -6.0,
            4.0,
            0.0,
            4.0,
            0.0,
            -f32::from_bits(2),
            -0.0,
            -0.0,
            0.0,
            -0.0,
            0.0,
            -4.0,
            -6.0,
            5.0,
            -2.0,
            -0.0,
            -3.0,
            0.0,
            -0.0,
            -0.0,
        ],
    );
    assert_eq!(output.joints, source.joints);
    assert_eq!(output.names, source.names);
    assert_eq!(
        output.influence_vertex_offsets,
        source.influence_vertex_offsets
    );
    assert_eq!(output.influence_joints, source.influence_joints);
    assert_eq!(
        output.influence_source_orders,
        source.influence_source_orders
    );
    assert_float_bits(&output.influence_weights, &[0.25, -0.0, 2.0]);
    assert_float_bits(&output.morph_weights, &[-0.0, -2.0, 1.25, -0.0]);
    assert_eq!(output.morph_sparse_indices, [1]);
    assert!(matches!(output.morph_deltas[1].payload,
        StoredMorphDeltaPayload::Sparse { logical_count: 3, indices, values }
            if indices == TableRange::new(0, 1) && values == TableRange::new(9, 3)));
    assert!(matches!(output.morph_deltas[3].payload,
        StoredMorphDeltaPayload::Sparse { logical_count: 3, indices, values }
            if indices == TableRange::new(1, 0) && values == TableRange::new(21, 0)));
    assert!(matches!(output.morph_deltas[4].payload,
        StoredMorphDeltaPayload::Dense(values) if values == TableRange::new(21, 0)));
    assert!(output.morph_targets[3].deltas.is_empty());
    assert!(
        output.primitives[1]
            .influences
            .unwrap()
            .influences
            .is_empty()
    );
    assert!(output.primitives[2].influences.is_none());
    assert_eq!(
        output.mesh_instances[2].morph_weights,
        Some(TableRange::new(4, 0))
    );
    assert!(output.mesh_instances[3].morph_weights.is_none());
    let mut validation = Control::default();
    assert!(
        plan.validate_correspondence(&output, &mut validation)
            .unwrap()
    );
    assert_eq!(validation.completed, 72);
    assert_eq!(validation.strings, 0);
    assert_eq!(
        DeformationStoragePlan::for_storage(&output).unwrap(),
        DeformationStoragePlan::for_storage(&source).unwrap(),
    );
    assert_matrix_bits(source.inverse_bind_matrices[0], singular());
    assert_float_bits(&source.morph_values[..3], &[1.0, 2.0, 3.0]);
}

#[test]
fn a_singular_inline_correction_is_converted_without_decomposition_or_inversion() {
    let mut source = fixture();
    source.skins[0].matrix_spaces = correction(singular());
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let output = plan.materialize(&mut Control::default()).unwrap();
    let matrix = output.skins[0].matrix_spaces.correction().unwrap().matrix();
    assert!(matrix.is_finite() && matrix.is_affine());
    assert_matrix_bits(matrix, singular_expected());
    assert!(
        plan.validate_correspondence(&output, &mut Control::default())
            .unwrap()
    );
}

#[test]
fn identity_and_winding_only_are_bit_exact_and_planning_allocates_no_cache() {
    let source = fixture();
    let winding = SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            1.0,
            FrontFaceWinding::Clockwise,
        )
        .unwrap(),
        None,
    )
    .unwrap();
    for conversion in [units(1.0), winding] {
        let mut plan = None;
        let allocations = allocation_counter::measure(|| {
            plan = Some(
                DeformationSpacePlan::try_new(&source, conversion, &mut Control::default())
                    .unwrap(),
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        let plan = plan.unwrap();
        assert!(!plan.changes_storage());
        assert!(
            plan.validate_correspondence(&source, &mut Control::default())
                .unwrap()
        );
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert!(
            plan.validate_correspondence(&output, &mut Control::default())
                .unwrap()
        );
        assert_matrix_bits(output.inverse_bind_matrices[0], singular());
    }
}

fn vectors(semantic: MorphDeltaSemantic, values: Vec<f32>) -> DeformationStorage {
    let length = values.len();
    DeformationStorage {
        skins: Vec::new(),
        logical_morphs: vec![StoredLogicalMorph {
            definitions: TableRange::new(0, 1),
            default_weights: TableRange::new(0, 1),
        }],
        morph_definitions: vec![StoredMorphDefinition {
            logical_mesh: StoredLogicalMeshIndex(0),
            target_ordinal: 0,
            stable_identity: MorphTargetIdentity::new(0),
            name: None,
        }],
        primitives: vec![StoredPrimitiveDeformation {
            influences: None,
            morph_targets: TableRange::new(0, 1),
        }],
        morph_targets: vec![StoredMorphTarget {
            primitive: 0,
            target_ordinal: 0,
            deltas: TableRange::new(0, 1),
        }],
        morph_deltas: vec![StoredMorphDelta {
            target: StoredMorphTargetIndex(0),
            semantic,
            payload: StoredMorphDeltaPayload::Dense(TableRange::new(0, length)),
        }],
        mesh_instances: Vec::new(),
        joints: Vec::new(),
        inverse_bind_matrices: Vec::new(),
        names: Vec::new(),
        influence_vertex_offsets: Vec::new(),
        influence_joints: Vec::new(),
        influence_weights: Vec::new(),
        influence_source_orders: Vec::new(),
        morph_weights: vec![-2.0],
        morph_values: values,
        morph_sparse_indices: Vec::new(),
    }
}

#[test]
fn normal_and_three_lane_tangent_ignore_units_and_keep_authored_magnitudes() {
    for semantic in [MorphDeltaSemantic::Normal, MorphDeltaSemantic::Tangent] {
        let source = vectors(semantic, vec![f32::MAX, f32::from_bits(1), -0.0]);
        for target_units in [1.0e-300, 1.0e300] {
            let plan = DeformationSpacePlan::try_new(
                &source,
                units(target_units),
                &mut Control::default(),
            )
            .unwrap();
            assert!(!plan.changes_storage());
            assert!(
                plan.validate_correspondence(&source, &mut Control::default())
                    .unwrap()
            );
        }
        let plan =
            DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_float_bits(&output.morph_values, &[-f32::MAX, 0.0, f32::from_bits(1)]);
        assert_float_bits(&output.morph_weights, &[-2.0]);
    }
}

#[test]
fn change_detection_includes_signed_zero_but_not_empty_or_unaffected_payloads() {
    let source = vectors(MorphDeltaSemantic::Normal, vec![0.0, 0.0, 0.0]);
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    assert!(plan.changes_storage());
    let output = plan.materialize(&mut Control::default()).unwrap();
    assert_float_bits(&output.morph_values, &[-0.0, -0.0, 0.0]);

    let mut source = vectors(MorphDeltaSemantic::Position, Vec::new());
    source.morph_deltas[0].payload = StoredMorphDeltaPayload::Sparse {
        logical_count: 0,
        indices: TableRange::new(0, 0),
        values: TableRange::new(0, 0),
    };
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    assert!(!plan.changes_storage());
    let output = plan.materialize(&mut Control::default()).unwrap();
    assert!(matches!(
        output.morph_deltas[0].payload,
        StoredMorphDeltaPayload::Sparse { .. }
    ));
    assert!(
        plan.validate_correspondence(&output, &mut Control::default())
            .unwrap()
    );
}

#[test]
fn numeric_errors_name_the_correction_bind_or_explicit_delta_tuple_without_owner_search() {
    for (value, conversion, expected) in [
        (
            f32::MAX,
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            f32::from_bits(1),
            units(2.0),
            NumericConversionError::UnderflowToZero,
        ),
    ] {
        let mut source = fixture();
        let mut matrix = mirrored();
        matrix.cols[3][0] = value;
        source.skins[1].matrix_spaces = correction(matrix);
        assert!(matches!(
            DeformationSpacePlan::try_new(&source, conversion, &mut Control::default()),
            Err(DeformationSpacePlanError::Numeric {
                location: DeformationSpaceNumericLocation::SkinCorrection { skin: 1 },
                source,
            }) if source == expected
        ));
        let mut source = fixture();
        source.inverse_bind_matrices[1].cols[3][1] = value;
        assert!(matches!(
            DeformationSpacePlan::try_new(&source, conversion, &mut Control::default()),
            Err(DeformationSpacePlanError::Numeric {
                location: DeformationSpaceNumericLocation::InverseBind { matrix: 1 },
                source,
            }) if source == expected
        ));
        let source = vectors(
            MorphDeltaSemantic::Position,
            vec![0.0, 0.0, 0.0, 1.0, value, 2.0],
        );
        assert!(matches!(
            DeformationSpacePlan::try_new(&source, conversion, &mut Control::default()),
            Err(DeformationSpacePlanError::Numeric {
                location: DeformationSpaceNumericLocation::MorphDelta { delta: 0, tuple: 1 },
                source,
            }) if source == expected
        ));
    }
    let mut source = vectors(
        MorphDeltaSemantic::Position,
        vec![0.0, 0.0, 0.0, f32::MAX, 0.0, 0.0],
    );
    source.morph_deltas[0].payload = StoredMorphDeltaPayload::Sparse {
        logical_count: 20,
        indices: TableRange::new(0, 2),
        values: TableRange::new(0, 6),
    };
    source.morph_sparse_indices = vec![3, 17];
    assert!(matches!(
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()),
        Err(DeformationSpacePlanError::Numeric {
            location: DeformationSpaceNumericLocation::MorphDelta { delta: 0, tuple: 1 },
            ..
        })
    ));
    // Change detection must continue preflight after the first changed tuple.
    let source = vectors(
        MorphDeltaSemantic::Position,
        vec![1.0, 2.0, 3.0, f32::MAX, 0.0, 0.0],
    );
    assert!(matches!(
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()),
        Err(DeformationSpacePlanError::Numeric {
            location: DeformationSpaceNumericLocation::MorphDelta { delta: 0, tuple: 1 },
            ..
        })
    ));
}

#[test]
fn shape_and_compact_value_coverage_use_the_production_factory_and_checked_ranges() {
    let mut source = fixture();
    source.influence_weights.pop();
    let mut control = Control::default();
    assert!(matches!(
        DeformationSpacePlan::try_new(&source, reflected(), &mut control),
        Err(DeformationSpacePlanError::Plan(
            StorePlanError::InvalidBlockTable
        ))
    ));
    assert_eq!(control.completed, 0);
    for mutate in [
        (|s: &mut DeformationStorage| s.morph_values.extend_from_slice(&[0.0; 3]))
            as fn(&mut DeformationStorage),
        |s| s.morph_deltas[0].payload = StoredMorphDeltaPayload::Dense(TableRange::new(1, 9)),
        |s| s.morph_deltas[0].payload = StoredMorphDeltaPayload::Dense(TableRange::new(0, 8)),
        |s| {
            s.morph_deltas[1].payload = StoredMorphDeltaPayload::Sparse {
                logical_count: 3,
                indices: TableRange::new(0, 0),
                values: TableRange::new(9, 3),
            }
        },
    ] {
        let mut source = fixture();
        mutate(&mut source);
        assert!(matches!(
            DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()),
            Err(DeformationSpacePlanError::Invariant)
        ));
    }
}

#[test]
fn correspondence_rejects_changed_bits_descriptors_preserved_pools_and_foreign_source() {
    type Mutation = fn(&mut DeformationStorage);
    let cases: &[(&str, Mutation)] = &[
        ("skin name presence", |s| s.skins[0].name = None),
        ("skin joints range", |s| {
            s.skins[0].joints = TableRange::new(1, 1)
        }),
        ("skin bind range", |s| {
            s.skins[0].inverse_bind_matrices = TableRange::new(1, 1)
        }),
        ("skeleton root", |s| s.skins[0].skeleton_root = None),
        ("correction presence", |s| {
            s.skins[0].matrix_spaces = SkinMatrixSpaces::canonical()
        }),
        ("correction spaces", |s| {
            let matrix = s.skins[0].matrix_spaces.correction().unwrap().matrix();
            s.skins[0].matrix_spaces = SkinMatrixSpaces::canonical().with_correction(
                SkinMatrixCorrection::new(
                    SkinMatrixSpace::Scene,
                    SkinMatrixSpace::MeshInstanceLocal,
                    matrix,
                )
                .unwrap(),
            );
        }),
        ("correction signed zero", |s| {
            let mut matrix = s.skins[0].matrix_spaces.correction().unwrap().matrix();
            matrix.cols[3][2] = 0.0;
            s.skins[0].matrix_spaces = correction(matrix);
        }),
        ("logical definitions", |s| {
            s.logical_morphs[0].definitions = TableRange::new(0, 1)
        }),
        ("logical weights", |s| {
            s.logical_morphs[0].default_weights = TableRange::new(1, 2)
        }),
        ("definition mesh", |s| {
            s.morph_definitions[0].logical_mesh = StoredLogicalMeshIndex(1)
        }),
        ("definition ordinal", |s| {
            s.morph_definitions[0].target_ordinal = 1
        }),
        ("stable identity", |s| {
            s.morph_definitions[0].stable_identity = MorphTargetIdentity::new(0)
        }),
        ("definition empty name", |s| {
            s.morph_definitions[0].name = None
        }),
        ("influence presence", |s| s.primitives[1].influences = None),
        ("influence offsets range", |s| {
            s.primitives[0].influences.as_mut().unwrap().vertex_offsets = TableRange::new(1, 4)
        }),
        ("influence columns range", |s| {
            s.primitives[0].influences.as_mut().unwrap().influences = TableRange::new(1, 2)
        }),
        ("primitive target range", |s| {
            s.primitives[0].morph_targets = TableRange::new(1, 1)
        }),
        ("target primitive", |s| s.morph_targets[0].primitive = 1),
        ("target ordinal", |s| s.morph_targets[0].target_ordinal = 1),
        ("target delta range", |s| {
            s.morph_targets[0].deltas = TableRange::new(1, 2)
        }),
        ("delta target", |s| {
            s.morph_deltas[0].target = StoredMorphTargetIndex(1)
        }),
        ("delta semantic", |s| {
            s.morph_deltas[0].semantic = MorphDeltaSemantic::Normal
        }),
        ("delta dense range", |s| {
            s.morph_deltas[0].payload = StoredMorphDeltaPayload::Dense(TableRange::new(3, 9))
        }),
        ("delta representation", |s| {
            s.morph_deltas[1].payload = StoredMorphDeltaPayload::Dense(TableRange::new(9, 3))
        }),
        ("sparse logical count", |s| {
            if let StoredMorphDeltaPayload::Sparse { logical_count, .. } =
                &mut s.morph_deltas[1].payload
            {
                *logical_count = 4;
            }
        }),
        ("sparse range", |s| {
            if let StoredMorphDeltaPayload::Sparse { indices, .. } = &mut s.morph_deltas[1].payload
            {
                *indices = TableRange::new(1, 0);
            }
        }),
        ("empty sparse presence", |s| {
            s.morph_deltas[3].payload = StoredMorphDeltaPayload::Dense(TableRange::new(21, 0))
        }),
        ("instance skin", |s| {
            s.mesh_instances[0].skin = Some(StoredSkinIndex(1))
        }),
        ("override presence", |s| {
            s.mesh_instances[2].morph_weights = None
        }),
        ("override range", |s| {
            s.mesh_instances[0].morph_weights = Some(TableRange::new(0, 2))
        }),
        ("joint reference", |s| s.joints[0] = StoredNodeIndex(1)),
        ("bind value", |s| {
            s.inverse_bind_matrices[0].cols[3][0] += 1.0
        }),
        ("bind signed zero", |s| {
            s.inverse_bind_matrices[0].cols[0][3] = -0.0
        }),
        ("name byte", |s| s.names[0] = b'S'),
        ("offset", |s| s.influence_vertex_offsets[1] = 1),
        ("palette index", |s| s.influence_joints[0] = 1),
        ("influence signed zero", |s| s.influence_weights[1] = 0.0),
        ("source tie order", |s| s.influence_source_orders[0] = 0),
        ("default weight bit", |s| s.morph_weights[0] = 0.0),
        ("override weight bit", |s| s.morph_weights[3] = 0.0),
        ("morph component", |s| s.morph_values[0] += 1.0),
        ("explicit normal zero", |s| s.morph_values[10] = 0.0),
        ("sparse vertex index", |s| s.morph_sparse_indices[0] = 0),
        ("pool length", |s| {
            s.morph_values.pop();
        }),
    ];
    let source = fixture();
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for &(name, mutate) in cases {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        mutate(&mut actual);
        assert!(
            !plan
                .validate_correspondence(&actual, &mut Control::default())
                .unwrap(),
            "{name}"
        );
    }
    let mut foreign = fixture();
    foreign.morph_weights[2] = 2.5;
    let foreign_plan =
        DeformationSpacePlan::try_new(&foreign, reflected(), &mut Control::default()).unwrap();
    let actual = foreign_plan.materialize(&mut Control::default()).unwrap();
    assert!(
        !plan
            .validate_correspondence(&actual, &mut Control::default())
            .unwrap()
    );
}

#[test]
fn every_preflight_and_correspondence_action_is_interruptible_and_cannot_be_omitted() {
    let source = fixture();
    let mut planning = Control::default();
    let plan = DeformationSpacePlan::try_new(&source, reflected(), &mut planning).unwrap();
    for before in 0..planning.completed {
        let mut control = Control {
            stop: Some(before),
            ..Control::default()
        };
        assert!(matches!(
            DeformationSpacePlan::try_new(&source, reflected(), &mut control),
            Err(DeformationSpacePlanError::Control(Interrupted(value))) if value == before
        ));
        assert_eq!(control.completed, before);
    }
    let actual = plan.materialize(&mut Control::default()).unwrap();
    for before in 0..plan.correspondence_work() {
        let mut control = Control {
            stop: Some(before),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Err(Interrupted(before))
        );
        assert_eq!(control.completed, before);
        let mut control = Control {
            omit: Some(before),
            ..Control::default()
        };
        assert!(!plan.validate_correspondence(&actual, &mut control).unwrap());
        assert_eq!(control.completed, before);
    }
}

#[test]
fn materialization_releases_all_partial_buffers_at_every_cancellation_and_unwind_prefix() {
    let source = fixture();
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for before in 0..plan.work() {
        let mut control = Control {
            stop: Some(before),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                plan.materialize(&mut control),
                Err(TypedStorageBuildError::Control(Interrupted(value))) if value == before
            ));
        });
        assert_eq!(control.completed, before);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);

        let mut control = Control {
            panic: Some(before),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            let result = catch_unwind(AssertUnwindSafe(|| plan.materialize(&mut control)));
            assert!(result.is_err());
            drop(result);
        });
        assert_eq!(control.completed, before);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
    }
    let original = fixture();
    let identity =
        DeformationSpacePlan::try_new(&original, units(1.0), &mut Control::default()).unwrap();
    assert!(
        identity
            .validate_correspondence(&source, &mut Control::default())
            .unwrap()
    );
}

#[test]
fn successful_build_owns_exactly_the_seventeen_nonempty_buffers_and_comparison_allocates_nothing() {
    let source = fixture();
    let plan =
        DeformationSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let allocations = allocation_counter::measure(|| {
        let output = plan.materialize(&mut Control::default()).unwrap();
        drop(output);
    });
    assert_eq!(allocations.count_total, 17);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
    let actual = plan.materialize(&mut Control::default()).unwrap();
    let allocations = allocation_counter::measure(|| {
        assert!(
            plan.validate_correspondence(&actual, &mut Control::default())
                .unwrap()
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn empty_storage_needs_no_work_or_allocations_and_can_be_shared() {
    let mut source = vectors(MorphDeltaSemantic::Position, Vec::new());
    source.logical_morphs.clear();
    source.morph_definitions.clear();
    source.primitives.clear();
    source.morph_targets.clear();
    source.morph_deltas.clear();
    source.morph_weights.clear();
    let mut control = Control::default();
    let plan = DeformationSpacePlan::try_new(&source, reflected(), &mut control).unwrap();
    assert_eq!(control.completed, 0);
    assert_eq!(plan.work(), 0);
    assert_eq!(plan.correspondence_work(), 0);
    assert_eq!(plan.name_bytes(), 0);
    assert!(!plan.changes_storage());
    let allocations = allocation_counter::measure(|| {
        let output = plan.materialize(&mut control).unwrap();
        assert!(plan.validate_correspondence(&output, &mut control).unwrap());
        drop(output);
    });
    assert_eq!(control.completed, 0);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn profile_selection_visits_all_owners_with_exact_aggregate_work_and_no_cache() {
    let source = fixture();
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        for (skin, expected) in [true, true].into_iter().enumerate() {
            assert_eq!(
                skin_changes(&source, skin, reflected(), &mut control),
                Ok(expected),
            );
        }
        for (delta, expected) in [true, true, true, false, false].into_iter().enumerate() {
            assert_eq!(
                morph_delta_changes(&source, delta, reflected(), &mut control),
                Ok(expected),
            );
        }
    });
    assert_eq!(control.completed, 16); // 2 skins + 2 binds + 5 deltas + 21/3 explicit tuples.
    assert_eq!(control.strings, 0);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn skin_selection_checks_later_binds_and_keeps_charging_after_an_early_change() {
    for (changed_bind, changed_correction, expected) in [
        (Some(1), false, true),
        (Some(0), false, true),
        (None, true, true),
        (None, false, false),
    ] {
        let mut source = fixture();
        // Keep the validated compact bind partition and palette references while giving skin 0
        // both matrices. Skin 1 remains present, empty, and unused by an instance.
        source.skins[0].joints = TableRange::new(0, 2);
        source.skins[0].inverse_bind_matrices = TableRange::new(0, 2);
        source.skins[1].joints = TableRange::new(2, 0);
        source.skins[1].inverse_bind_matrices = TableRange::new(2, 0);
        source.mesh_instances[1].skin = Some(StoredSkinIndex(0));
        source.inverse_bind_matrices.fill(Mat4::IDENTITY);
        if let Some(index) = changed_bind {
            source.inverse_bind_matrices[index].cols[3][0] = 1.0;
        }
        let mut matrix = Mat4::IDENTITY;
        matrix.cols[3][0] = if changed_correction { 1.0 } else { 0.0 };
        source.skins[0].matrix_spaces = correction(matrix);
        let mut control = Control::default();
        assert_eq!(
            skin_changes(&source, 0, units(0.5), &mut control),
            Ok(expected),
        );
        assert_eq!(control.completed, 3); // Correction plus both binds, regardless of the result.
        assert_eq!(control.strings, 0);
        for before in 0..3 {
            let mut control = Control {
                stop: Some(before),
                ..Control::default()
            };
            assert_eq!(
                skin_changes(&source, 0, units(0.5), &mut control),
                Err(Interrupted(before)),
            );
            assert_eq!(control.completed, before);
        }
    }
}

#[test]
fn morph_selection_checks_unchanged_prefix_changed_middle_and_every_remaining_tuple() {
    for semantic in [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ] {
        for sparse in [false, true] {
            let mut source = vectors(
                semantic,
                vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, -0.0, 0.0, -0.0],
            );
            if sparse {
                source.morph_sparse_indices = vec![1, 5, 9];
                source.morph_deltas[0].payload = StoredMorphDeltaPayload::Sparse {
                    logical_count: 12,
                    indices: TableRange::new(0, 3),
                    values: TableRange::new(0, 9),
                };
            }
            let mut control = Control::default();
            assert_eq!(
                morph_delta_changes(&source, 0, units(0.5), &mut control),
                Ok(semantic == MorphDeltaSemantic::Position),
            );
            assert_eq!(control.completed, 4); // Descriptor and all three explicit tuples.
            assert_eq!(control.strings, 0);
            for before in 0..4 {
                let mut control = Control {
                    stop: Some(before),
                    ..Control::default()
                };
                assert_eq!(
                    morph_delta_changes(&source, 0, units(0.5), &mut control),
                    Err(Interrupted(before)),
                );
                assert_eq!(control.completed, before);
            }
        }
    }
}

#[test]
fn profile_selectors_return_false_for_missing_ranges_and_unadmitted_numeric_failures() {
    let mut source = fixture();
    let mut control = Control::default();
    assert_eq!(
        skin_changes(&source, usize::MAX, reflected(), &mut control),
        Ok(false),
    );
    assert_eq!(control.completed, 1);
    source.skins[0].inverse_bind_matrices = TableRange::new(usize::MAX, 1);
    let mut control = Control::default();
    assert_eq!(
        skin_changes(&source, 0, reflected(), &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 1);
    let mut control = Control::default();
    assert_eq!(
        morph_delta_changes(&source, usize::MAX, reflected(), &mut control),
        Ok(false),
    );
    assert_eq!(control.completed, 1);
    for payload in [
        StoredMorphDeltaPayload::Dense(TableRange::new(usize::MAX, 3)),
        StoredMorphDeltaPayload::Dense(TableRange::new(0, 2)),
        StoredMorphDeltaPayload::Sparse {
            logical_count: 3,
            indices: TableRange::new(usize::MAX, 1),
            values: TableRange::new(0, 3),
        },
        StoredMorphDeltaPayload::Sparse {
            logical_count: 3,
            indices: TableRange::new(0, 0),
            values: TableRange::new(0, 3),
        },
    ] {
        source.morph_deltas[0].payload = payload;
        let mut control = Control::default();
        assert_eq!(
            morph_delta_changes(&source, 0, reflected(), &mut control),
            Ok(false)
        );
        assert_eq!(control.completed, 1);
    }
    let mut source = fixture();
    source.inverse_bind_matrices[0].cols[3][0] = f32::MAX;
    let mut control = Control::default();
    assert_eq!(
        skin_changes(&source, 0, reflected(), &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 2); // The changed correction does not mask a failing bind.
    let source = vectors(
        MorphDeltaSemantic::Position,
        vec![1.0, 2.0, 3.0, f32::MAX, 0.0, 0.0, 0.0, 0.0, 0.0],
    );
    let mut control = Control::default();
    assert_eq!(
        morph_delta_changes(&source, 0, units(0.5), &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 4); // No early return on either the change or the numeric failure.
    let source = vectors(MorphDeltaSemantic::Position, Vec::new());
    let mut control = Control::default();
    assert_eq!(
        morph_delta_changes(&source, 0, reflected(), &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 1);
}
