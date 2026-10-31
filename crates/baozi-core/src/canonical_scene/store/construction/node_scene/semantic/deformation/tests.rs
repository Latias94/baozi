use crate::canonical_scene::store::production::input::deformation::MorphTargetIdentity;
use crate::canonical_scene::store::typed_storage::StoredLogicalMeshIndex;
use crate::canonical_scene::store::typed_storage::deformation::{
    StoredInfluenceRows, StoredLogicalMorph, StoredMeshInstanceDeformation, StoredMorphDefinition,
    StoredMorphDelta, StoredMorphTarget, StoredMorphTargetIndex, StoredPrimitiveDeformation,
    StoredSkinIndex,
};
use crate::deform::{SkinMatrixCorrection, SkinMatrixSpace};

use super::*;

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

fn fixture() -> DeformationStorage {
    let mut storage = empty_storage();
    let mut reflection_shear = Mat4::IDENTITY;
    reflection_shear.cols[0][0] = -1.0;
    reflection_shear.cols[0][1] = 0.25;
    reflection_shear.cols[0][3] = -0.0;
    reflection_shear.cols[1][0] = -0.0;
    let mut singular = Mat4::IDENTITY;
    singular.cols[2][2] = 0.0;
    singular.cols[2][1] = -0.0;
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        reflection_shear,
    )
    .unwrap();
    let singular_correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        singular,
    )
    .unwrap();
    storage.skins = vec![
        StoredSkin {
            name: Some(TextRange(TableRange::new(0, 3))),
            joints: TableRange::new(0, 2),
            inverse_bind_matrices: TableRange::new(0, 2),
            skeleton_root: Some(StoredNodeIndex(1)),
            matrix_spaces: SkinMatrixSpaces::canonical().with_correction(correction),
        },
        StoredSkin {
            name: Some(TextRange(TableRange::new(3, 0))),
            joints: TableRange::new(2, 0),
            inverse_bind_matrices: TableRange::new(2, 0),
            skeleton_root: None,
            matrix_spaces: SkinMatrixSpaces::canonical().with_correction(singular_correction),
        },
    ];
    storage.joints = vec![StoredNodeIndex(1), StoredNodeIndex(3)];
    storage.inverse_bind_matrices = vec![reflection_shear, singular];
    storage.names = "sα".as_bytes().to_vec();
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
            name: None,
        },
        StoredMorphDefinition {
            logical_mesh: StoredLogicalMeshIndex(0),
            target_ordinal: 1,
            stable_identity: MorphTargetIdentity::new(0),
            name: Some(TextRange(TableRange::new(3, 0))),
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
            morph_targets: TableRange::new(2, 0),
        },
        StoredPrimitiveDeformation {
            influences: None,
            morph_targets: TableRange::new(2, 0),
        },
    ];
    storage.influence_vertex_offsets = vec![0, 2, 2, 3, 0];
    storage.influence_joints = vec![1, 0, 1];
    storage.influence_weights = vec![0.25, -0.0, 0.75];
    storage.influence_source_orders = vec![1, 0, 0];
    storage.morph_targets = vec![
        StoredMorphTarget {
            primitive: 0,
            target_ordinal: 0,
            deltas: TableRange::new(0, 1),
        },
        StoredMorphTarget {
            primitive: 0,
            target_ordinal: 1,
            deltas: TableRange::new(1, 2),
        },
    ];
    storage.morph_deltas = vec![
        StoredMorphDelta {
            target: StoredMorphTargetIndex(0),
            semantic: MorphDeltaSemantic::Position,
            payload: StoredMorphDeltaPayload::Dense(TableRange::new(0, 9)),
        },
        StoredMorphDelta {
            target: StoredMorphTargetIndex(1),
            semantic: MorphDeltaSemantic::Position,
            payload: StoredMorphDeltaPayload::Sparse {
                logical_count: 3,
                indices: TableRange::new(0, 1),
                values: TableRange::new(9, 3),
            },
        },
        StoredMorphDelta {
            target: StoredMorphTargetIndex(1),
            semantic: MorphDeltaSemantic::Normal,
            payload: StoredMorphDeltaPayload::Sparse {
                logical_count: 3,
                indices: TableRange::new(1, 0),
                values: TableRange::new(12, 0),
            },
        },
    ];
    storage.morph_values = vec![-0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, -0.0, 0.0, 0.0];
    storage.morph_sparse_indices = vec![1];
    storage.mesh_instances = vec![
        StoredMeshInstanceDeformation {
            skin: Some(StoredSkinIndex(0)),
            morph_weights: None,
        },
        StoredMeshInstanceDeformation {
            skin: None,
            morph_weights: Some(TableRange::new(2, 0)),
        },
        StoredMeshInstanceDeformation {
            skin: None,
            morph_weights: Some(TableRange::new(2, 2)),
        },
    ];
    storage.morph_weights = vec![-0.0, 0.75, 0.5, -0.0];
    storage
}

// The tests exercise projection and traversal independently of central proof construction.
fn tables(storage: &DeformationStorage) -> NodeSceneDeformationTables<'_> {
    NodeSceneDeformationTables {
        storage,
        source_node_count: 3,
    }
}

#[test]
fn direct_reads_select_one_row_without_reading_a_broken_prefix() {
    let mut storage = fixture();
    storage.skins[0].joints = TableRange::new(usize::MAX, 2);
    storage.morph_deltas[0].payload =
        StoredMorphDeltaPayload::Dense(TableRange::new(usize::MAX, 9));
    take_row_read_count_for_test();
    assert_eq!(tables(&storage).skin_at(1).unwrap().name(), Some(""));
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables(&storage)
            .morph_delta_at(2)
            .unwrap()
            .into_payload()
            .explicit_count(),
        0
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert!(tables(&storage).skin_at(usize::MAX).is_err());
    assert!(tables(&storage).morph_delta_at(usize::MAX).is_err());
    assert_eq!(take_row_read_count_for_test(), 0);
    assert!(tables(&storage).skin_at(0).is_err());
    assert!(tables(&storage).morph_delta_at(0).is_err());
    assert_eq!(take_row_read_count_for_test(), 2);
}

#[test]
fn typed_joints_and_affine_matrices_preserve_source_indices_and_bits() {
    let storage = fixture();
    let mut skin = tables(&storage).skin_at(0).unwrap();
    assert_eq!(skin.name(), Some("sα"));
    assert_eq!(skin.skeleton_root(), Some(0));
    let mut joints = skin.take_joints().unwrap();
    assert_eq!(joints.next().unwrap(), Some(0));
    assert_eq!(joints.next().unwrap(), Some(2));
    assert_eq!(joints.next().unwrap(), None);
    joints.finish().unwrap();
    let borrowed = tables(&storage)
        .skin_at(0)
        .unwrap()
        .take_joints()
        .unwrap()
        .into_slice();
    assert!(std::ptr::eq(borrowed, storage.joints.as_slice()));

    let mut matrices = skin.take_inverse_bind_matrices().unwrap();
    let first = matrices.next().unwrap().unwrap();
    assert!(first.is_affine());
    assert_eq!(first, storage.inverse_bind_matrices[0]);
    assert_eq!(first.cols[0][0], -1.0);
    assert_eq!(first.cols[0][1], 0.25);
    assert_eq!(first.cols[0][3].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(first.cols[1][0].to_bits(), (-0.0_f32).to_bits());
    let second = matrices.next().unwrap().unwrap();
    assert!(second.is_affine());
    assert_eq!(second, storage.inverse_bind_matrices[1]);
    assert_eq!(second.cols[2][2], 0.0);
    assert_eq!(second.cols[2][1].to_bits(), (-0.0_f32).to_bits());
    matrices.finish().unwrap();
    let mut iter = tables(&storage)
        .skin_at(0)
        .unwrap()
        .take_inverse_bind_matrices()
        .unwrap()
        .into_matrices();
    assert_eq!(iter.len(), 2);
    assert_eq!(iter.clone().next_back().unwrap().unwrap(), second);
    assert_eq!(iter.next().unwrap().unwrap(), first);
    assert_eq!(skin.matrix_spaces().correction().unwrap().matrix(), first);
    assert_eq!(
        skin.matrix_spaces().correction().unwrap().matrix().cols[0][3].to_bits(),
        (-0.0_f32).to_bits()
    );
    let singular_correction = tables(&storage)
        .skin_at(1)
        .unwrap()
        .matrix_spaces()
        .correction()
        .unwrap()
        .matrix();
    assert_eq!(singular_correction, second);
    assert_eq!(
        singular_correction.cols[2][1].to_bits(),
        (-0.0_f32).to_bits()
    );

    let canonical = [StoredNodeIndex(0)];
    assert!(
        NodeSceneDeformationJointCursor::new(&canonical, 3)
            .next()
            .is_err()
    );
}

#[test]
fn borrowed_payloads_preserve_parallel_columns_optional_empties_and_morph_identity() {
    let storage = fixture();
    let allocations = allocation_counter::measure(|| {
        let (offsets, joints, weights, orders) = tables(&storage)
            .primitive_at(0)
            .unwrap()
            .take_influences()
            .unwrap()
            .into_slices();
        assert!(std::ptr::eq(
            offsets,
            &storage.influence_vertex_offsets[..4]
        ));
        assert!(std::ptr::eq(joints, storage.influence_joints.as_slice()));
        assert!(std::ptr::eq(weights, storage.influence_weights.as_slice()));
        assert!(std::ptr::eq(
            orders,
            storage.influence_source_orders.as_slice()
        ));
        assert_eq!(weights[1].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(
            tables(&storage)
                .primitive_at(1)
                .unwrap()
                .take_influences()
                .unwrap()
                .vertex_count(),
            0
        );
        assert!(
            !tables(&storage)
                .primitive_at(2)
                .unwrap()
                .influences_present()
        );

        let dense = tables(&storage).morph_delta_at(0).unwrap().into_payload();
        assert_eq!(dense.logical_count(), 3);
        let NodeSceneDeformationMorphPayload::Dense { values, .. } = dense else {
            panic!("dense representation changed")
        };
        assert!(std::ptr::eq(
            values.into_slice(),
            &storage.morph_values[..9]
        ));
        let sparse = tables(&storage).morph_delta_at(1).unwrap().into_payload();
        assert_eq!(sparse.explicit_count(), 1);
        let NodeSceneDeformationMorphPayload::Sparse { values, .. } = sparse else {
            panic!("sparse representation changed")
        };
        let (indices, values) = values.into_slices();
        assert!(std::ptr::eq(
            indices,
            storage.morph_sparse_indices.as_slice()
        ));
        assert!(std::ptr::eq(values, &storage.morph_values[9..]));
        assert_eq!(values[0].to_bits(), (-0.0_f32).to_bits());
        let NodeSceneDeformationMorphPayload::Sparse {
            logical_count,
            values,
        } = tables(&storage).morph_delta_at(2).unwrap().into_payload()
        else {
            panic!("empty sparse representation changed")
        };
        assert_eq!(logical_count, 3);
        assert_eq!(values.declared_count(), 0);
        values.finish().unwrap();

        let first = tables(&storage).morph_definition_at(0).unwrap();
        let second = tables(&storage).morph_definition_at(1).unwrap();
        assert_eq!(
            (
                first.target_ordinal(),
                first.stable_identity(),
                first.name()
            ),
            (0, 1, None)
        );
        assert_eq!(
            (
                second.target_ordinal(),
                second.stable_identity(),
                second.name()
            ),
            (1, 0, Some(""))
        );
        let defaults = tables(&storage)
            .logical_morph_at(0)
            .unwrap()
            .take_default_weights()
            .unwrap()
            .into_slice();
        assert!(std::ptr::eq(defaults, &storage.morph_weights[..2]));
        assert_eq!(defaults[0].to_bits(), (-0.0_f32).to_bits());
        assert!(
            tables(&storage)
                .logical_morph_at(1)
                .unwrap()
                .take_default_weights()
                .unwrap()
                .into_slice()
                .is_empty()
        );
        assert!(
            !tables(&storage)
                .mesh_instance_at(0)
                .unwrap()
                .morph_weights_present()
        );
        let mut empty = tables(&storage).mesh_instance_at(1).unwrap();
        assert_eq!(empty.skin(), None);
        assert_eq!(empty.take_morph_weights().unwrap().into_slice(), &[]);
        let weights = tables(&storage)
            .mesh_instance_at(2)
            .unwrap()
            .take_morph_weights()
            .unwrap()
            .into_slice();
        assert!(std::ptr::eq(weights, &storage.morph_weights[2..]));
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn exhaustive_traversal_consumes_every_table_and_borrowed_payload() {
    let storage = fixture();
    let mut cursor = tables(&storage).into_cursor();
    take_row_read_count_for_test();
    while let Some(mut skin) = cursor.next_skin().unwrap() {
        let mut joints = skin.take_joints().unwrap();
        while joints.next().unwrap().is_some() {}
        joints.finish().unwrap();
        let mut matrices = skin.take_inverse_bind_matrices().unwrap();
        while matrices.next().unwrap().is_some() {}
        matrices.finish().unwrap();
    }
    while let Some(mut logical) = cursor.next_logical_morph().unwrap() {
        let mut weights = logical.take_default_weights().unwrap();
        while weights.next().unwrap().is_some() {}
        weights.finish().unwrap();
    }
    while cursor.next_morph_definition().unwrap().is_some() {}
    while let Some(mut primitive) = cursor.next_primitive().unwrap() {
        if let Some(mut rows) = primitive.take_influences() {
            while let Some(row) = rows.next_row().unwrap() {
                let mut influences = row.into_influences();
                while influences.next().unwrap().is_some() {}
                influences.finish().unwrap();
            }
            rows.finish().unwrap();
        }
    }
    while cursor.next_morph_target().unwrap().is_some() {}
    while let Some(delta) = cursor.next_morph_delta().unwrap() {
        match delta.into_payload() {
            NodeSceneDeformationMorphPayload::Dense { mut values, .. } => {
                while values.next().unwrap().is_some() {}
                values.finish().unwrap();
            }
            NodeSceneDeformationMorphPayload::Sparse { mut values, .. } => {
                while values.next().unwrap().is_some() {}
                values.finish().unwrap();
            }
        }
    }
    while let Some(mut instance) = cursor.next_mesh_instance().unwrap() {
        if let Some(mut weights) = instance.take_morph_weights() {
            while weights.next().unwrap().is_some() {}
            weights.finish().unwrap();
        }
    }
    cursor.finish().unwrap();
    assert_eq!(take_row_read_count_for_test(), 17);
}

#[test]
fn finish_rejects_each_unexhausted_phase_and_cannot_return_to_an_earlier_phase() {
    let storage = fixture();
    for phase in 0..7 {
        let mut cursor = tables(&storage).into_cursor();
        if phase > 0 {
            while cursor.next_skin().unwrap().is_some() {}
        }
        if phase > 1 {
            while cursor.next_logical_morph().unwrap().is_some() {}
        }
        if phase > 2 {
            while cursor.next_morph_definition().unwrap().is_some() {}
        }
        if phase > 3 {
            while cursor.next_primitive().unwrap().is_some() {}
        }
        if phase > 4 {
            while cursor.next_morph_target().unwrap().is_some() {}
        }
        if phase > 5 {
            while cursor.next_morph_delta().unwrap().is_some() {}
        }
        assert!(cursor.finish().is_err(), "phase {phase} was not exhausted");
    }
    let mut cursor = tables(&storage).into_cursor();
    assert!(cursor.next_logical_morph().is_err());
    while cursor.next_skin().unwrap().is_some() {}
    assert!(cursor.next_logical_morph().unwrap().is_some());
    assert!(cursor.next_skin().is_err());
}

#[test]
fn exhaustive_traversal_rejects_noncontiguous_empty_ranges_and_orphaned_payloads() {
    let mut storage = fixture();
    storage.skins[1].name = Some(TextRange(TableRange::new(0, 0)));
    let mut cursor = tables(&storage).into_cursor();
    assert!(cursor.next_skin().unwrap().is_some());
    assert!(cursor.next_skin().is_err());
    let mut storage = empty_storage();
    storage.morph_values = vec![0.0; 3];
    assert!(tables(&storage).into_cursor().finish().is_err());
    assert!(tables(&empty_storage()).into_cursor().finish().is_ok());
}

#[test]
fn payload_cursors_require_exhaustion_and_check_only_the_requested_item() {
    let weights = [0.0, f32::NAN];
    let mut cursor = NodeSceneDeformationF32Cursor::new(&weights);
    assert_eq!(cursor.next().unwrap(), Some(0.0));
    assert!(cursor.next().is_err());
    assert!(cursor.finish().is_err());
    assert!(NodeSceneDeformationVec3Cursor::try_new(&[0.0, 1.0]).is_err());
    assert!(
        NodeSceneDeformationVec3Cursor::try_new(&[0.0; 3])
            .unwrap()
            .finish()
            .is_err()
    );
    let mut sparse = NodeSceneDeformationSparseDeltaCursor::try_new(3, &[1, 1], &[0.0; 6]).unwrap();
    assert_eq!(sparse.next().unwrap().unwrap().index(), 1);
    assert!(sparse.next().is_err());
    assert!(sparse.finish().is_err());
    let matrices = [Mat4::IDENTITY];
    assert!(
        NodeSceneDeformationMatrixCursor::new(&matrices)
            .finish()
            .is_err()
    );
    assert!(
        NodeSceneDeformationJointCursor::new(&[StoredNodeIndex(1)], 3)
            .finish()
            .is_err()
    );
    let mut rows =
        NodeSceneDeformationInfluenceRowsCursor::try_new(&[0, 1], &[0], &[1.0], &[0]).unwrap();
    let row = rows.next_row().unwrap().unwrap();
    assert!(row.into_influences().finish().is_err());
    rows.finish().unwrap();
    assert!(
        NodeSceneDeformationInfluenceRowsCursor::try_new(&[0, 0], &[], &[], &[])
            .unwrap()
            .finish()
            .is_err()
    );
}
