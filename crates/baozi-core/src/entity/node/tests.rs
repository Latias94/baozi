use super::*;
use crate::entity::AuthoredTransformKind;
use crate::math::{Quat, Vec3};
use crate::provenance::{ProvenanceAttachments, test_provenance_fixture};

#[test]
fn node_stores_only_canonical_child_and_attachment_indexes() {
    let (_arena, provenance) = test_provenance_fixture();
    let child = NodeIndex::try_from(3).expect("fixture index should fit");
    let camera = CameraIndex::try_from(1).expect("fixture index should fit");
    let light = LightIndex::try_from(2).expect("fixture index should fit");
    let node = Node::try_new_unchecked_references_for_test(
        EntityName::try_new("root").ok(),
        AuthoredTransform::identity_trs(),
        [child],
        Some(camera),
        Some(light),
        provenance,
        [],
    )
    .expect("node should be valid");

    assert_eq!(node.children().collect::<Vec<_>>(), vec![child]);
    assert_eq!(node.camera_index(), Some(camera));
    assert_eq!(node.light_index(), Some(light));
    assert_eq!(node.name().map(EntityName::as_str), Some("root"));
    assert_eq!(node.authored_transform().kind(), AuthoredTransformKind::Trs);
    assert_eq!(node.local_matrix(), Mat4::IDENTITY);
}

#[test]
fn node_preserves_every_provenance_attachment() {
    let (_arena, [first, second]) = crate::provenance::test_provenance_pair_fixture();
    let attachments = ProvenanceAttachments::try_from_iter([first, second]).unwrap();
    let node = Node::try_new_unchecked_references_for_test(
        None,
        AuthoredTransform::identity_trs(),
        [],
        None,
        None,
        attachments,
        [],
    )
    .unwrap();

    assert_eq!(
        node.provenance().iter().collect::<Vec<_>>(),
        [first, second]
    );
}

#[test]
fn node_transform_fork_shares_every_non_transform_attachment() {
    let (_arena, provenance) = test_provenance_fixture();
    let child = NodeIndex::from_u32(7);
    let camera = CameraIndex::from_u32(3);
    let light = LightIndex::from_u32(5);
    let extension = ExtensionEntry::descriptive_metadata(
        crate::namespaced::NamespacedName::new("test:node-metadata").unwrap(),
        crate::extension::ExtensionValue::from_bool(true),
        provenance,
    );
    let node = Node::try_new_unchecked_references_for_test(
        EntityName::try_new("shared-node").ok(),
        AuthoredTransform::identity_trs(),
        [child],
        Some(camera),
        Some(light),
        provenance,
        [extension],
    )
    .unwrap();
    let transformed = node.with_authored_transform(
        AuthoredTransform::try_trs(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::IDENTITY,
            Vec3::new(1.0, 1.0, 1.0),
        )
        .unwrap(),
    );

    assert!(node.shares_payload_with(&transformed));
    assert_eq!(
        transformed.name().map(EntityName::as_str),
        Some("shared-node")
    );
    assert_eq!(transformed.children().collect::<Vec<_>>(), [child]);
    assert_eq!(transformed.camera_index(), Some(camera));
    assert_eq!(transformed.light_index(), Some(light));
    assert_eq!(transformed.provenance(), node.provenance());
    assert_eq!(
        transformed.extensions().collect::<Vec<_>>(),
        node.extensions().collect::<Vec<_>>()
    );
}

#[test]
fn node_payload_allocation_failure_is_typed() {
    let (_arena, provenance) = test_provenance_fixture();
    crate::shared_storage::fail_next_allocation_for_test();

    assert_eq!(
        Node::try_new_unchecked_references_for_test(
            None,
            AuthoredTransform::identity_trs(),
            [],
            None,
            None,
            provenance,
            [],
        )
        .unwrap_err(),
        NodeError::PayloadAllocationFailed
    );
}

#[test]
fn prepared_node_parts_are_adopted_once_and_report_disjoint_footprints() {
    let (_arena, provenance) = test_provenance_fixture();
    let child = NodeIndex::from_u32(3);
    let mut children = Vec::with_capacity(4);
    children.push(child);
    let children_ptr = children.as_ptr();
    let children_capacity = children.capacity();

    let (node, footprint) = Node::try_from_prepared_parts(NodePreparedParts {
        name: Some(EntityName::try_new("prepared-node").expect("fixture name should be valid")),
        authored_transform: AuthoredTransform::identity_trs(),
        children,
        camera: None,
        light: None,
        provenance: provenance.into(),
        extensions: Vec::new(),
    })
    .expect("prepared node fields should enter shared storage");

    assert_eq!(node.payload.as_ref().children.as_ptr(), children_ptr);
    assert_eq!(node.payload.as_ref().children.capacity(), children_capacity);
    assert_eq!(
        footprint.value_storage_bytes(),
        Node::value_storage_layout_bytes().unwrap()
    );
    assert_eq!(
        u64::try_from(node.retained_allocation_bytes().unwrap()).unwrap(),
        footprint.value_storage_bytes() + footprint.nested_payload_bytes()
    );
}
