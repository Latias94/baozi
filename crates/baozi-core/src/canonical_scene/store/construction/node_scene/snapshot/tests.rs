use baozi_operation::{
    CancellationToken, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};

use crate::SceneValidationLimits;
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneSuccessParts, ValidatedNodeScene,
};
use crate::canonical_scene::store::construction::{
    SceneConstructionLimits, SceneConstructionOperation,
};
use crate::canonical_scene::store::production::{
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, Presence,
};
use crate::entity::AuthoredTransform;
use crate::math::{Mat4, Quat, Vec3};

use super::{
    MAX_COLLECTION_ITEMS, MAX_PREVIEW_BYTES, MAX_WORK, NodeSceneSnapshot,
    NodeSceneSnapshotAlgorithm, NodeSceneSnapshotError, NodeSceneSnapshotLimit,
    NodeSceneSnapshotLimits, SnapshotWorkMeter, capture_node_scene_with_test_transcript,
};

const KNOWN_SINGLE_TRS_DIGEST: [u8; 32] = [
    0x11, 0x09, 0x82, 0xe7, 0xe5, 0xa1, 0xd0, 0xc9, 0x3b, 0xa9, 0xc2, 0x5b, 0xab, 0xc6, 0x4e, 0x1e,
    0xcf, 0x4f, 0xe7, 0xed, 0x67, 0xaf, 0x6a, 0xe8, 0x29, 0x1c, 0x9a, 0x97, 0x0c, 0x8c, 0x33, 0x39,
];
const KNOWN_SINGLE_TRS_TRANSCRIPT_HEX: &str = concat!(
    "01230000000000000062616f7a692e63616e6f6e6963616c2d6e6f64652d7363656e652e736e",
    "617073686f740201000000030600000000000000736861323536040c01080d00000000000000000b",
    "0100000000000000050c01060d00000000000000000a020e000000000e000000000e000000000e",
    "000000000e000000000e000000000e0000803f0e0000803f0e0000803f0e0000803f0b0a000000",
    "00000000070c010b00000000000000000b0100000000000000",
);
const KNOWN_MATRIX_CHILD_DIGEST: [u8; 32] = [
    0xa6, 0xd5, 0xf9, 0xab, 0xb1, 0xcc, 0xdb, 0xcd, 0xdc, 0xae, 0x5b, 0xc8, 0x1b, 0x41, 0x7f, 0xd3,
    0x17, 0x2e, 0x93, 0xa4, 0x1f, 0x7e, 0x54, 0x36, 0x81, 0xa3, 0x47, 0x0e, 0x12, 0x3f, 0x3c, 0x8e,
];
const KNOWN_MATRIX_CHILD_TRANSCRIPT_HEX: &str = concat!(
    "01230000000000000062616f7a692e63616e6f6e6963616c2d6e6f64652d7363656e652e736e617073686f74",
    "0201000000030600000000000000736861323536040c01080d00000000000000000b0100000000000000050c",
    "01060d00000000000000000a010e0000803f0e000000000e000000000e000000000e000000000e0000803f0e",
    "000000000e000000000e000000000e000000000e0000803f0e000000000e000080400e000000000e00000000",
    "0e0000803f0b1000000000000000070c01090d01000000000000000b0100000000000000060d010000000000",
    "00000a020e000000410e000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f",
    "0e0000803f0e0000803f0b0a00000000000000070c010b00000000000000000b0200000000000000",
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
    let prepared = SceneConstructionOperation::with_account(account, operation_limits())
        .expect("snapshot Scene operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(roots), nodes),
            SceneValidationLimits::default(),
        )
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

#[test]
fn known_vector_is_stable_and_human_reviewable() {
    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(4_096, 64), None)
            .expect("known Scene snapshot");

    assert_eq!(snapshot.domain, "baozi.canonical-node-scene.snapshot");
    assert_eq!(snapshot.schema_revision, 1);
    assert_eq!(snapshot.algorithm, NodeSceneSnapshotAlgorithm::Sha256);
    assert_eq!(snapshot.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert!(transcript.complete);
    assert_hex(&transcript.bytes, KNOWN_SINGLE_TRS_TRANSCRIPT_HEX);
    assert_eq!(snapshot.totals.node_count, 1);
    assert_eq!(snapshot.totals.root_count, 1);
    assert_eq!(snapshot.totals.edge_count, 0);
    assert_eq!(snapshot.totals.transcript_bytes, 181);
    assert_eq!(snapshot.totals.work_units, 21);
    assert!(snapshot.preview.text.contains("roots(present,total=1)=[0]"));
    assert!(snapshot.preview.text.contains("transform=trs(total=10)"));
    assert!(
        snapshot
            .preview
            .text
            .contains("children(present)=[];total=0")
    );
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn matrix_and_child_known_vector_pins_remaining_v1_framing() {
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
    assert_eq!(snapshot.totals.transcript_bytes, 304);
    assert_eq!(snapshot.totals.work_units, 41);
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

    let first_snapshot =
        capture_node_scene(&first.scene, limits, None).expect("first empty snapshot");
    let second_snapshot =
        capture_node_scene(&second.scene, limits, None).expect("second empty snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview, second_snapshot.preview);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_eq!(first_snapshot.totals.node_count, 0);
    assert_eq!(first_snapshot.totals.root_count, 0);
    assert_eq!(first_snapshot.totals.edge_count, 0);
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
fn proof_qualified_read_failures_do_not_publish_partial_snapshots() {
    let factory = factory();
    let account = account(&factory);

    let mut missing_reference = single_root_scene(&account, AuthoredTransform::identity_trs());
    assert!(
        missing_reference
            .scene
            .install_semantic_layout_fault_for_test(2, 2)
    );

    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(Presence::Absent, trs(0.0)),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0)),
    ];
    let mut missing_node = build_scene(&account, &roots, &nodes);
    assert!(
        missing_node
            .scene
            .install_semantic_layout_fault_for_test(3, 1)
    );
    let mut truncated_proof = build_scene(&account, &roots, &nodes);
    assert!(
        truncated_proof
            .scene
            .install_semantic_layout_fault_for_test(1, 1)
    );
    let retained = account.snapshot();

    assert!(matches!(
        capture_node_scene(&missing_reference.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table row count disagrees with validation proof"
        })
    ));
    assert!(matches!(
        capture_node_scene(&missing_node.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table row count disagrees with validation proof"
        })
    ));
    assert!(matches!(
        capture_node_scene(&truncated_proof.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table row count disagrees with validation proof"
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
            88 + 93 * node_count as u64
        );
        assert_eq!(snapshot.totals.work_units, 7 + 14 * node_count as u64);
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
