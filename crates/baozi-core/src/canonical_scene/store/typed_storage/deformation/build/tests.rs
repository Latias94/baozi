use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::production::{
    AuthoringRange, CanonicalAuthoringControl, DeformationAuthoringBatch,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMeshLocalId, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphTargetDefinitionAuthoring,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, SkinLocalId, validate_deformation_batch_with_control,
    validate_geometry_batch_with_control, validate_node_batch_with_control,
};
use crate::deform::{SkinMatrixCorrection, SkinMatrixSpace};
use crate::entity::AuthoredTransform;
use crate::{SceneValidationLimits, SceneValidationWork};

use super::*;

struct AuthoringControl;

impl CanonicalAuthoringControl for AuthoringControl {
    type Error = Infallible;
    type Reservation = u64;

    fn poll(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        _work: SceneValidationWork,
        mut action: impl FnMut() -> R,
    ) -> Result<R, Self::Error> {
        Ok(action())
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        Ok(bytes)
    }

    fn validate_scratch(&mut self, _reservation: &Self::Reservation) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        *reservation = required_bytes;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(usize);

#[derive(Default)]
struct Control {
    completed: usize,
    string_completed: usize,
    reject_before: Option<usize>,
    panic_before: Option<usize>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, Interrupted> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.panic_before == Some(self.completed) {
            // Bypass the global panic hook so the allocation probe measures the build
            // buffers and the caught payload, without test-output capture allocations.
            std::panic::resume_unwind(Box::new(Interrupted(self.completed)));
        }
        let result = action();
        self.completed += 1;
        self.string_completed += usize::from(string);
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

fn node(parent: Presence<NodeParent>) -> NodeAuthoringNode<'static> {
    NodeAuthoringNode::new(
        parent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )
}

fn with_fixture<T>(
    action: impl FnOnce(&ValidatedDeformationBatch<'_, u64>, &ValidatedNodeBatch<'_, u64>) -> T,
) -> T {
    let limits = SceneValidationLimits::default();
    let mut authoring = AuthoringControl;
    // Local indices deliberately differ from the canonical root/child traversal.
    let roots = [NodeLocalId::new(2), NodeLocalId::new(1)];
    let nodes = [
        node(Presence::Present(NodeParent::new(NodeLocalId::new(2), 0))),
        node(Presence::Absent),
        node(Presence::Absent),
    ];
    let validated_nodes = validate_node_batch_with_control(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        limits,
        &mut authoring,
    )
    .unwrap();
    assert_eq!(validated_nodes.canonical_index_by_local(), [3, 2, 1]);

    let meshes = [
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2)),
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(2, 1)),
    ];
    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let geometry_primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(0, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(1, 0),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(1, 0),
        ),
    ];
    let geometry_instances = [
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(2), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(2), LogicalMeshLocalId::new(1)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(1)),
    ];
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(
            &meshes,
            &geometry_primitives,
            &attributes,
            &geometry_instances,
        ),
        nodes.len(),
        0,
        limits,
        &mut authoring,
    )
    .unwrap();

    let joints = [NodeLocalId::new(0), NodeLocalId::new(2)];
    let mut second_inverse_bind = Mat4::IDENTITY;
    second_inverse_bind.cols[0][1] = f32::from_bits(1);
    second_inverse_bind.cols[2][3] = -0.0;
    let inverse_bind_matrices = [Mat4::IDENTITY, second_inverse_bind];
    let mut correction_matrix = Mat4::IDENTITY;
    correction_matrix.cols[3][0] = -0.0;
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        correction_matrix,
    )
    .unwrap();
    let skins = [
        SkinAuthoring::new(
            Presence::Present("骨架"),
            &joints,
            &inverse_bind_matrices,
            Presence::Present(NodeLocalId::new(2)),
            SkinMatrixSpaces::canonical().with_correction(correction),
        ),
        SkinAuthoring::new(
            Presence::Present(""),
            &[],
            &[],
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        ),
        SkinAuthoring::new(
            Presence::Absent,
            &[],
            &[],
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        ),
    ];
    let definitions = [
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(1), Presence::Present("")),
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(0), Presence::Present("眉")),
    ];
    let defaults = [-0.0, 0.75];
    let logical_morphs = [
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 2), &defaults),
        LogicalMorphAuthoring::new(AuthoringRange::new(2, 0), &[]),
    ];
    let dense = [f32::from_bits(1), -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let sparse_indices = [1];
    let sparse_values = [-0.0, 0.0, 0.0];
    let first_deltas = [
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Position,
            MorphDeltaPayloadAuthoring::Dense { values: &dense },
        ),
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Normal,
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count: 3,
                indices: &sparse_indices,
                values: &sparse_values,
            },
        ),
    ];
    let empty_tangent = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Tangent,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &[],
            values: &[],
        },
    )];
    let empty_position = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 0,
            indices: &[],
            values: &[],
        },
    )];
    let first_targets = [
        PrimitiveMorphTargetAuthoring::new(&first_deltas),
        PrimitiveMorphTargetAuthoring::new(&empty_tangent),
    ];
    let second_targets = [
        PrimitiveMorphTargetAuthoring::new(&empty_position),
        PrimitiveMorphTargetAuthoring::new(&[]),
    ];
    let influence_offsets = [0, 2, 2, 3];
    let influences = [
        InfluenceAuthoring::new(1, 0.25, 1),
        InfluenceAuthoring::new(0, -0.0, 0),
        InfluenceAuthoring::new(1, 2.0, 0),
    ];
    let primitives = [
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&influence_offsets, &influences)),
            &first_targets,
        ),
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&[0], &[])),
            &second_targets,
        ),
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]),
    ];
    let overrides = [4.0, -0.0];
    let instances = [
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(0)),
            Presence::Present(&overrides),
        ),
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(0)),
            Presence::Absent,
        ),
        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Present(&[])),
        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent),
    ];
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::new(
            &skins,
            &definitions,
            &logical_morphs,
            &primitives,
            &instances,
        ),
        &validated_nodes,
        &geometry,
        limits,
        &mut authoring,
    )
    .unwrap();
    action(&deformation, &validated_nodes)
}

#[test]
fn build_owns_exact_source_bits_presence_order_and_seventeen_compact_buffers() {
    let (mut storage, plan, control) = with_fixture(|batch, nodes| {
        let plan = DeformationStoragePlan::new(batch).unwrap();
        assert_eq!(plan.row_counts(), [3, 2, 2, 3, 4, 4, 4]);
        assert_eq!(plan.payload_counts(), [2, 2, 9, 5, 3, 3, 3, 4, 12, 1]);
        assert_eq!(plan.work().unwrap(), 66);
        let mut control = Control::default();
        let storage = plan.build(batch, nodes, &mut control).unwrap();
        (storage, plan, control)
    });
    // Authoring buffers and their compiled plans have left scope. All following slices
    // borrow the new owner, including the three separate contiguous influence columns.
    assert_eq!(control.completed, 66);
    assert_eq!(control.string_completed, plan.name_bytes());
    assert_eq!(plan.name_bytes(), 9);
    assert_eq!(storage.names, "骨架眉".as_bytes());
    assert_eq!(
        storage.skins[0].name.unwrap().get(&storage.names),
        Some("骨架")
    );
    assert_eq!(storage.skins[1].name.unwrap().get(&storage.names), Some(""));
    assert!(storage.skins[2].name.is_none());
    assert_eq!(storage.skins[0].skeleton_root, Some(StoredNodeIndex(1)));
    assert_eq!(storage.joints, [StoredNodeIndex(3), StoredNodeIndex(1)]);
    assert_eq!(storage.inverse_bind_matrices[1].cols[0][1].to_bits(), 1);
    assert_eq!(
        storage.inverse_bind_matrices[1].cols[2][3].to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        storage.skins[0]
            .matrix_spaces
            .correction()
            .unwrap()
            .matrix()
            .cols[3][0]
            .to_bits(),
        (-0.0_f32).to_bits(),
    );
    assert_eq!(storage.morph_definitions[0].stable_identity.value(), 1);
    assert_eq!(storage.morph_definitions[1].stable_identity.value(), 0);
    assert_eq!(storage.morph_definitions[0].target_ordinal, 0);
    assert_eq!(storage.morph_definitions[1].target_ordinal, 1);
    assert_eq!(
        storage.morph_definitions[0]
            .name
            .unwrap()
            .get(&storage.names),
        Some("")
    );
    assert_eq!(
        storage.morph_definitions[1]
            .name
            .unwrap()
            .get(&storage.names),
        Some("眉")
    );
    assert_eq!(storage.logical_morphs[0].definitions, TableRange::new(0, 2));
    assert_eq!(storage.logical_morphs[1].definitions, TableRange::new(2, 0));

    let rows = storage.primitives[0].influences.unwrap();
    assert_eq!(
        rows.vertex_offsets
            .get(&storage.influence_vertex_offsets)
            .unwrap(),
        [0, 2, 2, 3]
    );
    assert_eq!(
        rows.influences.get(&storage.influence_joints).unwrap(),
        [1, 0, 1]
    );
    let borrowed_weights = TableRange::<f32>::new(rows.influences.start(), rows.influences.len())
        .get(&storage.influence_weights)
        .unwrap();
    assert_eq!(borrowed_weights[1].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(borrowed_weights[2], 2.0);
    assert_eq!(
        rows.influences
            .get(&storage.influence_source_orders)
            .unwrap(),
        [1, 0, 0]
    );
    let empty_rows = storage.primitives[1].influences.unwrap();
    assert_eq!(
        empty_rows
            .vertex_offsets
            .get(&storage.influence_vertex_offsets)
            .unwrap(),
        [0]
    );
    assert!(empty_rows.influences.is_empty());
    assert!(storage.primitives[2].influences.is_none());

    assert_eq!(storage.morph_deltas[0].target, StoredMorphTargetIndex(0));
    assert!(matches!(
        storage.morph_deltas[0].payload,
        StoredMorphDeltaPayload::Dense(_)
    ));
    assert_eq!(storage.morph_values[0].to_bits(), 1);
    assert_eq!(storage.morph_values[1].to_bits(), (-0.0_f32).to_bits());
    let StoredMorphDeltaPayload::Sparse {
        logical_count,
        indices,
        values,
    } = storage.morph_deltas[1].payload
    else {
        panic!("explicit sparse zero must stay sparse");
    };
    assert_eq!(logical_count, 3);
    assert_eq!(indices.get(&storage.morph_sparse_indices).unwrap(), [1]);
    assert_eq!(
        values.get(&storage.morph_values).unwrap()[0].to_bits(),
        (-0.0_f32).to_bits()
    );
    let StoredMorphDeltaPayload::Sparse {
        logical_count,
        indices,
        values,
    } = storage.morph_deltas[2].payload
    else {
        panic!("an empty sparse stream must stay present");
    };
    assert_eq!(logical_count, 3);
    assert!(indices.is_empty() && values.is_empty());
    assert!(storage.morph_targets[3].deltas.is_empty());
    assert_eq!(storage.morph_targets[2].primitive, 1);
    assert_eq!(storage.morph_targets[2].target_ordinal, 0);

    assert_eq!(
        storage.logical_morphs[0].default_weights,
        TableRange::new(0, 2)
    );
    assert_eq!(
        storage.mesh_instances[0].morph_weights,
        Some(TableRange::new(2, 2))
    );
    assert_eq!(storage.mesh_instances[0].skin, Some(StoredSkinIndex(0)));
    assert!(storage.mesh_instances[1].morph_weights.is_none());
    assert_eq!(
        storage.mesh_instances[2].morph_weights,
        Some(TableRange::new(4, 0))
    );
    assert!(storage.mesh_instances[3].morph_weights.is_none());
    assert_eq!(storage.morph_weights[0].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(storage.morph_weights[3].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(DeformationStoragePlan::for_storage(&storage).unwrap(), plan);

    // Increasing allocator capacity cannot increase the initialized canonical footprint.
    let canonical_bytes = plan.canonical_bytes().unwrap();
    storage.skins.reserve(20);
    storage.morph_values.reserve(100);
    assert_eq!(
        DeformationStoragePlan::for_storage(&storage)
            .unwrap()
            .canonical_bytes()
            .unwrap(),
        canonical_bytes
    );
}

#[test]
fn every_materialization_action_releases_partial_buffers_on_control_failure_and_unwind() {
    with_fixture(|batch, nodes| {
        let plan = DeformationStoragePlan::new(batch).unwrap();
        for before in 0..usize::try_from(plan.work().unwrap()).unwrap() {
            let mut control = Control {
                reject_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                assert!(matches!(
                    plan.build(batch, nodes, &mut control),
                    Err(TypedStorageBuildError::Control(Interrupted(completed))) if completed == before,
                ));
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);

            let mut control = Control {
                panic_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                let unwind =
                    catch_unwind(AssertUnwindSafe(|| plan.build(batch, nodes, &mut control)));
                assert!(unwind.is_err());
                drop(unwind);
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
        }
    });
}

#[test]
fn affine_bind_and_inline_correction_preserve_shear_reflection_and_singular_bits() {
    let joints = [NodeLocalId::new(1)];
    let mut inverse = Mat4::IDENTITY;
    inverse.cols[0][0] = -2.0;
    inverse.cols[1][0] = 0.5;
    inverse.cols[2][2] = 0.0;
    inverse.cols[0][3] = -0.0;
    inverse.cols[1][2] = -0.0;
    let matrices = [inverse];
    let mut correction_matrix = Mat4::IDENTITY;
    correction_matrix.cols[1][0] = -0.75;
    correction_matrix.cols[2][2] = -3.0;
    correction_matrix.cols[3][0] = f32::from_bits(1);
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        correction_matrix,
    )
    .unwrap();
    let skin = SkinAuthoring::new(
        Presence::Present(""),
        &joints,
        &matrices,
        Presence::Present(NodeLocalId::new(0)),
        SkinMatrixSpaces::canonical().with_correction(correction),
    );
    let mut authoring = AuthoringControl;
    let limits = SceneValidationLimits::default();
    let nodes = [node(Presence::Absent), node(Presence::Absent)];
    let roots = [NodeLocalId::new(1), NodeLocalId::new(0)];
    let nodes = validate_node_batch_with_control(
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        limits,
        &mut authoring,
    )
    .unwrap();
    assert_eq!(nodes.canonical_index_by_local(), [2, 1]);
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        2,
        0,
        limits,
        &mut authoring,
    )
    .unwrap();
    let skins = [skin];
    let batch = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::new(&skins, &[], &[], &[], &[]),
        &nodes,
        &geometry,
        limits,
        &mut authoring,
    )
    .unwrap();
    let plan = DeformationStoragePlan::new(&batch).unwrap();
    assert_eq!(batch.canonical_bytes(), plan.canonical_bytes().unwrap());
    assert_eq!(batch.f32_scalar_count(), 32);
    let mut control = Control::default();
    let storage = plan.build(&batch, &nodes, &mut control).unwrap();
    assert_eq!(control.completed, 3);
    assert_eq!(control.string_completed, 0);
    assert_eq!(storage.skins[0].name.unwrap().get(&storage.names), Some(""));
    assert_eq!(storage.skins[0].skeleton_root, Some(StoredNodeIndex(2)));
    for (actual, expected) in storage.inverse_bind_matrices[0]
        .cols
        .into_iter()
        .flatten()
        .zip(inverse.cols.into_iter().flatten())
    {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    let actual_correction = storage.skins[0]
        .matrix_spaces
        .correction()
        .unwrap()
        .matrix();
    for (actual, expected) in actual_correction
        .cols
        .into_iter()
        .flatten()
        .zip(correction_matrix.cols.into_iter().flatten())
    {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

#[test]
fn empty_deformation_has_seventeen_empty_capacities_and_only_its_initialized_header() {
    let limits = SceneValidationLimits::default();
    let mut authoring = AuthoringControl;
    let nodes = validate_node_batch_with_control(
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        limits,
        &mut authoring,
    )
    .unwrap();
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        0,
        0,
        limits,
        &mut authoring,
    )
    .unwrap();
    let batch = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &nodes,
        &geometry,
        limits,
        &mut authoring,
    )
    .unwrap();
    let plan = DeformationStoragePlan::new(&batch).unwrap();
    assert_eq!(
        plan.capacities()
            .unwrap()
            .map(|capacity| capacity.desired_capacity()),
        [0; 17]
    );
    assert_eq!(plan.work().unwrap(), 0);
    assert_eq!(
        plan.canonical_bytes().unwrap(),
        std::mem::size_of::<DeformationStorage>() as u64
    );
    let mut control = Control::default();
    let storage = plan.build(&batch, &nodes, &mut control).unwrap();
    assert_eq!(control.completed, 0);
    assert_eq!(DeformationStoragePlan::for_storage(&storage).unwrap(), plan);
}

#[test]
fn checked_shapes_reject_unrepresentable_rows_mismatched_columns_and_overflow() {
    // The persistent plan owns only independent counts. Derived capacity envelopes
    // must not grow every carried Scene edit plan by another seventeen large values.
    assert!(
        std::mem::size_of::<DeformationStoragePlan>() < std::mem::size_of::<[CapacityPlan; 17]>()
    );
    assert!(
        DeformationStoragePlan::for_shape([u64::from(u32::MAX) + 1, 0, 0, 0, 0, 0, 0], [0; 10])
            .is_err()
    );
    for payloads in [
        [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 1, 0, 1, 0, 0, 0],
        [0, 0, 0, 0, 1, 1, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 0],
    ] {
        assert!(matches!(
            DeformationStoragePlan::for_shape([0; 7], payloads),
            Err(StorePlanError::InvalidBlockTable)
        ));
    }
    assert!(
        DeformationStoragePlan::for_shape([0; 7], [u64::MAX, u64::MAX, 0, 0, 0, 0, 0, 0, 0, 0])
            .is_err()
    );
    assert!(matches!(
        DeformationStoragePlan::canonical_bytes_for_shape([u64::MAX; 7], [u64::MAX; 10]),
        Err(StorePlanError::SizeOverflow),
    ));
    with_fixture(|batch, nodes| {
        let empty = DeformationStoragePlan::for_shape([0; 7], [0; 10]).unwrap();
        let mut control = Control::default();
        assert!(matches!(
            empty.build(batch, nodes, &mut control),
            Err(TypedStorageBuildError::Invariant)
        ));
        assert_eq!(control.completed, 0);
    });
}
