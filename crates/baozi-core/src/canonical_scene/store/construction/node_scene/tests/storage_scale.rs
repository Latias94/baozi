//! Complete authoring -> materialization -> central validation -> publication scale fixtures.
//!
//! Run all four fixtures serially with `cargo +1.95 nextest run --locked -p baozi-core --lib
//! -E 'test(storage_scale)' --run-ignored all --test-threads 1 --success-output immediate`.
//! Fixture allocation and output inspection are outside each allocation-counter measurement.
//! Requested allocation sizes and ledger live-byte peaks are not process RSS measurements.

use std::collections::HashSet;
use std::mem::{size_of, size_of_val};

use allocation_counter::AllocationInfo;
use baozi_operation::{OperationControl, PhysicalStorageAccount, StoreLedgerEvidence};

use super::*;
use crate::canonical_scene::store::construction::node_scene::NodeSceneSuccessParts;
use crate::canonical_scene::store::construction::node_scene::edit::{
    NodeSceneEditTerminal, NodeTransformEditStart, try_begin_node_transform_edit,
};
use crate::canonical_scene::store::production::{
    GeometryMeshInstanceAuthoring, InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMeshLocalId,
    MeshInstanceDeformationAuthoring, SkinAuthoring, SkinLocalId,
};
use crate::canonical_scene::store::schema::profile::{
    ProfileMetadataBytesSchema, profile_metadata_directory_field,
};
use crate::deform::SkinMatrixSpaces;
use crate::extension::ExtensionMediaType;
use crate::geometry::TopologyKind;
use crate::identity::NodeId;
use crate::provenance::OptionsDigest;
use crate::{AttributeData, ValidatedScene};

const MIB: usize = 1024 * 1024;

fn measured_scene(
    account: &PhysicalStorageAccount,
    nodes: NodeAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
    deformation: DeformationAuthoringBatch<'_>,
    metadata: MetadataAuthoringBatch<'_>,
) -> (NodeSceneSuccessParts, AllocationInfo) {
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    let input = SceneAuthoringBatch::new(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animation,
        profile.batch_with_metadata(metadata),
    );
    let mut result = None;
    let allocations = allocation_counter::measure(|| {
        let build = NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .expect("scale input admission")
        .finish_draft()
        .expect("scale materialization and central validation");
        result = Some(successful_parts(
            build.publish_for_test().expect("scale publication"),
        ));
    });
    (result.expect("measured Scene output"), allocations)
}

fn measured_edit(
    source: &ValidatedScene,
    node_index: u32,
) -> (NodeSceneEditTerminal, AllocationInfo) {
    let mut result = None;
    let allocations = allocation_counter::measure(|| {
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            source.as_node_scene(),
            NodeId::new(source.generation(), node_index),
            trs_transform(42.0),
            OptionsDigest::sha256([0xd1; 32]),
        )
        .expect("scale transform edit planning") else {
            panic!("the replacement transform must differ from the input");
        };
        result = Some(
            pending
                .prepare(
                    source.as_node_scene(),
                    test_limits(),
                    OperationControl::default(),
                )
                .expect("scale edit materialization and central validation")
                .handoff(),
        );
    });
    (result.expect("measured edit output"), allocations)
}

fn record_measurement(
    label: &str,
    evidence: &StoreLedgerEvidence,
    allocations: AllocationInfo,
    account: &PhysicalStorageAccount,
) -> u64 {
    assert_eq!(
        evidence.header().terminal_cause(),
        StoreTerminalCause::Success
    );
    let work = evidence
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("scale store dimension");
    assert!(work.attempted() >= work.consumed());
    assert!(work.consumed() > 0);
    assert!(work.committed_canonical_output() > 0);
    assert_eq!(work.pending_publication(), 0);
    assert!(allocations.count_total > 0);
    let physical = account.snapshot();
    eprintln!(
        "{label}: requested_allocations={}, requested_bytes_total={}, requested_bytes_peak={}, \
         requested_bytes_net={}, ledger_peak_bytes={}, attempted_work={}, consumed_work={}, \
         committed_output={}, physical_live_bytes={}, physical_live_allocations={}",
        allocations.count_total,
        allocations.bytes_total,
        allocations.bytes_max,
        allocations.bytes_current,
        evidence.header().peak_live_bytes(),
        work.attempted(),
        work.consumed(),
        work.committed_canonical_output(),
        physical.live_retained_bytes(),
        physical.live_retained_allocations(),
    );
    work.consumed()
}

fn assert_account_released(account: &PhysicalStorageAccount) {
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

fn positions(scene: &ValidatedScene) -> &[f32] {
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let AttributeData::DenseF32 { lanes: 3, values } = primitive.positions().unwrap().data() else {
        panic!("scale POSITION must retain its three F32 lanes");
    };
    values
}

fn metadata_bytes(scene: &ValidatedScene) -> &[u8] {
    // Metadata has no public borrowed reader yet. Inspect the actual retained byte lane;
    // do not generate a snapshot or another copy of the metadata to observe this pilot.
    let scene = scene.as_node_scene();
    let canonical = scene.read_canonical();
    let directory = canonical
        .records(scene.read_proof().profile_metadata_directory().into_store())
        .expect("scale metadata directory");
    let bytes = directory
        .bytes_ref::<ProfileMetadataBytesSchema>(0, profile_metadata_directory_field::BYTES)
        .expect("scale metadata byte reference");
    canonical.bytes(bytes).expect("scale metadata byte payload")
}

#[test]
fn storage_scale_pipeline_preserves_admission_budget_and_cancellation() {
    let factory = test_factory();
    let account = test_account(&factory, u64::MAX);
    let indices = [0_u32, 1, 2];
    let values = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &values,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&indices),
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances);
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        &logical_morphs,
        &primitive_deformations,
        &instance_deformations,
    );
    let nodes = single_trs_batch();
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    let input = SceneAuthoringBatch::new(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animation,
        profile.batch(),
    );

    let denied = NodeSceneTestTransaction::with_account(
        &account,
        SceneConstructionLimits {
            operation_live_bytes: 0,
            ..test_limits()
        },
        input,
        SceneValidationLimits::default(),
    );
    assert!(denied.is_err(), "zero-byte operation admission must fail");
    drop(denied);
    assert_account_released(&account);

    let build = NodeSceneTestTransaction::with_account(
        &account,
        test_limits_with_work(10),
        input,
        SceneValidationLimits::default(),
    )
    .expect("bounded scale input admission")
    .finish_draft()
    .expect("work rejection must retain an exact outcome");
    let failure = rich_failure(
        build
            .publish_for_test()
            .expect("budget failure publication"),
    );
    let work = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .unwrap();
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(work.attempted(), 11);
    assert_eq!(work.consumed(), 10);
    assert_eq!(work.committed_canonical_output(), 0);
    assert_released(failure, &account);

    let cancellation = CancelOnSignalPoll::new();
    cancellation.cancel_after_additional_polls(1);
    let build = NodeSceneTestTransaction::with_account_and_cancellation(
        &account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
        cancellation,
    )
    .expect("cancellable scale input admission")
    .finish_draft()
    .expect("cancellation must retain an exact outcome");
    let failure = rich_failure(build.publish_for_test().expect("cancellation publication"));
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Control
    );
    assert_eq!(
        failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_released(failure, &account);
}

#[test]
#[ignore = "manual complete-pipeline allocation and charged-work pilot"]
fn independent_indexed_primitive_buffers_scale() {
    let mut previous_work = None;
    for count in [256_usize, 1_024, 4_096] {
        let factory = test_factory();
        let account = test_account(&factory, u64::MAX);
        let source_indices: Vec<_> = (0..count).map(|_| vec![0_u32, 1, 2]).collect();
        let source_positions: Vec<_> = (0..count)
            .map(|index| {
                let x = u16::try_from(index).unwrap();
                let x = f32::from(x);
                vec![x, 0.0, 0.0, x + 1.0, 0.0, 0.0, x, 1.0, 0.0]
            })
            .collect();
        let attributes: Vec<_> = source_positions
            .iter()
            .map(|values| {
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    values,
                    Presence::Absent,
                )
            })
            .collect();
        let primitives: Vec<_> = source_indices
            .iter()
            .enumerate()
            .map(|(index, indices)| {
                GeometryPrimitiveAuthoring::new(
                    GeometryTopologyAuthoring::TriangleList,
                    Presence::Present(indices),
                    Presence::Absent,
                    AuthoringRange::new(u32::try_from(index).unwrap(), 1),
                )
            })
            .collect();
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
            0,
            u32::try_from(count).unwrap(),
        ))];
        let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
        let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let primitive_deformations =
            vec![PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); count];
        let deformation =
            DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
        let (parts, allocations) = measured_scene(
            &account,
            single_trs_batch(),
            geometry,
            deformation,
            MetadataAuthoringBatch::empty(),
        );
        let work = record_measurement(
            &format!("independent-primitives/{count}/build"),
            parts.report.ledger_evidence(),
            allocations,
            &account,
        );
        // This bound observes charged work on the real central path. There is currently no
        // central-path cache-comparison counter; the separate cache-helper test is not one.
        if let Some(previous) = previous_work {
            assert!(work > previous);
            assert!(
                work <= previous * 4,
                "charged pipeline work must grow at most with the fourfold input"
            );
        }
        previous_work = Some(work);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        assert_eq!(scene.mesh_count(), 1);
        assert_eq!(scene.primitive_count(), count);
        assert_eq!(scene.attribute_count(), count);
        let mut index_addresses = HashSet::with_capacity(count);
        let mut position_addresses = HashSet::with_capacity(count);
        for (index, primitive) in scene.meshes().next().unwrap().primitives().enumerate() {
            assert_eq!(primitive.topology(), TopologyKind::TriangleList);
            assert_eq!(primitive.vertex_count(), 3);
            assert_eq!(primitive.corner_count(), 3);
            assert_eq!(primitive.face_count(), 1);
            let indices = primitive.indices().unwrap();
            assert_eq!(indices, [0, 1, 2]);
            assert!(
                index_addresses.insert(indices.as_ptr()),
                "each indexed primitive must own a distinct buffer"
            );
            let AttributeData::DenseF32 { lanes: 3, values } =
                primitive.positions().unwrap().data()
            else {
                panic!("expected dense POSITION");
            };
            let x = f32::from(u16::try_from(index).unwrap());
            assert_eq!(values, [x, 0.0, 0.0, x + 1.0, 0.0, 0.0, x, 1.0, 0.0]);
            assert!(position_addresses.insert(values.as_ptr()));
        }
        assert_eq!(index_addresses.len(), count);
        assert_eq!(position_addresses.len(), count);
        drop(scene);
        drop(parts.report);
        assert_account_released(&account);
    }
}

#[test]
#[ignore = "manual 8 MiB shared-payload node-transform allocation pilot"]
fn shared_eight_mib_points_payload_node_edit_scale() {
    // Three F32 coordinates and one U32 index per point: exactly 8 MiB total.
    const VERTICES: usize = 8 * MIB / (3 * size_of::<f32>() + size_of::<u32>());
    for count in [1_000_usize, 10_000] {
        let factory = test_factory();
        let account = test_account(&factory, u64::MAX);
        let roots: Vec<_> = (0..count)
            .map(|index| NodeLocalId::new(u32::try_from(index).unwrap()))
            .collect();
        let node_rows: Vec<_> = (0..count)
            .map(|_| {
                NodeAuthoringNode::new(
                    Presence::Absent,
                    AuthoredTransform::identity_trs(),
                    Presence::Absent,
                    Presence::Absent,
                )
            })
            .collect();
        let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &node_rows);
        let mut values = vec![0.0_f32; VERTICES * 3];
        values[(VERTICES - 1) * 3..].copy_from_slice(&[7.25, 8.5, 9.75]);
        let indices: Vec<_> = (0..u32::try_from(VERTICES).unwrap()).collect();
        assert_eq!(
            size_of_val(values.as_slice()) + size_of_val(indices.as_slice()),
            8 * MIB
        );
        let attributes = [GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &values,
            Presence::Absent,
        )];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Present(&indices),
            Presence::Absent,
            AuthoringRange::new(0, 1),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances: Vec<_> = roots
            .iter()
            .copied()
            .map(|node| GeometryMeshInstanceAuthoring::new(node, LogicalMeshLocalId::new(0)))
            .collect();
        let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances);
        let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
        let instance_deformations =
            vec![MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent); count];
        let deformation = DeformationAuthoringBatch::new(
            &[],
            &[],
            &logical_morphs,
            &primitive_deformations,
            &instance_deformations,
        );
        let (parts, allocations) = measured_scene(
            &account,
            nodes,
            geometry,
            deformation,
            MetadataAuthoringBatch::empty(),
        );
        record_measurement(
            &format!("shared-points/{count}/build"),
            parts.report.ledger_evidence(),
            allocations,
            &account,
        );
        let source = ValidatedScene::from_node_scene(parts.scene);
        let target = u32::try_from(count / 2).unwrap();
        let (terminal, allocations) = measured_edit(&source, target);
        let NodeSceneEditTerminal::Success(edited) = terminal else {
            panic!("scale node edit must publish");
        };
        record_measurement(
            &format!("shared-points/{count}/edit"),
            edited.report.ledger_evidence(),
            allocations,
            &account,
        );
        let destination = ValidatedScene::from_node_scene(edited.scene);
        assert_ne!(source.generation(), destination.generation());
        let source_primitive = source.meshes().next().unwrap().primitives().next().unwrap();
        let destination_primitive = destination
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(destination_primitive.topology(), TopologyKind::Points);
        assert_eq!(
            destination_primitive.vertex_count(),
            u32::try_from(VERTICES).unwrap()
        );
        assert!(
            std::ptr::eq(positions(&source), positions(&destination)),
            "editing a node must share the full position payload"
        );
        assert!(
            std::ptr::eq(
                source_primitive.indices().unwrap(),
                destination_primitive.indices().unwrap()
            ),
            "editing a node must share the full index payload"
        );
        for (index, point) in positions(&destination).chunks_exact(3).enumerate() {
            assert_eq!(
                point,
                if index + 1 == VERTICES {
                    &[7.25, 8.5, 9.75]
                } else {
                    &[0.0, 0.0, 0.0]
                }
            );
        }
        assert!(
            destination_primitive
                .indices()
                .unwrap()
                .iter()
                .copied()
                .eq(0..u32::try_from(VERTICES).unwrap())
        );
        for scene in [&source, &destination] {
            assert_eq!(scene.node_count(), count);
            assert_eq!(scene.root_count(), count);
            assert!(
                scene
                    .roots()
                    .map(|node| node.id().index())
                    .eq(0..u32::try_from(count).unwrap())
            );
            assert_eq!(scene.mesh_count(), 1);
            assert_eq!(scene.primitive_count(), 1);
            assert_eq!(scene.mesh_instance_count(), count);
            let shared_positions = positions(scene);
            for (index, instance) in scene.mesh_instances().enumerate() {
                let index = u32::try_from(index).unwrap();
                assert_eq!(instance.node().id().index(), index);
                assert_eq!(instance.mesh().id(), scene.meshes().next().unwrap().id());
                let primitive = instance.mesh().primitives().next().unwrap();
                let AttributeData::DenseF32 { values, .. } = primitive.positions().unwrap().data()
                else {
                    panic!("expected shared POSITION");
                };
                assert!(std::ptr::eq(values, shared_positions));
                let expected = if scene.generation() == destination.generation() && index == target
                {
                    trs_transform(42.0)
                } else {
                    AuthoredTransform::identity_trs()
                };
                assert!(
                    instance
                        .node()
                        .authored_transform()
                        .has_same_authored_bits(expected)
                );
            }
        }
        drop(source);
        drop(parts.report);
        // The edited publication must retain the shared payload after the source is destroyed.
        assert_eq!(positions(&destination).last(), Some(&9.75));
        drop(destination);
        drop(edited.report);
        assert_account_released(&account);
    }
}

#[test]
#[ignore = "manual 10k/100k four-influence and 1/8 MiB metadata allocation pilot"]
fn influence_and_metadata_validation_scale() {
    for vertices in [10_000_usize, 100_000] {
        for metadata_size in [MIB, 8 * MIB] {
            let factory = test_factory();
            let account = test_account(&factory, u64::MAX);
            let roots = [
                NodeLocalId::new(0),
                NodeLocalId::new(1),
                NodeLocalId::new(2),
                NodeLocalId::new(3),
            ];
            let node_rows: Vec<_> = roots
                .iter()
                .map(|_| {
                    NodeAuthoringNode::new(
                        Presence::Absent,
                        AuthoredTransform::identity_trs(),
                        Presence::Absent,
                        Presence::Absent,
                    )
                })
                .collect();
            let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &node_rows);
            let values = vec![0.0_f32; vertices * 3];
            let indices: Vec<_> = (0..u32::try_from(vertices).unwrap()).collect();
            let attributes = [GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &values,
                Presence::Absent,
            )];
            let primitives = [GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Points,
                Presence::Present(&indices),
                Presence::Absent,
                AuthoringRange::new(0, 1),
            )];
            let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
            let instances = [GeometryMeshInstanceAuthoring::new(
                NodeLocalId::new(0),
                LogicalMeshLocalId::new(0),
            )];
            let geometry =
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances);
            let inverse_binds = [Mat4::IDENTITY; 4];
            let skins = [SkinAuthoring::new(
                Presence::Absent,
                &roots,
                &inverse_binds,
                Presence::Absent,
                SkinMatrixSpaces::canonical(),
            )];
            let offsets: Vec<_> = (0..=u32::try_from(vertices).unwrap())
                .map(|vertex| vertex * 4)
                .collect();
            let influences: Vec<_> = (0..vertices)
                .flat_map(|_| {
                    [
                        InfluenceAuthoring::new(3, 0.125, 2),
                        InfluenceAuthoring::new(1, 0.25, 0),
                        InfluenceAuthoring::new(0, 0.5, 3),
                        InfluenceAuthoring::new(2, 0.125, 1),
                    ]
                })
                .collect();
            let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
            let primitive_deformations = [PrimitiveDeformationAuthoring::new(
                Presence::Present(InfluenceRowsAuthoring::new(&offsets, &influences)),
                &[],
            )];
            let instance_deformations = [MeshInstanceDeformationAuthoring::new(
                Presence::Present(SkinLocalId::new(0)),
                Presence::Absent,
            )];
            let deformation = DeformationAuthoringBatch::new(
                &skins,
                &[],
                &logical_morphs,
                &primitive_deformations,
                &instance_deformations,
            );
            let key = NamespacedName::new("scale:payload").unwrap();
            let media_type = ExtensionMediaType::new("application/octet-stream").unwrap();
            let payload: Vec<_> = (0..metadata_size)
                .map(|index| u8::try_from(index % 251).unwrap())
                .collect();
            let metadata = [MetadataAuthoringEntry::new(
                ProfileOwner::Scene,
                &key,
                ExtensionValueRef::Bytes {
                    media_type: &media_type,
                    payload: &payload,
                },
            )];
            let (parts, allocations) = measured_scene(
                &account,
                nodes,
                geometry,
                deformation,
                MetadataAuthoringBatch::new(&metadata),
            );
            record_measurement(
                &format!("influences/{vertices}/metadata/{metadata_size}/build"),
                parts.report.ledger_evidence(),
                allocations,
                &account,
            );
            // At fixture capture, the old validator copied all three U32 lanes into scratch:
            // (vertices + 1) row offsets, 4 * vertices joints, and 4 * vertices source orders.
            // These are hand-counted payload bytes, not measured allocation or RSS results.
            assert_eq!(
                (offsets.len() + 2 * influences.len()) * size_of::<u32>(),
                vertices * 36 + 4
            );
            eprintln!(
                "influences/{vertices}/metadata/{metadata_size}: logical_influence_u32_bytes={}",
                vertices * 36 + 4
            );
            let source = ValidatedScene::from_node_scene(parts.scene);
            let (terminal, allocations) = measured_edit(&source, 3);
            let NodeSceneEditTerminal::Success(edited) = terminal else {
                panic!("influence/metadata node edit must publish");
            };
            record_measurement(
                &format!("influences/{vertices}/metadata/{metadata_size}/edit"),
                edited.report.ledger_evidence(),
                allocations,
                &account,
            );
            let destination = ValidatedScene::from_node_scene(edited.scene);
            for scene in [&source, &destination] {
                let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
                assert_eq!(primitive.vertex_count(), u32::try_from(vertices).unwrap());
                let rows = primitive.influences().unwrap();
                assert_eq!(rows.len(), vertices);
                for (vertex, row) in rows.iter().enumerate() {
                    assert_eq!(row.vertex(), u32::try_from(vertex).unwrap());
                    assert_eq!(row.joint_indices(), [3, 1, 0, 2]);
                    assert_eq!(row.weights(), [0.125, 0.25, 0.5, 0.125]);
                    assert_eq!(row.source_orders(), [2, 0, 3, 1]);
                }
                assert!(rows.get(vertices).is_none());
                let skin = scene.mesh_instances().next().unwrap().skin().unwrap();
                assert_eq!(skin.joint_count(), 4);
                assert!(skin.joints().map(|joint| joint.index()).eq(0..4));
                assert!(
                    skin.inverse_bind_matrices()
                        .all(|matrix| matrix == Mat4::IDENTITY)
                );
                let bytes = metadata_bytes(scene);
                assert_eq!(bytes.len(), metadata_size);
                assert!(
                    bytes
                        .iter()
                        .enumerate()
                        .all(|(index, byte)| usize::from(*byte) == index % 251)
                );
            }
            let source_rows = source
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .influences()
                .unwrap();
            let destination_rows = destination
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .influences()
                .unwrap();
            for vertex in [0, vertices - 1] {
                let before = source_rows.get(vertex).unwrap();
                let after = destination_rows.get(vertex).unwrap();
                assert!(std::ptr::eq(before.joint_indices(), after.joint_indices()));
                assert!(std::ptr::eq(before.weights(), after.weights()));
                assert!(std::ptr::eq(before.source_orders(), after.source_orders()));
            }
            assert!(std::ptr::eq(
                metadata_bytes(&source),
                metadata_bytes(&destination)
            ));
            assert!(std::ptr::eq(positions(&source), positions(&destination)));
            assert!(
                source
                    .nodes()
                    .nth(3)
                    .unwrap()
                    .authored_transform()
                    .has_same_authored_bits(AuthoredTransform::identity_trs())
            );
            assert!(
                destination
                    .nodes()
                    .nth(3)
                    .unwrap()
                    .authored_transform()
                    .has_same_authored_bits(trs_transform(42.0))
            );
            drop(source);
            drop(parts.report);
            assert_eq!(metadata_bytes(&destination).len(), metadata_size);
            assert_eq!(
                destination_rows.get(vertices - 1).unwrap().weights(),
                [0.125, 0.25, 0.5, 0.125]
            );
            drop(destination);
            drop(edited.report);
            assert_account_released(&account);
        }
    }
}
