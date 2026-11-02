mod common;

use baozi_core::{AttributeData, AttributeDomain, TopologyKind};
use baozi_format_obj::ObjFailureCode;
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDiagnostic, ImportLossDisposition,
    ImportLossEvidence, ImportLossKind, ImportLossSubject, LossPolicy, ResourceLimits,
    SceneSnapshot, ScopeAbortCause, SourceScopeEntityKind,
};
use common::{
    TestResult, expected_failure, expected_success, import_assets, import_bytes,
    import_bytes_result_with_loss_policy, scene_snapshot, snapshot_line, snapshot_line_containing,
};

const PARTIAL_TEXCOORD_OBJ: &[u8] =
    b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2//1 3/3/1\n";
const PARTIAL_NORMAL_OBJ: &[u8] =
    b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2 3/3/1\n";
const PARTIAL_BOTH_OBJ: &[u8] = b"g mixed\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\nf 1 2 3\n";

// Original fixture: every source table has a different length at each face record.
const NEGATIVE_CORNER_OBJ: &str = "v 10 0 0\nv 0 20 0\nv 0 0 30\n\
    vt 0.125 0.25\nvt 0.5 0.75\nvt 0.875 1\nvt -1 2\n\
    vn 2 0 0\nvn 0 -3 0\ng first\nf -3/-4/-2 -1/-1/-1 2/2/1\n\
    v 99 98 97\nvt 9 8\nvn 0 0 7\ng second\nf -1/-1/-1 1/1/1 2/2/2\n";

fn assert_triangle_positions(snapshot: &SceneSnapshot) {
    assert!(
        snapshot_line_containing(snapshot, "semantic=position").contains(concat!(
            "values(present,total=9)=[",
            "0x00000000,0x00000000,0x00000000,",
            "0x3f800000,0x00000000,0x00000000,",
            "0x00000000,0x3f800000,0x00000000]"
        ))
    );
}

fn assert_partial_channel_loss(
    evidence: &ImportLossEvidence,
    diagnostic: &ImportDiagnostic,
    source: &str,
    segment_index: usize,
    channel: &str,
    disposition: ImportLossDisposition,
) {
    let code = format!("obj.partial_{channel}_channel");
    let object_path = format!("/segments/{segment_index}/attributes/{channel}");
    assert_eq!(evidence.kind(), ImportLossKind::SourceSemanticLoss);
    assert_eq!(
        evidence.subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::Mesh)
    );
    assert_eq!(evidence.code().as_str(), code);
    assert_eq!(evidence.source().as_str(), source);
    assert_eq!(evidence.raw_range(), None);
    assert_eq!(evidence.disposition(), disposition);
    assert_eq!(diagnostic.code(), evidence.code());
    assert_eq!(diagnostic.object_path(), Some(object_path.as_str()));
}

#[test]
fn imports_one_triangle_obj() -> TestResult<()> {
    let bytes = b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_bytes("triangle.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert_eq!(snapshot.mesh_instance_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.polygon_boundary_count(), 0);
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=triangle_list vertices=3 corners=3 faces=1 ",
        "polygon_boundaries(absent,total=0)=[] indices(present,total=3)=[0,1,2]"
    )));
    assert!(snapshot_line_containing(&snapshot, "semantic=position").contains("elements=3"));
    Ok(())
}

#[test]
fn imports_quad_as_polygon_with_face_counts() -> TestResult<()> {
    let bytes = b"g quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";

    let report = import_bytes("quad.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=quad "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    Ok(())
}

#[test]
fn preserves_corner_uv_and_normal_domains_without_duplicating_positions() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\nvt 0.25 0.25\nvn 0 0 1\nvn 0 0 -1\nvn 1 0 0\nf 1/1/1 2/2/1 3/3/1\nf 1/4/2 3/3/2 2/2/2\n";

    let report = import_bytes("remap.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(snapshot.attribute_count(), 3);
    assert_eq!(snapshot.primitive_index_count(), 6);
    assert_eq!(snapshot.f32_scalar_count(), 39);
    let primitive = snapshot_line(&snapshot, "primitive[0]");
    assert!(
        primitive.contains("topology=triangle_list vertices=3 corners=6 faces=2")
            && primitive.contains("indices(present,total=6)=[0,1,2,0,2,1]"),
        "unexpected canonical primitive: {primitive}"
    );
    let position = snapshot_line_containing(&snapshot, "semantic=position");
    assert!(position.contains("domain=vertex scalar=f32 lanes=3"));
    assert!(position.contains("payload=dense elements=3 explicit=3 canonical_scalars=9"));

    let texcoord = snapshot_line_containing(&snapshot, "semantic=texcoord");
    assert!(texcoord.contains("domain=corner scalar=f32 lanes=2"));
    assert!(texcoord.contains("payload=dense elements=6 explicit=6 canonical_scalars=12"));
    assert!(texcoord.contains(concat!(
        "values(present,total=12)=[",
        "0x00000000,0x00000000,0x3f800000,0x00000000,",
        "0x00000000,0x3f800000,0x3f800000,0x3f800000,",
        "0x00000000,0x3f800000,0x3f800000,0x00000000]"
    )));

    let normal = snapshot_line_containing(&snapshot, "semantic=normal");
    assert!(normal.contains("domain=corner scalar=f32 lanes=3"));
    assert!(normal.contains("payload=dense elements=6 explicit=6 canonical_scalars=18"));
    assert!(normal.contains(concat!(
        "values(present,total=18)=[",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0xbf800000,",
        "0x00000000,0x00000000,0xbf800000,",
        "0x00000000,0x00000000,0xbf800000]"
    )));
    Ok(())
}

#[test]
fn isolates_corner_source_projection_between_segments() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 1 0\nvt 2 0\nvt 3 0\nvt 4 0\nvn 1 0 0\nvn 2 0 0\nvn 3 0 0\nvn 4 0 0\ng first\nf 1/1/1 2/2/2 3/3/3\ng second\nf 1/3/3 3/4/4 2/1/1\n";

    let report = import_bytes("segment-remap.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.attribute_count(), 6);
    assert!(snapshot_line(&snapshot, "attribute[1]").contains(concat!(
        "semantic=normal semantic_set=0 domain=corner scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=9 ",
        "values(present,total=9)=[",
        "0x3f800000,0x00000000,0x00000000,",
        "0x40000000,0x00000000,0x00000000,",
        "0x40400000,0x00000000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[2]").contains(concat!(
        "semantic=texcoord semantic_set=0 domain=corner scalar=f32 lanes=2 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=6 ",
        "values(present,total=6)=[",
        "0x3f800000,0x00000000,0x40000000,0x00000000,",
        "0x40400000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[4]").contains(concat!(
        "semantic=normal semantic_set=0 domain=corner scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=9 ",
        "values(present,total=9)=[",
        "0x40400000,0x00000000,0x00000000,",
        "0x40800000,0x00000000,0x00000000,",
        "0x3f800000,0x00000000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[5]").contains(concat!(
        "semantic=texcoord semantic_set=0 domain=corner scalar=f32 lanes=2 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=6 ",
        "values(present,total=6)=[",
        "0x40400000,0x00000000,0x40800000,0x00000000,",
        "0x3f800000,0x00000000]"
    )));
    Ok(())
}

#[test]
fn partial_channels_require_source_semantic_loss_authority() -> TestResult<()> {
    let cases = [
        ("partial-uv.obj", PARTIAL_TEXCOORD_OBJ, "texcoord"),
        ("partial-normal.obj", PARTIAL_NORMAL_OBJ, "normal"),
        ("partial-both.obj", PARTIAL_BOTH_OBJ, "texcoord"),
    ];
    for (source, bytes, first_partial_channel) in cases {
        for loss_policy in [
            LossPolicy::preserve_or_fail(),
            LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
        ] {
            let failure = expected_failure(import_bytes_result_with_loss_policy(
                source,
                bytes,
                ResourceLimits::default(),
                loss_policy,
            )?);

            assert_eq!(failure.loss_evidence().len(), 1);
            assert_eq!(failure.diagnostics().len(), 1);
            assert!(matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(
                        abort.cause(),
                        ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }
                            if *code == failure.loss_evidence()[0].code()
                    )
            ));
            assert_partial_channel_loss(
                &failure.loss_evidence()[0],
                &failure.diagnostics()[0],
                source,
                0,
                first_partial_channel,
                ImportLossDisposition::RejectedByLossPolicy,
            );
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
        }
    }
    Ok(())
}

#[test]
fn explicit_loss_authority_retains_complete_normals_when_texcoords_are_partial() -> TestResult<()> {
    let report = expected_success(import_bytes_result_with_loss_policy(
        "partial-uv.obj",
        PARTIAL_TEXCOORD_OBJ,
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 2);
    assert!(!snapshot.preview().contains("semantic=texcoord"));
    assert_triangle_positions(&snapshot);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=triangle_list vertices=3 corners=3 faces=1 ",
        "polygon_boundaries(absent,total=0)=[] indices(present,total=3)=[0,1,2]"
    )));
    assert!(
        snapshot_line_containing(&snapshot, "semantic=normal").contains(concat!(
            "domain=corner scalar=f32 lanes=3 normalization=none ",
            "payload=dense elements=3 explicit=3 canonical_scalars=9 ",
            "values(present,total=9)=[",
            "0x00000000,0x00000000,0x3f800000,",
            "0x00000000,0x00000000,0x3f800000,",
            "0x00000000,0x00000000,0x3f800000]"
        ))
    );
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_partial_channel_loss(
        &report.loss_evidence()[0],
        &report.diagnostics()[0],
        "partial-uv.obj",
        0,
        "texcoord",
        ImportLossDisposition::Applied,
    );
    Ok(())
}

#[test]
fn explicit_loss_authority_retains_complete_texcoords_when_normals_are_partial() -> TestResult<()> {
    let report = expected_success(import_bytes_result_with_loss_policy(
        "partial-normal.obj",
        PARTIAL_NORMAL_OBJ,
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 2);
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert_triangle_positions(&snapshot);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=triangle_list vertices=3 corners=3 faces=1 ",
        "polygon_boundaries(absent,total=0)=[] indices(present,total=3)=[0,1,2]"
    )));
    assert!(
        snapshot_line_containing(&snapshot, "semantic=texcoord").contains(concat!(
            "domain=corner scalar=f32 lanes=2 normalization=none ",
            "payload=dense elements=3 explicit=3 canonical_scalars=6 ",
            "values(present,total=6)=[",
            "0x00000000,0x00000000,0x3f800000,0x00000000,",
            "0x00000000,0x3f800000]"
        ))
    );
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_partial_channel_loss(
        &report.loss_evidence()[0],
        &report.diagnostics()[0],
        "partial-normal.obj",
        0,
        "normal",
        ImportLossDisposition::Applied,
    );
    Ok(())
}

#[test]
fn preserves_corner_uv_and_normal_cardinality_for_polygons() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\n";

    let report = import_bytes("corner-polygon.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.attribute_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 32);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    let texcoord = snapshot_line_containing(&snapshot, "semantic=texcoord");
    assert!(texcoord.contains("domain=corner scalar=f32 lanes=2"));
    assert!(texcoord.contains("payload=dense elements=4"));
    let normal = snapshot_line_containing(&snapshot, "semantic=normal");
    assert!(normal.contains("domain=corner scalar=f32 lanes=3"));
    assert!(normal.contains("payload=dense elements=4"));
    Ok(())
}

#[test]
fn supports_negative_face_indices() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n";

    let report = import_bytes("negative.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    Ok(())
}

#[test]
fn negative_corner_indices_use_each_table_at_the_face_record() -> TestResult<()> {
    let report = import_bytes("negative-corners.obj", NEGATIVE_CORNER_OBJ.as_bytes())?;

    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 2);
    assert_eq!(scene.primitive_count(), 2);
    assert_eq!(scene.attribute_count(), 6);

    let expected = [
        (
            "first",
            [[10.0_f32, 0.0, 0.0], [0.0, 0.0, 30.0], [0.0, 20.0, 0.0]],
            [[0.125_f32, 0.25], [-1.0, 2.0], [0.5, 0.75]],
            [[2.0_f32, 0.0, 0.0], [0.0, -3.0, 0.0], [2.0, 0.0, 0.0]],
        ),
        (
            "second",
            [[99.0, 98.0, 97.0], [10.0, 0.0, 0.0], [0.0, 20.0, 0.0]],
            [[9.0, 8.0], [0.125, 0.25], [0.5, 0.75]],
            [[0.0, 0.0, 7.0], [2.0, 0.0, 0.0], [0.0, -3.0, 0.0]],
        ),
    ];
    for (mesh, (name, expected_positions, expected_texcoords, expected_normals)) in
        scene.meshes().zip(expected)
    {
        assert_eq!(mesh.name(), Some(name));
        assert_eq!(mesh.primitive_count(), 1);
        let primitive = mesh
            .primitives()
            .next()
            .expect("one OBJ triangle per segment");
        assert_eq!(primitive.topology(), TopologyKind::TriangleList);
        assert_eq!(primitive.vertex_count(), 3);
        assert_eq!(primitive.corner_count(), 3);
        assert_eq!(primitive.face_count(), 1);
        assert_eq!(primitive.attributes().len(), 3);
        let indices = primitive.indices().expect("indexed OBJ triangle");
        assert_eq!(indices.len(), 3);
        let positions = primitive.positions().expect("OBJ positions");
        let texcoords = primitive
            .texcoords(0, AttributeDomain::Corner)
            .expect("OBJ corner UVs");
        let normals = primitive
            .normals(AttributeDomain::Corner)
            .expect("OBJ corner normals");
        for (attribute, domain, lanes) in [
            (positions, AttributeDomain::Vertex, 3),
            (texcoords, AttributeDomain::Corner, 2),
            (normals, AttributeDomain::Corner, 3),
        ] {
            assert_eq!(attribute.domain(), domain);
            assert_eq!(attribute.lanes(), lanes);
            assert_eq!(attribute.len(), 3);
        }
        let (positions, texcoords, normals) =
            match (positions.data(), texcoords.data(), normals.data()) {
                (
                    AttributeData::DenseF32 {
                        lanes: 3,
                        values: positions,
                    },
                    AttributeData::DenseF32 {
                        lanes: 2,
                        values: texcoords,
                    },
                    AttributeData::DenseF32 {
                        lanes: 3,
                        values: normals,
                    },
                ) => (positions, texcoords, normals),
                payloads => {
                    panic!("expected dense OBJ position/UV/normal streams, got {payloads:?}")
                }
            };
        for (corner, index) in indices.iter().copied().enumerate() {
            let vertex = index as usize;
            assert_eq!(
                &positions[vertex * 3..vertex * 3 + 3],
                &expected_positions[corner],
                "{name} corner {corner} position"
            );
            assert_eq!(
                &texcoords[corner * 2..corner * 2 + 2],
                &expected_texcoords[corner],
                "{name} corner {corner} UV"
            );
            assert_eq!(
                &normals[corner * 3..corner * 3 + 3],
                &expected_normals[corner],
                "{name} corner {corner} normal"
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_corner_indices_are_not_rescued_by_semantic_loss_authority() -> TestResult<()> {
    for (reference, subject, reason) in [
        ("-3/0/-2", "texcoord", "zero"),
        ("-3/-5/-2", "texcoord", "out-of-range"),
        ("-3/-4/0", "normal", "zero"),
        ("-3/-4/-3", "normal", "out-of-range"),
    ] {
        let document = NEGATIVE_CORNER_OBJ.replacen("-3/-4/-2", reference, 1);
        for loss_policy in [
            LossPolicy::preserve_or_fail(),
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        ] {
            let failure = expected_failure(import_bytes_result_with_loss_policy(
                "invalid-negative-corner.obj",
                document.as_bytes(),
                ResourceLimits::default(),
                loss_policy,
            )?);
            let ImportCause::Format(envelope) = failure.cause() else {
                panic!(
                    "expected OBJ format failure for {reference}, got {:?}",
                    failure.cause()
                );
            };
            assert_eq!(envelope.code(), ObjFailureCode::InvalidIndex.as_str());
            for (key, expected) in [("line", 11), ("column", 3)] {
                assert_eq!(
                    envelope
                        .details()
                        .iter()
                        .find(|detail| detail.key() == key)
                        .map(|detail| detail.value()),
                    Some(FormatFailureDetailValue::Unsigned(expected)),
                    "{reference}: {key}"
                );
            }
            for (key, expected) in [("subject", subject), ("reason", reason)] {
                let actual = envelope
                    .details()
                    .iter()
                    .find(|detail| detail.key() == key)
                    .and_then(|detail| match detail.value() {
                        FormatFailureDetailValue::String(value) => Some(value.as_str()),
                        _ => None,
                    });
                assert_eq!(actual, Some(expected), "{reference}: {key}");
            }
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
            assert!(failure.diagnostics().is_empty());
            assert!(failure.loss_evidence().is_empty());
        }
    }
    Ok(())
}

#[test]
fn explicit_loss_authority_drops_partial_uv_and_normal_channels() -> TestResult<()> {
    let report = expected_success(import_bytes_result_with_loss_policy(
        "partial-both.obj",
        PARTIAL_BOTH_OBJ,
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert_triangle_positions(&snapshot);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=triangle_list vertices=3 corners=6 faces=2 ",
        "polygon_boundaries(absent,total=0)=[] indices(present,total=6)=[0,1,2,0,1,2]"
    )));
    assert!(!snapshot.preview().contains("semantic=texcoord"));
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert_eq!(report.loss_evidence().len(), 2);
    assert_eq!(report.diagnostics().len(), 2);
    for (index, channel) in ["texcoord", "normal"].into_iter().enumerate() {
        assert_partial_channel_loss(
            &report.loss_evidence()[index],
            &report.diagnostics()[index],
            "partial-both.obj",
            0,
            channel,
            ImportLossDisposition::Applied,
        );
    }
    Ok(())
}

#[test]
fn partial_channel_loss_is_scoped_to_its_segment() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\ng complete\nf 1/1/1 2/2/1 3/3/1\ng partial\nf 1/1/1 2//1 3/3/1\n";
    let report = expected_success(import_bytes_result_with_loss_policy(
        "partial-segment.obj",
        bytes,
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.attribute_count(), 5);
    assert!(snapshot_line(&snapshot, "attribute[2]").contains("semantic=texcoord"));
    assert!(snapshot_line(&snapshot, "attribute[3]").contains("semantic=position"));
    assert!(snapshot_line(&snapshot, "attribute[4]").contains("semantic=normal"));
    for index in 0..2 {
        assert!(
            snapshot_line(&snapshot, &format!("primitive[{index}]")).contains(concat!(
                "topology=triangle_list vertices=3 corners=3 faces=1 ",
                "polygon_boundaries(absent,total=0)=[] indices(present,total=3)=[0,1,2]"
            ))
        );
    }
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_partial_channel_loss(
        &report.loss_evidence()[0],
        &report.diagnostics()[0],
        "partial-segment.obj",
        1,
        "texcoord",
        ImportLossDisposition::Applied,
    );
    Ok(())
}

#[test]
fn state_changes_split_meshes_and_preserve_material_bindings() -> TestResult<()> {
    let bytes = b"o first-object\ng first-group\nusemtl red\ns 1\n\
        v 0 0 0\nv 1 0 0\nv 0 1 0\nv 2 0 0\nv 3 0 0\nv 2 1 0\n\
        f 1 2 3\no second-object\ng second-group\nusemtl blue\nf 4 5 6\n";

    let report = import_bytes("state-switch.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.primitive_count(), 2);
    assert_eq!(snapshot.mesh_instance_count(), 2);
    assert_eq!(snapshot.material_count(), 2);
    assert_eq!(report.diagnostics().len(), 2);
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=first-group "));
    assert!(snapshot_line(&snapshot, "logical_mesh[1]").contains(" name=second-group "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(" material=0 "));
    assert!(snapshot_line(&snapshot, "primitive[1]").contains(" material=1 "));
    assert!(snapshot_line(&snapshot, "material[0]").contains(" name=red"));
    assert!(snapshot_line(&snapshot, "material[1]").contains(" name=blue"));
    assert!(
        snapshot.preview().lines().any(|line| {
            line.contains("key=obj:object") && line.contains("string=first-object")
        })
    );
    let smoothing_attributes = snapshot
        .preview()
        .lines()
        .filter(|line| line.contains("semantic=smoothing_group"))
        .collect::<Vec<_>>();
    assert_eq!(smoothing_attributes.len(), 2);
    assert!(smoothing_attributes[0].contains("values(present,total=1)=[1]"));
    assert!(smoothing_attributes[1].contains("values(present,total=1)=[1]"));
    assert!(!snapshot.preview().contains("key=obj:smoothing"));
    Ok(())
}

#[test]
fn repeating_the_current_state_does_not_split_the_mesh() -> TestResult<()> {
    let bytes = b"o same\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no same\nf 1 2 3\n";

    let report = import_bytes("same-state.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 6);
    assert!(
        snapshot_line(&snapshot, "primitive[0]").contains("indices(present,total=6)=[0,1,2,0,1,2]")
    );
    Ok(())
}

#[test]
fn smoothing_changes_are_preserved_as_one_face_domain_attribute() -> TestResult<()> {
    let obj = b"mtllib shared.mtl\no shared-object\ng shared-group\nusemtl shared\n\
        v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
        f 1 2 3\ns 7\nf 1 3 4\ns off\nf 1 4 2\n";
    let mtl = b"newmtl shared\nKd 1 1 1\n";

    let report = import_assets(
        "smoothing-groups.obj",
        [
            ("smoothing-groups.obj", obj.as_slice()),
            ("shared.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 2);
    assert_eq!(snapshot.material_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(" material=0 "));
    assert!(!snapshot.preview().contains("key=obj:smoothing"));

    let smoothing = snapshot_line_containing(&snapshot, "semantic=smoothing_group");
    assert!(smoothing.contains("domain=face scalar=u32 lanes=1"));
    assert!(smoothing.contains("payload=dense elements=3 explicit=3 canonical_scalars=3"));
    assert!(smoothing.contains("values(present,total=3)=[0,7,0]"));
    Ok(())
}

#[test]
fn zero_and_off_smoothing_literals_are_canonically_equivalent() -> TestResult<()> {
    let zero_report = import_bytes(
        "zero-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 0\nf 1 2 3\n",
    )?;
    let off_report = import_bytes(
        "off-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns off\nf 1 2 3\n",
    )?;
    let zero_snapshot = scene_snapshot(&zero_report)?;
    let off_snapshot = scene_snapshot(&off_report)?;

    assert!(zero_report.diagnostics().is_empty());
    assert!(off_report.diagnostics().is_empty());
    let zero_smoothing = snapshot_line_containing(&zero_snapshot, "semantic=smoothing_group");
    let off_smoothing = snapshot_line_containing(&off_snapshot, "semantic=smoothing_group");
    assert_eq!(zero_smoothing, off_smoothing);
    assert!(zero_smoothing.contains("values(present,total=1)=[0]"));
    assert!(!zero_snapshot.preview().contains("key=obj:smoothing"));
    assert!(!off_snapshot.preview().contains("key=obj:smoothing"));
    Ok(())
}

#[test]
fn maximum_u32_smoothing_group_is_preserved() -> TestResult<()> {
    let report = import_bytes(
        "maximum-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 4294967295\nf 1 2 3\n",
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(
        snapshot_line_containing(&snapshot, "semantic=smoothing_group")
            .contains("values(present,total=1)=[4294967295]")
    );
    Ok(())
}
