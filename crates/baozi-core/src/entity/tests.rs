use super::*;
use crate::identity::{CameraIndex, LightIndex, NodeIndex};
use crate::provenance::{ProvenanceAttachments, test_provenance_fixture};

#[test]
fn node_stores_only_canonical_child_and_attachment_indexes() {
    let (_arena, provenance) = test_provenance_fixture();
    let child = NodeIndex::try_from(3).expect("fixture index should fit");
    let camera = CameraIndex::try_from(1).expect("fixture index should fit");
    let light = LightIndex::try_from(2).expect("fixture index should fit");
    let node = Node::try_new(
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
    let node = Node::try_new(
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
    let node = Node::try_new(
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
        Node::try_new(
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
fn authored_transform_preserves_matrix_or_trs_without_conversion() {
    let mut transform = Mat4::IDENTITY;
    transform.cols[3][0] = -0.0;
    let matrix = AuthoredTransform::try_matrix(transform).unwrap();
    assert_eq!(matrix.kind(), AuthoredTransformKind::Matrix);
    assert_eq!(matrix.authored_matrix(), Some(transform));
    assert_eq!(matrix.local_matrix(), transform);
    assert_eq!(matrix.authored_trs(), None);

    let trs = AuthoredTransform::try_trs(
        Vec3::new(4.0, 5.0, 6.0),
        Quat::IDENTITY,
        Vec3::new(-2.0, 0.0, 3.0),
    )
    .unwrap();
    assert_eq!(trs.kind(), AuthoredTransformKind::Trs);
    assert_eq!(trs.authored_matrix(), None);
    assert_eq!(
        trs.authored_trs().unwrap().scale(),
        Vec3::new(-2.0, 0.0, 3.0)
    );
    assert_eq!(trs.local_matrix().cols[3], [4.0, 5.0, 6.0, 1.0]);

    let positive_zero_trs = AuthoredTransform::identity_trs();
    let negative_zero_trs = AuthoredTransform::try_trs(
        Vec3::new(-0.0, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .unwrap();
    assert_eq!(positive_zero_trs, negative_zero_trs);
    assert!(!positive_zero_trs.has_same_authored_bits(negative_zero_trs));

    let positive_zero_matrix = AuthoredTransform::try_matrix(Mat4::IDENTITY).unwrap();
    assert_eq!(matrix, positive_zero_matrix);
    assert!(!matrix.has_same_authored_bits(positive_zero_matrix));
}

#[test]
fn authored_transform_rejects_invalid_matrix_and_trs_values() {
    let mut non_finite = Mat4::IDENTITY;
    non_finite.cols[2][1] = f32::NAN;
    assert_eq!(
        AuthoredTransform::try_matrix(non_finite),
        Err(AuthoredTransformError::MatrixNotFinite)
    );

    let mut non_affine = Mat4::IDENTITY;
    non_affine.cols[0][3] = 0.25;
    assert_eq!(
        AuthoredTransform::try_matrix(non_affine),
        Err(AuthoredTransformError::MatrixNotAffine)
    );

    assert_eq!(
        AuthoredTransform::try_trs(
            Vec3::ZERO,
            Quat::from_xyzw(0.0, 0.0, 0.0, 2.0),
            Vec3::new(1.0, 1.0, 1.0),
        ),
        Err(AuthoredTransformError::RotationNotUnit)
    );
}

#[test]
fn camera_projection_rejects_invalid_frusta() {
    assert_eq!(
        CameraProjection::perspective(0.0, None, 0.1, None),
        Err(CameraProjectionError::VerticalFieldOfView)
    );
    assert_eq!(
        CameraProjection::perspective(1.0, Some(0.0), 0.1, None),
        Err(CameraProjectionError::AspectRatio)
    );
    assert_eq!(
        CameraProjection::orthographic(1.0, 1.0, 2.0, 1.0),
        Err(CameraProjectionError::FarPlane)
    );
}

#[test]
fn light_kind_makes_spot_invariants_explicit() {
    assert_eq!(
        LightKind::spot(None, 0.5, 0.25),
        Err(LightError::InvalidSpotCone)
    );
    assert!(LightKind::spot(Some(10.0), 0.25, 0.5).is_ok());
    assert_eq!(LightKind::point(Some(0.0)), Err(LightError::InvalidRange));
}

#[test]
fn entity_name_bounds_and_sanitizes_display_text() {
    assert_eq!(
        EntityName::try_new("bad\nname"),
        Err(EntityNameError::ControlCharacter { offset: 3 })
    );
    let oversized = "x".repeat(MAX_ENTITY_NAME_BYTES + 1);
    assert_eq!(
        EntityName::try_new(oversized),
        Err(EntityNameError::TooLong {
            actual: MAX_ENTITY_NAME_BYTES + 1,
            maximum: MAX_ENTITY_NAME_BYTES,
        })
    );
}
