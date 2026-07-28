use baozi_operation::{
    CancellationToken, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};

use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneSuccessParts, ValidatedNodeScene,
};
use crate::canonical_scene::store::construction::{
    SceneConstructionLimits, SceneConstructionOperation,
};
use crate::canonical_scene::store::production::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LogicalMeshLocalId, MaterialLocalId, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, NodeParent, Presence, SceneAuthoringBatch,
};
use crate::entity::AuthoredTransform;
use crate::math::{Mat4, Quat, Vec3};

use super::{
    MAX_COLLECTION_ITEMS, MAX_PREVIEW_BYTES, MAX_WORK, NodeSceneSnapshot,
    NodeSceneSnapshotAlgorithm, NodeSceneSnapshotError, NodeSceneSnapshotLimit,
    NodeSceneSnapshotLimits, SnapshotWorkMeter, capture_node_scene_with_test_transcript,
};

const KNOWN_SINGLE_TRS_DIGEST: [u8; 32] = [
    0xab, 0xe2, 0xf5, 0x68, 0xa0, 0x49, 0x2f, 0x8c, 0xd1, 0x2b, 0x51, 0xff, 0x70, 0x2c, 0x0b, 0xce,
    0xd8, 0x10, 0x08, 0x94, 0xfe, 0x97, 0xdc, 0xfa, 0xaf, 0xdf, 0xc7, 0x45, 0x40, 0x37, 0x44, 0x42,
];
const KNOWN_SINGLE_TRS_TRANSCRIPT_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74",
    "0202000000030600000000000000736861323536040c01080d00000000000000000b010000000000",
    "0000050c01060d00000000000000000a020e000000000e000000000e000000000e000000000e0000",
    "00000e000000000e0000803f0e0000803f0e0000803f0e0000803f0b0a00000000000000070c010b",
    "00000000000000000b01000000000000000f0c01100c010b0000000000000000120c010b00000000",
    "00000000140c010b0000000000000000190c010b0000000000000000280c010b0000000000000000",
);
const KNOWN_MATRIX_CHILD_DIGEST: [u8; 32] = [
    0x80, 0x72, 0x57, 0xea, 0xf8, 0x2e, 0x83, 0x21, 0x22, 0xc2, 0x0d, 0x49, 0x2b, 0xc8, 0x2c, 0x4c,
    0x4c, 0x00, 0x9e, 0x69, 0x43, 0x93, 0x4e, 0xac, 0x89, 0xd8, 0xe9, 0xde, 0xd1, 0x78, 0x00, 0x0b,
];
const KNOWN_MATRIX_CHILD_TRANSCRIPT_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74020200000003",
    "0600000000000000736861323536040c01080d00000000000000000b0100000000000000050c01060d00000000",
    "000000000a010e0000803f0e000000000e000000000e000000000e000000000e0000803f0e000000000e000000",
    "000e000000000e000000000e0000803f0e000000000e000080400e000000000e000000000e0000803f0b100000",
    "0000000000070c01090d01000000000000000b0100000000000000060d01000000000000000a020e000000410e",
    "000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f0e0000803f0e0000803f0b",
    "0a00000000000000070c010b00000000000000000b02000000000000000f0c01100c010b000000000000000012",
    "0c010b0000000000000000140c010b0000000000000000190c010b0000000000000000280c010b000000000000",
    "0000",
);
const EMPTY_GEOMETRY_TRANSCRIPT_HEX: &str = concat!(
    "0f0c01100c010b0000000000000000120c010b0000000000000000140c010b000000000000000019",
    "0c010b0000000000000000280c010b0000000000000000",
);
const KNOWN_POINT_GEOMETRY_DIGEST: [u8; 32] = [
    0x78, 0x8d, 0x04, 0x88, 0x0d, 0x18, 0x7b, 0x56, 0x4a, 0x89, 0xd0, 0x90, 0xdf, 0xc4, 0x90, 0x32,
    0x55, 0xb9, 0xdb, 0xd2, 0xed, 0xdb, 0xe0, 0x87, 0xb6, 0x7f, 0xfc, 0x05, 0xc4, 0x72, 0x02, 0xd2,
];
const KNOWN_POINT_GEOMETRY_TRANSCRIPT_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74020200000003",
    "0600000000000000736861323536040c01080d00000000000000000b0100000000000000050c01060d00000000",
    "000000000a020e000000000e000000000e000000000e000000000e000000000e000000000e0000803f0e000080",
    "3f0e0000803f0e0000803f0b0a00000000000000070c010b00000000000000000b01000000000000000f0c0110",
    "0c01110d00000000000000000b0100000000000000120c01130d00000000000000002c00000000000000000b01",
    "000000000000000b0100000000000000140c01150d00000000000000002e002f01000000300100000031010000",
    "00160c000b0000000000000000170c000b0000000000000000180c010d0000000000000000192d000000000000",
    "00000b01000000000000000b0100000000000000190c011a0d00000000000000001b001c000000001d001e001f",
    "032000210022010000002301000000240300000000000000250c010e0000803f0e000000400e000040400b030000",
    "0000000000260c000b0000000000000000270c000b00000000000000000b0100000000000000280c01290d0000",
    "0000000000002a00000000000000002b00000000000000000b0100000000000000",
);

fn operation_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

fn snapshot_limits(bytes: usize, items: usize) -> NodeSceneSnapshotLimits {
    NodeSceneSnapshotLimits::try_new(bytes, items, MAX_WORK).expect("valid snapshot limits")
}

fn capture_node_scene(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<NodeSceneSnapshot, NodeSceneSnapshotError> {
    scene.capture_snapshot(limits, cancellation)
}

fn factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 16)
            .expect("snapshot test factory limits"),
    )
    .expect("snapshot test factory")
}

fn account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 512).expect("snapshot test account limits"),
        )
        .expect("snapshot test account")
}

fn build_scene(
    account: &PhysicalStorageAccount,
    roots: &[NodeLocalId],
    nodes: &[NodeAuthoringNode],
) -> NodeSceneSuccessParts {
    build_scene_with_geometry(account, roots, nodes, GeometryAuthoringBatch::empty())
}

fn build_scene_with_geometry<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    geometry: GeometryAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    let input = SceneAuthoringBatch::new(
        NodeAuthoringBatch::new(Presence::Present(roots), nodes),
        geometry,
    );
    let prepared = SceneConstructionOperation::with_account_for_scene_input(
        account,
        operation_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("snapshot Scene input admission")
    .try_validate()
    .unwrap_or_else(|_| panic!("snapshot input must pass authoring validation"))
    .prepare()
    .expect("snapshot Scene must prepare");
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("valid snapshot Scene published rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("valid snapshot Scene published minimal fallback")
        }
    }
}

fn parent(node: u32, ordinal: u32) -> Presence<NodeParent> {
    Presence::Present(NodeParent::new(NodeLocalId::new(node), ordinal))
}

fn trs(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(translation_x, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("valid authored TRS")
}

fn matrix(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [translation_x, 0.0, 0.0, 1.0],
        ],
    })
    .expect("valid authored matrix")
}

fn assert_hex(actual: &[u8], expected: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    assert_eq!(actual.len().checked_mul(2), Some(expected.len()));
    for (index, byte) in actual.iter().copied().enumerate() {
        assert_eq!(expected.as_bytes()[index * 2], HEX[usize::from(byte >> 4)]);
        assert_eq!(
            expected.as_bytes()[index * 2 + 1],
            HEX[usize::from(byte & 0x0f)]
        );
    }
}

fn single_root_scene(
    account: &PhysicalStorageAccount,
    transform: AuthoredTransform,
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(Presence::Absent, transform)];
    build_scene(account, &roots, &nodes)
}

fn point_geometry_scene(
    account: &PhysicalStorageAccount,
    positions: &[f32],
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Present(MaterialLocalId::new(0)),
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(1, &logical_meshes, &primitives, &attributes, &instances),
    )
}

fn point_index_presence_scene(
    account: &PhysicalStorageAccount,
    indices_present: bool,
) -> NodeSceneSuccessParts {
    let roots = [];
    let nodes = [];
    let positions = [0.0_f32, 0.0, 0.0];
    let empty_indices = [];
    let indices: Presence<&[u32]> = if indices_present {
        Presence::Present(&empty_indices)
    } else {
        Presence::Absent
    };
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        indices,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &attributes, &[]),
    )
}

fn corner_normal_scene<'input>(
    account: &PhysicalStorageAccount,
    normal_values: &'input [f32],
    normal_source_indices: Presence<&'input [u32]>,
) -> NodeSceneSuccessParts {
    let roots = [];
    let nodes = [];
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            normal_values,
            normal_source_indices,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &attributes, &[]),
    )
}

fn all_attribute_payload_kinds_scene(account: &PhysicalStorageAccount) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
    )];
    let triangle_indices = [0_u32, 1, 2];
    let polygon_boundaries = [3_u32];
    let first_positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let second_positions = [0.0_f32, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let third_positions = [0.0_f32, 0.0, 2.0, 1.0, 0.0, 2.0, 0.0, 1.0, 2.0];
    let sparse_indices = [0_u32];
    let sparse_u32_values = [7_u32];
    let sparse_u32_default = [0_u32];
    let sparse_f32_values = [0.25_f32, 0.5, 0.75];
    let sparse_f32_default = [0.0_f32, 0.0, 0.0];
    let dense_u32_values = [3_u32];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &first_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            1,
            &sparse_indices,
            &sparse_u32_values,
            &sparse_u32_default,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &second_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Face,
            3,
            1,
            &sparse_indices,
            &sparse_f32_values,
            &sparse_f32_default,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &third_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            &dense_u32_values,
        ),
    ];
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&triangle_indices),
            Presence::Present(MaterialLocalId::new(0)),
            AuthoringRange::new(0, 2),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Polygons(&polygon_boundaries),
            Presence::Present(&triangle_indices),
            Presence::Absent,
            AuthoringRange::new(2, 2),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(4, 2),
        ),
    ];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 3))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(1, &logical_meshes, &primitives, &attributes, &instances),
    )
}

#[test]
fn known_vector_is_stable_and_human_reviewable() {
    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(4_096, 64), None)
            .expect("known Scene snapshot");

    assert_eq!(snapshot.domain, "baozi.canonical-scene.snapshot");
    assert_eq!(snapshot.schema_revision, 2);
    assert_eq!(snapshot.algorithm, NodeSceneSnapshotAlgorithm::Sha256);
    assert_eq!(snapshot.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert!(transcript.complete);
    assert_hex(&transcript.bytes, KNOWN_SINGLE_TRS_TRANSCRIPT_HEX);
    assert_eq!(snapshot.totals.node_count, 1);
    assert_eq!(snapshot.totals.root_count, 1);
    assert_eq!(snapshot.totals.edge_count, 0);
    assert_eq!(snapshot.totals.material_count, 0);
    assert_eq!(snapshot.totals.logical_mesh_count, 0);
    assert_eq!(snapshot.totals.primitive_count, 0);
    assert_eq!(snapshot.totals.attribute_count, 0);
    assert_eq!(snapshot.totals.mesh_instance_count, 0);
    assert_eq!(snapshot.totals.transcript_bytes, 239);
    assert_eq!(snapshot.totals.work_units, 27);
    assert!(snapshot.preview.text.contains("roots(present,total=1)=[0]"));
    assert!(snapshot.preview.text.contains("transform=trs(total=10)"));
    assert!(
        snapshot
            .preview
            .text
            .contains("children(present)=[];total=0")
    );
    assert!(snapshot.preview.text.contains("geometry(present)"));
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn matrix_and_child_known_vector_pins_node_framing_and_empty_geometry() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(Presence::Absent, matrix(4.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(8.0)),
    ];
    let parts = build_scene(&account, &roots, &nodes);

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(8_192, 64), None)
            .expect("matrix-child known Scene snapshot");

    assert_eq!(snapshot.digest, KNOWN_MATRIX_CHILD_DIGEST);
    assert!(transcript.complete);
    assert_hex(&transcript.bytes, KNOWN_MATRIX_CHILD_TRANSCRIPT_HEX);
    assert_eq!(snapshot.totals.node_count, 2);
    assert_eq!(snapshot.totals.root_count, 1);
    assert_eq!(snapshot.totals.edge_count, 1);
    assert_eq!(snapshot.totals.transcript_bytes, 362);
    assert_eq!(snapshot.totals.work_units, 47);
}

#[test]
fn point_geometry_known_vector_pins_complete_scene_framing() {
    let factory = factory();
    let account = account(&factory);
    let positions = [1.0_f32, 2.0, 3.0];
    let parts = point_geometry_scene(&account, &positions);

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(8_192, 64), None)
            .expect("point-geometry known Scene snapshot");

    assert_eq!(snapshot.digest, KNOWN_POINT_GEOMETRY_DIGEST);
    assert!(transcript.complete);
    assert_hex(&transcript.bytes, KNOWN_POINT_GEOMETRY_TRANSCRIPT_HEX);
    assert_eq!(snapshot.totals.node_count, 1);
    assert_eq!(snapshot.totals.material_count, 1);
    assert_eq!(snapshot.totals.logical_mesh_count, 1);
    assert_eq!(snapshot.totals.primitive_count, 1);
    assert_eq!(snapshot.totals.attribute_count, 1);
    assert_eq!(snapshot.totals.mesh_instance_count, 1);
    assert_eq!(snapshot.totals.polygon_boundary_count, 0);
    assert_eq!(snapshot.totals.primitive_index_count, 0);
    assert_eq!(snapshot.totals.sparse_index_count, 0);
    assert_eq!(snapshot.totals.canonical_scalar_count, 3);
    assert_eq!(snapshot.totals.f32_scalar_count, 3);
    assert_eq!(snapshot.totals.u32_scalar_count, 0);
    assert_eq!(snapshot.totals.transcript_bytes, 484);
    assert_eq!(snapshot.totals.work_units, 40);
    assert!(snapshot.preview.text.contains("material[0]"));
    assert!(
        snapshot
            .preview
            .text
            .contains("primitive[0] topology=points")
    );
    assert!(snapshot.preview.text.contains("values(present,total=3)"));
    assert!(
        snapshot
            .preview
            .text
            .contains("mesh_instance[0] owner_node=0")
    );
}

#[test]
fn collection_framing_distinguishes_root_child_partitions() {
    let factory = factory();
    let account = account(&factory);
    let first_roots = [NodeLocalId::new(0)];
    let first_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0)),
        NodeAuthoringNode::new(parent(0, 1), trs(2.0)),
        NodeAuthoringNode::new(parent(1, 0), trs(3.0)),
    ];
    let second_roots = [NodeLocalId::new(0)];
    let second_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0)),
        NodeAuthoringNode::new(parent(1, 0), trs(2.0)),
        NodeAuthoringNode::new(parent(1, 1), trs(3.0)),
    ];
    let first = build_scene(&account, &first_roots, &first_nodes);
    let second = build_scene(&account, &second_roots, &second_nodes);
    let limits = snapshot_limits(16_384, 64);

    let first_snapshot =
        capture_node_scene(&first.scene, limits, None).expect("first partition snapshot");
    let second_snapshot =
        capture_node_scene(&second.scene, limits, None).expect("second partition snapshot");

    assert_eq!(first_snapshot.totals.node_count, 4);
    assert_eq!(first_snapshot.totals.root_count, 1);
    assert_eq!(first_snapshot.totals.edge_count, 3);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_ne!(first_snapshot.digest, second_snapshot.digest);
}

#[test]
fn empty_scene_snapshot_is_complete_and_deterministic() {
    let factory = factory();
    let first_account = account(&factory);
    let second_account = account(&factory);
    let first = build_scene(&first_account, &[], &[]);
    let second = build_scene(&second_account, &[], &[]);
    let limits = snapshot_limits(4_096, 64);

    let (first_snapshot, transcript) =
        capture_node_scene_with_test_transcript(&first.scene, limits, None)
            .expect("first empty snapshot");
    let second_snapshot =
        capture_node_scene(&second.scene, limits, None).expect("second empty snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview, second_snapshot.preview);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_eq!(first_snapshot.totals.node_count, 0);
    assert_eq!(first_snapshot.totals.root_count, 0);
    assert_eq!(first_snapshot.totals.edge_count, 0);
    assert_eq!(first_snapshot.totals.material_count, 0);
    assert_eq!(first_snapshot.totals.logical_mesh_count, 0);
    assert_eq!(first_snapshot.totals.primitive_count, 0);
    assert_eq!(first_snapshot.totals.attribute_count, 0);
    assert_eq!(first_snapshot.totals.mesh_instance_count, 0);
    assert_eq!(first_snapshot.totals.polygon_boundary_count, 0);
    assert_eq!(first_snapshot.totals.primitive_index_count, 0);
    assert_eq!(first_snapshot.totals.sparse_index_count, 0);
    assert_eq!(first_snapshot.totals.canonical_scalar_count, 0);
    assert_eq!(first_snapshot.totals.transcript_bytes, 146);
    assert_eq!(first_snapshot.totals.work_units, 13);
    let empty_tail_bytes = EMPTY_GEOMETRY_TRANSCRIPT_HEX.len() / 2;
    assert!(transcript.complete);
    assert_hex(
        &transcript.bytes[transcript.bytes.len() - empty_tail_bytes..],
        EMPTY_GEOMETRY_TRANSCRIPT_HEX,
    );
}

#[test]
fn geometry_tables_payload_kinds_and_totals_are_complete() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);

    let snapshot = capture_node_scene(&parts.scene, snapshot_limits(32_768, 64), None)
        .expect("complete geometry snapshot");

    assert_eq!(snapshot.totals.material_count, 1);
    assert_eq!(snapshot.totals.logical_mesh_count, 1);
    assert_eq!(snapshot.totals.primitive_count, 3);
    assert_eq!(snapshot.totals.attribute_count, 6);
    assert_eq!(snapshot.totals.mesh_instance_count, 1);
    assert_eq!(snapshot.totals.polygon_boundary_count, 1);
    assert_eq!(snapshot.totals.primitive_index_count, 6);
    assert_eq!(snapshot.totals.sparse_index_count, 2);
    assert_eq!(snapshot.totals.canonical_scalar_count, 36);
    assert_eq!(snapshot.totals.f32_scalar_count, 33);
    assert_eq!(snapshot.totals.u32_scalar_count, 3);
    assert_eq!(snapshot.totals.work_units, 108);
    assert!(snapshot.preview.text.contains("payload=dense"));
    assert!(snapshot.preview.text.contains("payload=sparse"));
    assert!(snapshot.preview.text.contains("sparse_default(present"));
    assert!(snapshot.preview.text.contains("polygon_boundaries(present"));
    assert!(
        snapshot
            .preview
            .text
            .contains("summary(nodes=1,roots=1,edges=0")
    );
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);
}

#[test]
fn absent_and_present_empty_geometry_leaves_remain_distinct() {
    let factory = factory();
    let account = account(&factory);
    let absent = point_index_presence_scene(&account, false);
    let present_empty = point_index_presence_scene(&account, true);
    let limits = snapshot_limits(16_384, 64);

    let absent_snapshot =
        capture_node_scene(&absent.scene, limits, None).expect("absent-index snapshot");
    let present_empty_snapshot = capture_node_scene(&present_empty.scene, limits, None)
        .expect("present-empty-index snapshot");

    assert_ne!(absent_snapshot.digest, present_empty_snapshot.digest);
    assert_eq!(absent_snapshot.totals, present_empty_snapshot.totals);
    assert!(
        absent_snapshot
            .preview
            .text
            .contains("indices(absent,total=0)=[]")
    );
    assert!(
        present_empty_snapshot
            .preview
            .text
            .contains("indices(present,total=0)=[]")
    );
}

#[test]
fn projected_and_expanded_attributes_have_the_same_canonical_snapshot() {
    let factory = factory();
    let account = account(&factory);
    let projected_values = [0.0_f32, 0.0, 1.0, 0.0, 1.0, 0.0];
    let projection = [0_u32, 1, 0];
    let expanded_values = [0.0_f32, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let projected =
        corner_normal_scene(&account, &projected_values, Presence::Present(&projection));
    let expanded = corner_normal_scene(&account, &expanded_values, Presence::Absent);
    let limits = snapshot_limits(16_384, 64);

    let projected_snapshot =
        capture_node_scene(&projected.scene, limits, None).expect("projected snapshot");
    let expanded_snapshot =
        capture_node_scene(&expanded.scene, limits, None).expect("expanded snapshot");

    assert_eq!(projected_snapshot.digest, expanded_snapshot.digest);
    assert_eq!(projected_snapshot.preview, expanded_snapshot.preview);
    assert_eq!(projected_snapshot.totals, expanded_snapshot.totals);
    assert_eq!(projected_snapshot.totals.canonical_scalar_count, 18);
    assert_eq!(projected_snapshot.totals.f32_scalar_count, 18);
}

#[test]
fn geometry_mutation_after_preview_boundary_changes_complete_digest() {
    let factory = factory();
    let account = account(&factory);
    let left_positions = [1.0_f32, 2.0, 3.0];
    let right_positions = [1.0_f32, 2.0, 4.0];
    let left = point_geometry_scene(&account, &left_positions);
    let right = point_geometry_scene(&account, &right_positions);
    let limits = snapshot_limits(8_192, 1);

    let left_snapshot =
        capture_node_scene(&left.scene, limits, None).expect("left geometry snapshot");
    let right_snapshot =
        capture_node_scene(&right.scene, limits, None).expect("right geometry snapshot");

    assert_eq!(left_snapshot.preview, right_snapshot.preview);
    assert!(left_snapshot.preview.items_truncated);
    assert_ne!(left_snapshot.digest, right_snapshot.digest);
    assert_eq!(left_snapshot.totals, right_snapshot.totals);
}

#[test]
fn geometry_capture_honors_cancellation_and_exact_work_bounds() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);
    let retained = account.snapshot();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    assert!(matches!(
        capture_node_scene(
            &parts.scene,
            snapshot_limits(32_768, 64),
            Some(&cancellation)
        ),
        Err(NodeSceneSnapshotError::Cancelled)
    ));

    let full = capture_node_scene(&parts.scene, snapshot_limits(32_768, 64), None)
        .expect("full geometry work measurement");
    let exact = NodeSceneSnapshotLimits::try_new(32_768, 64, full.totals.work_units)
        .expect("exact geometry work limit");
    assert!(capture_node_scene(&parts.scene, exact, None).is_ok());
    let one_short = NodeSceneSnapshotLimits::try_new(
        32_768,
        64,
        full.totals
            .work_units
            .checked_sub(1)
            .expect("nonzero geometry snapshot work"),
    )
    .expect("one-short geometry work limit");
    assert!(matches!(
        capture_node_scene(&parts.scene, one_short, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
    assert_eq!(account.snapshot(), retained);
}

#[test]
fn geometry_collection_fault_fails_before_a_snapshot_can_be_published() {
    let factory = factory();
    let account = account(&factory);
    let positions = [1.0_f32, 2.0, 3.0];
    let parts = point_geometry_scene(&account, &positions);
    let retained = account.snapshot();
    let source = parts.scene.semantic_view().expect("point semantic view");
    let mut nodes = source.into_cursor().expect("point semantic cursor");
    let mut roots = nodes.take_roots().expect("point roots");
    assert_eq!(roots.next().expect("point root"), Some(0));
    roots.finish().expect("complete point roots");
    let node = nodes
        .next_node()
        .expect("point node read")
        .expect("point node");
    node.into_children()
        .finish()
        .expect("complete point children");
    let mut geometry = nodes.into_geometry().expect("point geometry cursor");
    let wrong_counts = super::GeometryCollectionCounts {
        materials: 2,
        logical_meshes: 1,
        primitives: 1,
        attributes: 1,
        mesh_instances: 1,
    };
    let mut transcript = super::CanonicalDigestEncoder::new();
    let mut preview = super::PreviewSink::new(4_096, 64).expect("fault preview");
    let mut work = SnapshotWorkMeter::new(MAX_WORK, None);

    let failure = super::write_geometry(
        &mut geometry,
        wrong_counts,
        &mut transcript,
        &mut preview,
        &mut work,
    );
    assert!(matches!(
        failure,
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "material collection ended before declared total"
        })
    ));
    assert_eq!(account.snapshot(), retained);
}

#[test]
fn mutation_after_preview_boundary_changes_only_the_complete_digest() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let left_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(1.0)),
        NodeAuthoringNode::new(Presence::Absent, trs(2.0)),
    ];
    let right_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(1.0)),
        NodeAuthoringNode::new(Presence::Absent, trs(3.0)),
    ];
    let left = build_scene(&account, &roots, &left_nodes);
    let right = build_scene(&account, &roots, &right_nodes);
    let limits = snapshot_limits(4_096, 1);

    let left_snapshot =
        capture_node_scene(&left.scene, limits, None).expect("left bounded snapshot");
    let right_snapshot =
        capture_node_scene(&right.scene, limits, None).expect("right bounded snapshot");

    assert_eq!(left_snapshot.preview.text, right_snapshot.preview.text);
    assert_eq!(
        left_snapshot.preview.items_omitted_by_limit,
        right_snapshot.preview.items_omitted_by_limit
    );
    assert!(left_snapshot.preview.items_truncated);
    assert_ne!(left_snapshot.digest, right_snapshot.digest);
    assert_eq!(left_snapshot.totals, right_snapshot.totals);
}

#[test]
fn equivalent_semantics_ignore_local_indices_account_and_generation() {
    let factory = factory();
    let first_account = account(&factory);
    let second_account = account(&factory);

    let first_roots = [NodeLocalId::new(0)];
    let first_nodes = [
        NodeAuthoringNode::new(Presence::Absent, matrix(4.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(8.0)),
    ];
    let second_roots = [NodeLocalId::new(1)];
    let second_nodes = [
        NodeAuthoringNode::new(parent(1, 0), trs(8.0)),
        NodeAuthoringNode::new(Presence::Absent, matrix(4.0)),
    ];
    let first = build_scene(&first_account, &first_roots, &first_nodes);
    let second = build_scene(&second_account, &second_roots, &second_nodes);

    assert!(
        !first
            .scene
            .account()
            .is_same_account(second.scene.account())
    );
    assert_ne!(first.scene.generation(), second.scene.generation());
    let first_snapshot = capture_node_scene(&first.scene, snapshot_limits(8_192, 64), None)
        .expect("first equivalent snapshot");
    let second_snapshot = capture_node_scene(&second.scene, snapshot_limits(8_192, 64), None)
        .expect("second equivalent snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview, second_snapshot.preview);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
}

#[test]
fn sibling_order_representation_and_signed_zero_are_semantic() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let ordered_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0)),
        NodeAuthoringNode::new(parent(0, 1), trs(2.0)),
    ];
    let swapped_nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 1), trs(1.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(2.0)),
    ];
    let ordered = build_scene(&account, &roots, &ordered_nodes);
    let swapped = build_scene(&account, &roots, &swapped_nodes);
    let positive_zero = single_root_scene(&account, trs(0.0));
    let negative_zero = single_root_scene(&account, trs(-0.0));
    let matrix_identity = single_root_scene(&account, matrix(0.0));
    let trs_identity = single_root_scene(&account, AuthoredTransform::identity_trs());
    let limits = snapshot_limits(16_384, 64);

    let digest = |parts: &NodeSceneSuccessParts| {
        capture_node_scene(&parts.scene, limits, None)
            .expect("semantic discriminator snapshot")
            .digest
    };
    assert_ne!(digest(&ordered), digest(&swapped));
    assert_ne!(digest(&positive_zero), digest(&negative_zero));
    assert_ne!(digest(&matrix_identity), digest(&trs_identity));
}

#[test]
fn proof_and_mid_traversal_read_failures_do_not_publish_partial_snapshots() {
    let factory = factory();
    let account = account(&factory);

    let mut layout_mismatch = single_root_scene(&account, AuthoredTransform::identity_trs());
    assert!(
        layout_mismatch
            .scene
            .install_semantic_layout_fault_for_test(2, 2)
    );
    let donor = single_root_scene(&account, trs(1.0));
    let mut foreign_proof = single_root_scene(&account, trs(2.0));
    foreign_proof
        .scene
        .install_foreign_semantic_proof_for_test(&donor.scene);

    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0)),
    ];
    let mut child_read_failure = build_scene(&account, &roots, &nodes);
    child_read_failure
        .scene
        .install_child_reference_read_fault_for_test(1);
    let mut transform_read_failure = build_scene(&account, &roots, &nodes);
    transform_read_failure
        .scene
        .install_authored_transform_read_fault_for_test(1);
    let retained = account.snapshot();

    assert!(matches!(
        capture_node_scene(&layout_mismatch.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table row count disagrees with validation proof"
        })
    ));
    assert!(matches!(
        capture_node_scene(&foreign_proof.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table is unavailable or mistyped"
        })
    ));
    assert!(matches!(
        capture_node_scene(&child_read_failure.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "injected child-reference read failure"
        })
    ));
    assert!(matches!(
        capture_node_scene(
            &transform_read_failure.scene,
            snapshot_limits(4_096, 64),
            None
        ),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "injected authored-transform read failure"
        })
    ));

    assert_eq!(account.snapshot(), retained);
}

#[test]
fn limits_cancellation_and_capture_leave_physical_state_unchanged() {
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(MAX_PREVIEW_BYTES + 1, 1, 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::PreviewBytes,
            ..
        })
    ));
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(1, MAX_COLLECTION_ITEMS + 1, 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::CollectionItems,
            ..
        })
    ));
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(1, 1, MAX_WORK + 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::Work,
            ..
        })
    ));

    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());
    let retained = account.snapshot();

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        capture_node_scene(&parts.scene, snapshot_limits(256, 4), Some(&cancellation)),
        Err(NodeSceneSnapshotError::Cancelled)
    ));
    let insufficient_work =
        NodeSceneSnapshotLimits::try_new(256, 4, 1).expect("bounded work configuration");
    assert!(matches!(
        capture_node_scene(&parts.scene, insufficient_work, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
    let mut zero_work_result = None;
    let zero_work_allocations = allocation_counter::measure(|| {
        zero_work_result = Some(capture_node_scene(
            &parts.scene,
            NodeSceneSnapshotLimits::try_new(1, 1, 0).expect("zero work configuration"),
            None,
        ));
    });
    assert!(matches!(
        zero_work_result,
        Some(Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. }))
    ));
    assert_eq!(zero_work_allocations.count_total, 0);
    assert_eq!(zero_work_allocations.bytes_total, 0);

    for byte_limit in 1..=7 {
        let tiny = capture_node_scene(&parts.scene, snapshot_limits(byte_limit, 64), None)
            .expect("tiny preview limit remains a valid output bound");
        assert!(tiny.preview.text.len() <= byte_limit);
        assert!(tiny.preview.bytes_truncated);
        assert!(tiny.preview.text.capacity() >= byte_limit);
        assert!(tiny.preview.text.capacity() <= byte_limit.next_power_of_two());
    }
    let zero_preview = capture_node_scene(
        &parts.scene,
        NodeSceneSnapshotLimits::try_new(0, 0, MAX_WORK).expect("zero preview bounds"),
        None,
    )
    .expect("zero preview still captures complete digest");
    assert_eq!(zero_preview.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert!(zero_preview.preview.text.is_empty());
    assert!(zero_preview.preview.bytes_truncated);
    assert!(zero_preview.preview.items_truncated);
    assert_eq!(account.snapshot(), retained);

    let full =
        capture_node_scene(&parts.scene, snapshot_limits(4_096, 64), None).expect("full snapshot");
    assert_eq!(full.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert_eq!(account.snapshot(), retained);

    let exact_work = NodeSceneSnapshotLimits::try_new(4_096, 64, full.totals.work_units)
        .expect("exact work configuration");
    assert!(capture_node_scene(&parts.scene, exact_work, None).is_ok());
    let one_short = NodeSceneSnapshotLimits::try_new(
        4_096,
        64,
        full.totals
            .work_units
            .checked_sub(1)
            .expect("nonzero snapshot work"),
    )
    .expect("one-short work configuration");
    assert!(matches!(
        capture_node_scene(&parts.scene, one_short, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
}

#[test]
fn work_meter_observes_cancellation_at_the_bounded_poll_interval() {
    let cancellation = CancellationToken::new();
    let mut work = SnapshotWorkMeter::new(MAX_WORK, Some(&cancellation));
    for _ in 1..super::NODE_SCENE_SNAPSHOT_POLL_INTERVAL {
        work.observe().expect("work before the poll boundary");
    }
    cancellation.cancel();
    assert!(matches!(
        work.observe(),
        Err(NodeSceneSnapshotError::Cancelled)
    ));
}

#[test]
fn traversal_scales_linearly_while_preview_stays_bounded() {
    let factory = factory();
    let account = account(&factory);
    let mut observed = Vec::new();

    for node_count in [1_usize, 10, 100, 1_000] {
        let roots = [NodeLocalId::new(0)];
        let nodes = (0..node_count)
            .map(|index| {
                let parent_ref = if index == 0 {
                    Presence::Absent
                } else {
                    parent(0, u32::try_from(index - 1).expect("test child ordinal"))
                };
                NodeAuthoringNode::new(parent_ref, AuthoredTransform::identity_trs())
            })
            .collect::<Vec<_>>();
        let parts = build_scene(&account, &roots, &nodes);
        let mut captured = None;
        let allocations = allocation_counter::measure(|| {
            captured = Some(capture_node_scene(
                &parts.scene,
                snapshot_limits(64, 1),
                None,
            ));
        });
        let snapshot = captured
            .expect("scale capture result")
            .expect("scale snapshot");

        assert!(snapshot.preview.text.len() <= 64);
        assert_eq!(
            snapshot.totals.transcript_bytes,
            146 + 93 * node_count as u64
        );
        assert_eq!(snapshot.totals.work_units, 13 + 14 * node_count as u64);
        assert_eq!(snapshot.totals.node_count, node_count);
        assert_eq!(snapshot.totals.root_count, 1);
        assert_eq!(snapshot.totals.edge_count, node_count - 1);
        observed.push((
            snapshot.totals.transcript_bytes,
            snapshot.totals.work_units,
            allocations.count_total,
            allocations.bytes_total,
        ));
    }

    assert_eq!(observed[1].0 - observed[0].0, 9 * 93);
    assert_eq!(observed[2].0 - observed[0].0, 99 * 93);
    assert_eq!(observed[1].1 - observed[0].1, 9 * 14);
    assert_eq!(observed[2].1 - observed[0].1, 99 * 14);
    assert_eq!(observed[3].0 - observed[0].0, 999 * 93);
    assert_eq!(observed[3].1 - observed[0].1, 999 * 14);
    assert_eq!(observed[1].2, observed[0].2);
    assert_eq!(observed[2].2, observed[0].2);
    assert_eq!(observed[3].2, observed[0].2);
    assert_eq!(observed[1].3, observed[0].3);
    assert_eq!(observed[2].3, observed[0].3);
    assert_eq!(observed[3].3, observed[0].3);
}
