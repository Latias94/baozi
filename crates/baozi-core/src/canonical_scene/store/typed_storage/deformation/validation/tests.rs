use super::*;
use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::production::input::deformation::MorphTargetIdentity;
use crate::canonical_scene::store::typed_storage::{
    StoredAttribute, StoredAttributePayload, StoredAttributeSemantic, StoredLogicalMesh,
    StoredLogicalMeshIndex, StoredMeshInstance, StoredNode, StoredPrimitive, StoredTransform,
};
use crate::deform::SkinMatrixCorrection;
use crate::geometry::TopologyKind;
use crate::math::{Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(u64);

#[derive(Default)]
struct Control {
    completed: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
}

impl TypedStorageValidationControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.omit_before == Some(self.completed) {
            return Ok(None);
        }
        let result = action();
        self.completed += 1;
        Ok(Some(result))
    }
}

struct Fixture {
    storage: DeformationStorage,
    nodes: NodeStorage,
    geometry: GeometryStorage,
    counts: DeformationMeteredCounts,
}

impl Fixture {
    fn validate(
        &self,
        control: &mut Control,
    ) -> Result<Option<TypedDeformationValidation>, Interrupted> {
        let mut palette = [Some(u32::MAX); 2];
        let mut marks = [u64::MAX; 2];
        let mut scratch = DeformationValidationScratch::new(&mut palette, &mut marks);
        if !scratch.initialize(control)? {
            return Ok(None);
        }
        self.storage.validate(
            control,
            self.counts,
            &self.nodes,
            &self.geometry,
            &mut scratch,
        )
    }
}

fn fixture() -> Fixture {
    let nodes = NodeStorage {
        transform_scalar_count: 20,
        nodes: vec![
            StoredNode {
                children: TableRange::new(0, 1),
                transform: StoredTransform::Synthetic,
                name: None,
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(1, 1),
                transform: StoredTransform::Trs {
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(1.0, 1.0, 1.0),
                },
                name: None,
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(2, 0),
                transform: StoredTransform::Trs {
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(1.0, 1.0, 1.0),
                },
                name: None,
                camera: None,
                light: None,
            },
        ],
        children: vec![StoredNodeIndex(1), StoredNodeIndex(2)],
        names: Vec::new(),
    };
    let mut geometry = empty_geometry();
    for (index, (vertices, start)) in [(3, 0), (0, 9), (1, 9)].into_iter().enumerate() {
        geometry.attributes.push(StoredAttribute {
            semantic: StoredAttributeSemantic::Position,
            domain: AttributeDomain::Vertex,
            lanes: 3,
            element_count: vertices,
            payload: StoredAttributePayload::DenseF32(TableRange::new(
                start,
                vertices as usize * 3,
            )),
        });
        geometry.primitives.push(StoredPrimitive {
            topology: if index == 2 {
                TopologyKind::Points
            } else {
                TopologyKind::TriangleList
            },
            vertex_count: vertices,
            corner_count: vertices,
            face_count: u32::from(vertices != 0),
            polygon_boundaries: None,
            indices: None,
            material: None,
            attributes: TableRange::new(index, 1),
        });
    }
    geometry.f32_values = vec![0.0; 12];
    geometry.logical_meshes = vec![
        StoredLogicalMesh {
            primitives: TableRange::new(0, 2),
            name: None,
        },
        StoredLogicalMesh {
            primitives: TableRange::new(2, 1),
            name: None,
        },
    ];
    geometry.mesh_instances = vec![
        StoredMeshInstance {
            owner_node: StoredNodeIndex(1),
            logical_mesh: StoredLogicalMeshIndex(0),
        },
        StoredMeshInstance {
            owner_node: StoredNodeIndex(2),
            logical_mesh: StoredLogicalMeshIndex(0),
        },
        StoredMeshInstance {
            owner_node: StoredNodeIndex(1),
            logical_mesh: StoredLogicalMeshIndex(1),
        },
    ];
    let mut bind = Mat4::IDENTITY;
    bind.cols[0][0] = 0.0;
    bind.cols[1][0] = 0.5;
    bind.cols[3][1] = -0.0;
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        bind,
    )
    .unwrap();
    let mut storage = empty_storage();
    storage.skins.push(StoredSkin {
        name: Some(TextRange(TableRange::new(0, 3))),
        joints: TableRange::new(0, 2),
        inverse_bind_matrices: TableRange::new(0, 2),
        skeleton_root: Some(StoredNodeIndex(1)),
        matrix_spaces: SkinMatrixSpaces::canonical().with_correction(correction),
    });
    storage.joints = vec![StoredNodeIndex(2), StoredNodeIndex(1)];
    storage.inverse_bind_matrices = vec![bind, Mat4::IDENTITY];
    storage.names = "rigλ".as_bytes().to_vec();
    storage.logical_morphs = vec![
        StoredLogicalMorph {
            definitions: TableRange::new(0, 2),
            default_weights: TableRange::new(0, 2),
        },
        StoredLogicalMorph {
            definitions: TableRange::new(2, 0),
            default_weights: TableRange::new(2, 0),
        },
    ];
    storage.morph_definitions = vec![
        StoredMorphDefinition {
            logical_mesh: StoredLogicalMeshIndex(0),
            target_ordinal: 0,
            stable_identity: MorphTargetIdentity::new(1),
            name: Some(TextRange(TableRange::new(3, 2))),
        },
        StoredMorphDefinition {
            logical_mesh: StoredLogicalMeshIndex(0),
            target_ordinal: 1,
            stable_identity: MorphTargetIdentity::new(0),
            name: Some(TextRange(TableRange::new(5, 0))),
        },
    ];
    storage.primitives = vec![
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
    ];
    storage.influence_vertex_offsets = vec![0, 2, 2, 3, 0];
    storage.influence_joints = vec![1, 0, 0];
    storage.influence_weights = vec![2.0, -0.0, 0.25];
    storage.influence_source_orders = vec![1, 0, 0];
    storage.morph_targets = vec![
        StoredMorphTarget {
            primitive: 0,
            target_ordinal: 0,
            deltas: TableRange::new(0, 3),
        },
        StoredMorphTarget {
            primitive: 0,
            target_ordinal: 1,
            deltas: TableRange::new(3, 0),
        },
        StoredMorphTarget {
            primitive: 1,
            target_ordinal: 0,
            deltas: TableRange::new(3, 1),
        },
        StoredMorphTarget {
            primitive: 1,
            target_ordinal: 1,
            deltas: TableRange::new(4, 0),
        },
    ];
    storage.morph_deltas = vec![
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
                indices: TableRange::new(0, 2),
                values: TableRange::new(9, 6),
            },
        },
        StoredMorphDelta {
            target: StoredMorphTargetIndex(0),
            semantic: MorphDeltaSemantic::Tangent,
            payload: StoredMorphDeltaPayload::Sparse {
                logical_count: 3,
                indices: TableRange::new(2, 0),
                values: TableRange::new(15, 0),
            },
        },
        StoredMorphDelta {
            target: StoredMorphTargetIndex(2),
            semantic: MorphDeltaSemantic::Position,
            payload: StoredMorphDeltaPayload::Dense(TableRange::new(15, 0)),
        },
    ];
    storage.morph_values = vec![
        -0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, -0.0, 0.0, 0.0,
    ];
    storage.morph_sparse_indices = vec![0, 2];
    storage.morph_weights = vec![-0.0, 2.0, -1.5, 0.5];
    storage.mesh_instances = vec![
        StoredMeshInstanceDeformation {
            skin: Some(StoredSkinIndex(0)),
            morph_weights: Some(TableRange::new(2, 2)),
        },
        StoredMeshInstanceDeformation {
            skin: None,
            morph_weights: None,
        },
        StoredMeshInstanceDeformation {
            skin: None,
            morph_weights: Some(TableRange::new(4, 0)),
        },
    ];
    let counts = DeformationMeteredCounts::for_storage(&storage).unwrap();
    Fixture {
        storage,
        nodes,
        geometry,
        counts,
    }
}

fn empty_storage() -> DeformationStorage {
    DeformationStorage {
        skins: Vec::new(),
        logical_morphs: Vec::new(),
        morph_definitions: Vec::new(),
        primitives: Vec::new(),
        morph_targets: Vec::new(),
        morph_deltas: Vec::new(),
        mesh_instances: Vec::new(),
        joints: Vec::new(),
        inverse_bind_matrices: Vec::new(),
        names: Vec::new(),
        influence_vertex_offsets: Vec::new(),
        influence_joints: Vec::new(),
        influence_weights: Vec::new(),
        influence_source_orders: Vec::new(),
        morph_weights: Vec::new(),
        morph_values: Vec::new(),
        morph_sparse_indices: Vec::new(),
    }
}

fn empty_geometry() -> GeometryStorage {
    GeometryStorage {
        logical_meshes: Vec::new(),
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: Vec::new(),
        names: Vec::new(),
        f32_values: Vec::new(),
        f64_values: Vec::new(),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    }
}

#[test]
fn actual_rows_validate_without_allocation_or_rewriting_authored_values() {
    let fixture = fixture();
    let requirements =
        DeformationValidationScratchRequirements::try_new(fixture.counts, 2).unwrap();
    assert_eq!(requirements.palette_maxima, 2);
    assert_eq!(requirements.uniqueness_marks, 2);
    assert_eq!(requirements.initialization_work_units(), Some(4));
    // Two fixed actions, nineteen rows, and 38 actual payload elements/tuples.
    assert_eq!(fixture.counts.validation_work(), Some(59));
    let mut control = Control::default();
    let mut result = None;
    let allocations = allocation_counter::measure(|| {
        result = fixture.validate(&mut control).unwrap();
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 63);
    assert_eq!(result.unwrap().maximum_uniqueness_count, 2);
    assert_eq!(fixture.storage.influence_weights[0], 2.0);
    assert_eq!(
        fixture.storage.influence_weights[1].to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(fixture.storage.morph_weights[2], -1.5);
    assert_eq!(
        fixture.storage.morph_values[0].to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(fixture.storage.morph_sparse_indices, [0, 2]);
    assert!(fixture.storage.inverse_bind_matrices[0].is_affine());
    assert_eq!(fixture.storage.inverse_bind_matrices[0].cols[0][0], 0.0);
    assert_eq!(
        fixture.storage.morph_definitions[0].stable_identity.value(),
        1
    );
    assert!(matches!(
        fixture.storage.morph_deltas[2].payload,
        StoredMorphDeltaPayload::Sparse { indices, values, .. }
            if indices.is_empty() && values.is_empty()
    ));
    assert!(fixture.storage.primitives[1].influences.is_some());
    assert!(fixture.storage.primitives[2].influences.is_none());
    assert!(
        fixture.storage.mesh_instances[2]
            .morph_weights
            .unwrap()
            .is_empty()
    );
    assert!(fixture.storage.mesh_instances[1].morph_weights.is_none());
}

type Corruption = fn(&mut Fixture);

fn reject_cases(cases: &[(&str, Corruption)]) {
    for &(name, corrupt) in cases {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert_eq!(
            fixture.validate(&mut Control::default()).unwrap(),
            None,
            "{name}"
        );
    }
}

#[test]
fn actual_counts_compact_ranges_and_owner_relations_are_required() {
    reject_cases(&[
        ("sealed row count", |f| f.counts.morph_target_count -= 1),
        ("influence column length", |f| {
            f.storage.influence_weights.pop();
        }),
        ("geometry-aligned row count", |f| {
            f.geometry.mesh_instances.pop();
        }),
        ("overflowed range", |f| {
            f.storage.logical_morphs[0].definitions = TableRange::new(usize::MAX, 2);
        }),
        ("unowned name bytes", |f| {
            f.storage.names.push(b'x');
            f.counts = DeformationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
        ("split UTF-8 name", |f| {
            f.storage.morph_definitions[0].name = Some(TextRange(TableRange::new(3, 1)));
        }),
        ("name control character", |f| f.storage.names[0] = b'\n'),
        ("oversized entity name", |f| {
            f.storage.names = vec![b'x'; MAX_ENTITY_NAME_BYTES + 1];
            f.storage.skins[0].name = Some(TextRange(TableRange::new(0, f.storage.names.len())));
            f.counts = DeformationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
        ("wrong definition owner", |f| {
            f.storage.morph_definitions[0].logical_mesh = StoredLogicalMeshIndex(1);
        }),
        ("wrong definition ordinal", |f| {
            f.storage.morph_definitions[0].target_ordinal = 1
        }),
        ("duplicate stable identity", |f| {
            f.storage.morph_definitions[1].stable_identity = MorphTargetIdentity::new(1);
        }),
        ("stable identity outside permutation", |f| {
            f.storage.morph_definitions[0].stable_identity = MorphTargetIdentity::new(2);
        }),
        ("geometry primitive ownership gap", |f| {
            f.geometry.logical_meshes[1].primitives = TableRange::new(3, 0);
        }),
        ("primitive target width", |f| {
            f.storage.primitives[1].morph_targets = TableRange::new(2, 1)
        }),
        ("target owner", |f| f.storage.morph_targets[0].primitive = 1),
        ("target ordinal", |f| {
            f.storage.morph_targets[0].target_ordinal = 1
        }),
        ("delta owner", |f| {
            f.storage.morph_deltas[0].target = StoredMorphTargetIndex(1)
        }),
        ("target delta overlap", |f| {
            f.storage.morph_targets[1].deltas = TableRange::new(2, 0)
        }),
        ("instance node", |f| {
            f.geometry.mesh_instances[0].owner_node = StoredNodeIndex(0)
        }),
        ("instance mesh", |f| {
            f.geometry.mesh_instances[0].logical_mesh = StoredLogicalMeshIndex(2)
        }),
    ]);
}

#[test]
fn skins_check_real_nodes_matrix_contents_and_correction_spaces() {
    reject_cases(&[
        ("synthetic joint", |f| {
            f.storage.joints[0] = StoredNodeIndex(0)
        }),
        ("joint out of range", |f| {
            f.storage.joints[0] = StoredNodeIndex(3)
        }),
        ("full-domain joint index", |f| {
            f.storage.joints[0] = StoredNodeIndex(u32::MAX)
        }),
        ("skeleton root", |f| {
            f.storage.skins[0].skeleton_root = Some(StoredNodeIndex(0))
        }),
        ("inverse bind width", |f| {
            f.storage.skins[0].inverse_bind_matrices = TableRange::new(0, 1)
        }),
        ("nonfinite inverse bind", |f| {
            f.storage.inverse_bind_matrices[0].cols[0][0] = f32::NAN
        }),
        ("nonfinite inverse bind homogeneous row", |f| {
            f.storage.inverse_bind_matrices[1].cols[3][3] = f32::INFINITY
        }),
        ("nonaffine inverse bind projective row", |f| {
            f.storage.inverse_bind_matrices[0].cols[0][3] = 0.5
        }),
        ("nonaffine inverse bind homogeneous coordinate", |f| {
            f.storage.inverse_bind_matrices[1].cols[3][3] = 2.0
        }),
        ("nonaffine correction projective row", |f| {
            let mut matrix = Mat4::IDENTITY;
            matrix.cols[2][3] = -0.25;
            corrupt_correction_matrix(f, matrix);
        }),
        ("nonaffine correction homogeneous coordinate", |f| {
            let mut matrix = Mat4::IDENTITY;
            matrix.cols[3][3] = 0.0;
            corrupt_correction_matrix(f, matrix);
        }),
        ("nonfinite correction", |f| {
            let mut matrix = Mat4::IDENTITY;
            matrix.cols[1][0] = f32::NAN;
            corrupt_correction_matrix(f, matrix);
        }),
        ("correction spaces", |f| {
            f.storage.skins[0].matrix_spaces = SkinMatrixSpaces::canonical().with_correction(
                SkinMatrixCorrection::new(
                    SkinMatrixSpace::Scene,
                    SkinMatrixSpace::MeshInstanceLocal,
                    Mat4::IDENTITY,
                )
                .unwrap(),
            );
        }),
        ("instance skin", |f| {
            f.storage.mesh_instances[0].skin = Some(StoredSkinIndex(1))
        }),
    ]);
}

fn corrupt_correction_matrix(fixture: &mut Fixture, matrix: Mat4) {
    let mut correction = fixture.storage.skins[0].matrix_spaces.correction().unwrap();
    correction.corrupt_matrix_for_test(matrix);
    fixture.storage.skins[0].matrix_spaces =
        SkinMatrixSpaces::canonical().with_correction(correction);
}

#[test]
fn affine_skin_matrices_preserve_shear_singularity_reflection_and_signed_zero_bits() {
    let singular = Mat4 {
        cols: [
            [0.0, -0.0, 0.0, -0.0],
            [0.5, 2.0, 0.0, 0.0],
            [0.0, 0.0, -3.0, -0.0],
            [6.0, -7.0, 8.0, 1.0],
        ],
    };
    let reflected = Mat4 {
        cols: [
            [-2.0, -0.0, 0.0, -0.0],
            [0.5, 3.0, 0.0, 0.0],
            [0.0, 0.0, 4.0, -0.0],
            [1.0, 2.0, 3.0, 1.0],
        ],
    };
    assert!(singular.is_finite() && singular.is_affine());
    assert!(reflected.is_finite() && reflected.is_affine());
    let bits = |matrix: Mat4| matrix.cols.map(|column| column.map(f32::to_bits));
    for (bind, correction) in [(singular, reflected), (reflected, singular)] {
        let mut fixture = fixture();
        fixture.storage.inverse_bind_matrices[0] = bind;
        fixture.storage.skins[0].matrix_spaces = SkinMatrixSpaces::canonical().with_correction(
            SkinMatrixCorrection::new(
                SkinMatrixSpace::LogicalMeshLocalBind,
                SkinMatrixSpace::MeshInstanceLocal,
                correction,
            )
            .unwrap(),
        );
        let mut control = Control::default();
        assert!(fixture.validate(&mut control).unwrap().is_some());
        assert_eq!(control.completed, 63);
        assert_eq!(bits(fixture.storage.inverse_bind_matrices[0]), bits(bind));
        assert_eq!(
            bits(
                fixture.storage.skins[0]
                    .matrix_spaces
                    .correction()
                    .unwrap()
                    .matrix()
            ),
            bits(correction),
        );
    }
}

#[test]
fn csr_rows_require_bounded_offsets_finite_weights_and_source_order_permutations() {
    reject_cases(&[
        ("first offset", |f| {
            f.storage.influence_vertex_offsets[0] = 1
        }),
        ("decreasing offset", |f| {
            f.storage.influence_vertex_offsets[2] = 1
        }),
        ("final offset", |f| {
            f.storage.influence_vertex_offsets[3] = 2
        }),
        ("offset out of range", |f| {
            f.storage.influence_vertex_offsets[1] = 4
        }),
        ("row count", |f| f.geometry.primitives[0].vertex_count = 4),
        ("duplicate source order", |f| {
            f.storage.influence_source_orders[0] = 0
        }),
        ("source order outside row", |f| {
            f.storage.influence_source_orders[2] = 1
        }),
        ("negative weight", |f| {
            f.storage.influence_weights[0] = -0.25
        }),
        ("nonfinite weight", |f| {
            f.storage.influence_weights[0] = f32::INFINITY
        }),
        ("zero weight still selects palette entry", |f| {
            f.storage.influence_weights[2] = 0.0;
            f.storage.influence_joints[2] = 2;
        }),
    ]);
    let mut unskinned = fixture();
    unskinned.storage.mesh_instances[0].skin = None;
    unskinned.storage.influence_joints[2] = u32::MAX;
    assert!(
        unskinned
            .validate(&mut Control::default())
            .unwrap()
            .is_some()
    );
}

#[test]
fn every_instance_skin_must_fit_the_shared_mesh_palette() {
    let mut fixture = fixture();
    fixture.storage.skins.push(StoredSkin {
        name: None,
        joints: TableRange::new(2, 1),
        inverse_bind_matrices: TableRange::new(2, 1),
        skeleton_root: None,
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    fixture.storage.joints.push(StoredNodeIndex(1));
    fixture.storage.inverse_bind_matrices.push(Mat4::IDENTITY);
    fixture.storage.mesh_instances[1].skin = Some(StoredSkinIndex(1));
    fixture.counts = DeformationMeteredCounts::for_storage(&fixture.storage).unwrap();
    assert_eq!(fixture.validate(&mut Control::default()).unwrap(), None);

    fixture.storage.skins[1].joints = TableRange::new(2, 2);
    fixture.storage.skins[1].inverse_bind_matrices = TableRange::new(2, 2);
    fixture.storage.joints.push(StoredNodeIndex(2));
    fixture.storage.inverse_bind_matrices.push(Mat4::IDENTITY);
    fixture.counts = DeformationMeteredCounts::for_storage(&fixture.storage).unwrap();
    assert!(fixture.validate(&mut Control::default()).unwrap().is_some());
}

#[test]
fn morphs_check_semantic_order_sparse_shape_and_all_weight_value_contents() {
    reject_cases(&[
        ("duplicate semantic", |f| {
            f.storage.morph_deltas[1].semantic = MorphDeltaSemantic::Position
        }),
        ("semantic order", |f| {
            f.storage.morph_deltas[0].semantic = MorphDeltaSemantic::Normal;
            f.storage.morph_deltas[1].semantic = MorphDeltaSemantic::Position;
        }),
        ("dense width", |f| {
            f.storage.morph_deltas[0].payload =
                StoredMorphDeltaPayload::Dense(TableRange::new(0, 6))
        }),
        ("sparse logical count", |f| {
            if let StoredMorphDeltaPayload::Sparse { logical_count, .. } =
                &mut f.storage.morph_deltas[1].payload
            {
                *logical_count = 2;
            }
        }),
        ("sparse tuple width", |f| {
            if let StoredMorphDeltaPayload::Sparse { values, .. } =
                &mut f.storage.morph_deltas[1].payload
            {
                *values = TableRange::new(9, 3);
            }
        }),
        ("sparse duplicate", |f| {
            f.storage.morph_sparse_indices[1] = 0
        }),
        ("sparse out of domain", |f| {
            f.storage.morph_sparse_indices[1] = 3
        }),
        ("delta scalar", |f| f.storage.morph_values[14] = f32::NAN),
        ("default weight", |f| {
            f.storage.morph_weights[0] = f32::INFINITY
        }),
        ("instance override weight", |f| {
            f.storage.morph_weights[3] = f32::NAN
        }),
        ("override width", |f| {
            f.storage.mesh_instances[0].morph_weights = Some(TableRange::new(2, 1))
        }),
        ("defaults width", |f| {
            f.storage.logical_morphs[0].default_weights = TableRange::new(0, 1)
        }),
    ]);
}

#[test]
fn controls_reject_without_running_the_next_action_including_final_completion() {
    let fixture = fixture();
    for prefix in [0, 4, 5, 23, 62] {
        let mut rejected = Control {
            reject_before: Some(prefix),
            ..Control::default()
        };
        assert_eq!(fixture.validate(&mut rejected), Err(Interrupted(prefix)));
        assert_eq!(rejected.completed, prefix);
        let mut omitted = Control {
            omit_before: Some(prefix),
            ..Control::default()
        };
        assert_eq!(fixture.validate(&mut omitted).unwrap(), None);
        assert_eq!(omitted.completed, prefix);
    }
    let mut exact = Control {
        reject_before: Some(63),
        ..Control::default()
    };
    assert!(fixture.validate(&mut exact).unwrap().is_some());
    assert_eq!(exact.completed, 63);
}

#[test]
fn scratch_shortages_fail_closed_and_initialization_is_separately_metered() {
    let fixture = fixture();
    let mut palette = [Some(u32::MAX); 2];
    let mut marks = [u64::MAX; 2];
    let mut scratch = DeformationValidationScratch::new(&mut palette, &mut marks);
    let mut control = Control::default();
    assert!(scratch.initialize(&mut control).unwrap());
    assert_eq!(control.completed, 4);
    assert_eq!(scratch.palette_maxima, &[None, None]);
    assert_eq!(scratch.uniqueness_marks, &[0, 0]);
    let mut insufficient = DeformationValidationScratch::new(&mut palette, &mut marks[..1]);
    assert_eq!(
        fixture
            .storage
            .validate(
                &mut Control::default(),
                fixture.counts,
                &fixture.nodes,
                &fixture.geometry,
                &mut insufficient,
            )
            .unwrap(),
        None
    );
    let mut insufficient = DeformationValidationScratch::new(&mut palette[..1], &mut marks);
    assert_eq!(
        fixture
            .storage
            .validate(
                &mut Control::default(),
                fixture.counts,
                &fixture.nodes,
                &fixture.geometry,
                &mut insufficient,
            )
            .unwrap(),
        None
    );
}

#[test]
fn empty_domain_has_two_controlled_checks_and_no_scratch_or_heap() {
    let storage = empty_storage();
    let geometry = empty_geometry();
    let nodes = fixture().nodes;
    let counts = DeformationMeteredCounts::for_storage(&storage).unwrap();
    let requirements = DeformationValidationScratchRequirements::try_new(counts, 0).unwrap();
    assert_eq!(requirements.initialization_work_units(), Some(0));
    assert_eq!(counts.validation_work(), Some(2));
    let mut control = Control::default();
    let mut scratch = DeformationValidationScratch::new(&mut [], &mut []);
    let mut result = None;
    let allocations = allocation_counter::measure(|| {
        result = storage
            .validate(&mut control, counts, &nodes, &geometry, &mut scratch)
            .unwrap();
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(result.unwrap().maximum_uniqueness_count, 0);
    assert_eq!(control.completed, 2);
}

#[test]
fn counts_reject_unpaired_columns_and_do_not_truncate_combined_pools() {
    assert!(DeformationMeteredCounts::try_new([u64::from(u32::MAX) + 1; 7], [0; 10]).is_none());
    let mut payloads = [0; 10];
    payloads[1] = 1;
    assert!(DeformationMeteredCounts::try_new([0; 7], payloads).is_none());
    payloads = [0; 10];
    payloads[5] = 1;
    assert!(DeformationMeteredCounts::try_new([0; 7], payloads).is_none());
    payloads = [0; 10];
    payloads[8] = 1;
    assert!(DeformationMeteredCounts::try_new([0; 7], payloads).is_none());
    #[cfg(target_pointer_width = "64")]
    {
        payloads = [0; 10];
        payloads[2] = u64::from(u32::MAX) + 1;
        payloads[7] = u64::from(u32::MAX) + 1;
        let counts = DeformationMeteredCounts::try_new([0; 7], payloads).unwrap();
        assert_eq!(counts.name_bytes, payloads[2]);
        assert_eq!(counts.morph_weight_count, payloads[7]);
    }
}
