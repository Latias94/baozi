mod common;

use baozi_core::{
    AttributeData, AttributeDomain, AttributeRef, AttributeSemanticRef, PrimitiveRef,
};
use baozi_format_obj::ObjFailureCode;
use baozi_import::{
    DiagnosticDetailValue, FormatFailureDetailValue, ImportCause, ImportDiagnostic,
    ImportDimension, ImportLossDisposition, ImportLossKind, ImportLossSubject, ImportReport,
    LossPolicy, ResourceLimits, ScopeAbortCause, SourceScopeEntityKind,
};
use common::{
    TestResult, expected_failure, expected_success, import_bytes,
    import_bytes_result_with_loss_policy,
};

fn primitive(report: &ImportReport) -> PrimitiveRef<'_> {
    assert_eq!(report.scene().mesh_count(), 1);
    assert_eq!(report.scene().primitive_count(), 1);
    report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
}

fn assert_f32(attribute: AttributeRef<'_>, expected_lanes: u8, expected: &[f32]) {
    assert_eq!(attribute.lanes(), expected_lanes);
    assert_eq!(
        attribute.len(),
        expected.len() / usize::from(expected_lanes)
    );
    match attribute.data() {
        AttributeData::DenseF32 { lanes, values } => {
            assert_eq!(lanes, expected_lanes);
            assert_eq!(values, expected);
        }
        payload => panic!("expected dense f32 attribute, got {payload:?}"),
    }
}

fn diagnostic_unsigned(diagnostic: &ImportDiagnostic, key: &str) -> Option<u64> {
    diagnostic.details().iter().find_map(|detail| {
        if detail.key() != key {
            return None;
        }
        match detail.value() {
            DiagnosticDetailValue::Unsigned(value) => Some(*value),
            _ => None,
        }
    })
}

#[test]
fn homogeneous_vertices_are_divided_without_losing_source_positions() -> TestResult<()> {
    let report = import_bytes(
        "homogeneous.obj",
        b"v 2 4 6 2\nv -2 -4 -6 -2\nv 0 2 0 0.5\nf 1 2 3\n",
    )?;
    assert!(report.loss_evidence().is_empty());
    assert!(report.diagnostics().is_empty());
    let primitive = primitive(&report);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.corner_count(), 3);
    assert_eq!(primitive.face_count(), 1);
    assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
    assert_f32(
        primitive.positions().unwrap(),
        3,
        &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 0.0, 4.0, 0.0],
    );
    // Three source positions and three canonical positions; equal coordinates do not merge IDs.
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::Vertices)
            .unwrap()
            .consumed(),
        6
    );
    Ok(())
}

#[test]
fn uv_width_is_selected_per_segment_and_three_components_are_preserved() -> TestResult<()> {
    let report = import_bytes(
        "uv-width.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\n\
vt 0.25 0.5\nvt 1 0.5\nvt 0.25 1\n\
vt 0 0 0.75\nvt 1 0 0\nvt 0 1 1.5\n\
g flat\nf 1/1 2/2 3/3\n\
g volume\nf 1/-3 2/-2 3/-1\nf 1/1 3/-1 2/-2\n",
    )?;
    assert!(report.loss_evidence().is_empty());
    assert!(report.diagnostics().is_empty());
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 2);
    assert_eq!(scene.primitive_count(), 2);
    assert_eq!(scene.attribute_count(), 4);
    let flat = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(flat.vertex_count(), 3);
    let flat_uv = flat.texcoords(0).unwrap();
    assert_eq!(flat_uv.domain(), AttributeDomain::Corner);
    assert_f32(flat_uv, 2, &[0.25, 0.5, 1.0, 0.5, 0.25, 1.0]);
    let volume = scene
        .meshes()
        .next_back()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(volume.vertex_count(), 3);
    assert_eq!(volume.corner_count(), 6);
    assert_eq!(volume.face_count(), 2);
    assert_eq!(volume.indices(), Some(&[0, 1, 2, 0, 2, 1][..]));
    let volume_uv = volume.texcoords(0).unwrap();
    assert_eq!(volume_uv.domain(), AttributeDomain::Corner);
    assert_f32(
        volume_uv,
        3,
        &[
            0.0, 0.0, 0.75, 1.0, 0.0, 0.0, 0.0, 1.0, 1.5, 0.25, 0.5, 0.0, 0.0, 1.0, 1.5, 1.0, 0.0,
            0.0,
        ],
    );
    Ok(())
}

#[test]
fn an_authored_zero_third_uv_component_still_has_three_lanes() -> TestResult<()> {
    let report = import_bytes(
        "zero-third-uv.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.5 0\nf 1/1 2/1 3/1\n",
    )?;
    assert!(report.loss_evidence().is_empty());
    let primitive = primitive(&report);
    assert_eq!(primitive.vertex_count(), 3);
    assert_f32(
        primitive.texcoords(0).unwrap(),
        3,
        &[0.25, 0.5, 0.0, 0.25, 0.5, 0.0, 0.25, 0.5, 0.0],
    );
    Ok(())
}

#[test]
fn color_record_shapes_require_source_semantic_loss_authority() -> TestResult<()> {
    for statement in ["v 2 0 0 0.5 0.25 1", "v 2 0 0 0.5 0.25 1 0.75"] {
        let document = format!("{statement}\nv 0 1 0\nv 0 0 1\nf 1 2 3\n");
        for policy in [
            LossPolicy::preserve_or_fail(),
            LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
        ] {
            let failure = expected_failure(import_bytes_result_with_loss_policy(
                "colors-rejected.obj",
                document.as_bytes(),
                ResourceLimits::default(),
                policy,
            )?);
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
            assert_eq!(failure.loss_evidence().len(), 1);
            assert_eq!(failure.diagnostics().len(), 1);
            let evidence = &failure.loss_evidence()[0];
            assert_eq!(evidence.kind(), ImportLossKind::SourceSemanticLoss);
            assert_eq!(evidence.code().as_str(), "obj.vertex_color_omitted");
            assert_eq!(
                evidence.disposition(),
                ImportLossDisposition::RejectedByLossPolicy
            );
            assert_eq!(evidence.source().as_str(), "colors-rejected.obj");
            assert!(
                matches!(failure.cause(), ImportCause::Scope(abort) if matches!(
                    abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }
                        if *code == evidence.code()
                ))
            );
            assert_eq!(
                failure.diagnostics()[0].object_path(),
                Some("/records/1/vertex-color")
            );
        }
    }
    Ok(())
}

#[test]
fn authorized_color_omission_preserves_xyz_and_records_each_source_record() -> TestResult<()> {
    let report = expected_success(import_bytes_result_with_loss_policy(
        "colors-applied.obj",
        b"v 2 0 0 0.5 0.25 1\nv 0 4 0 0.25 0.5 0.75 1\nv 0 0 6\nf 1 2 3\n",
        ResourceLimits::default(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    let primitive = primitive(&report);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.corner_count(), 3);
    assert_eq!(primitive.face_count(), 1);
    assert_eq!(primitive.attributes().len(), 1);
    assert!(
        primitive
            .attribute(AttributeSemanticRef::Color(0))
            .is_none()
    );
    assert_f32(
        primitive.positions().unwrap(),
        3,
        &[2.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 6.0],
    );
    assert_eq!(report.loss_evidence().len(), 2);
    assert_eq!(report.diagnostics().len(), 2);
    for (index, components) in [6, 7].into_iter().enumerate() {
        let evidence = &report.loss_evidence()[index];
        let diagnostic = &report.diagnostics()[index];
        assert_eq!(evidence.kind(), ImportLossKind::SourceSemanticLoss);
        assert_eq!(
            evidence.subject(),
            ImportLossSubject::SourceEntity(SourceScopeEntityKind::Mesh)
        );
        assert_eq!(evidence.disposition(), ImportLossDisposition::Applied);
        assert_eq!(evidence.code().as_str(), "obj.vertex_color_omitted");
        assert_eq!(evidence.source().as_str(), "colors-applied.obj");
        assert_eq!(diagnostic.code(), evidence.code());
        assert_eq!(
            diagnostic.object_path(),
            Some(format!("/records/{}/vertex-color", index + 1).as_str())
        );
        assert_eq!(
            diagnostic_unsigned(diagnostic, "line"),
            Some(index as u64 + 1)
        );
        assert_eq!(
            diagnostic_unsigned(diagnostic, "component_count"),
            Some(components)
        );
        assert_eq!(
            diagnostic_unsigned(diagnostic, "discarded_components"),
            Some(components - 3)
        );
    }
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::Vertices)
            .unwrap()
            .consumed(),
        6
    );
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::LossEvidenceCount)
            .unwrap()
            .consumed(),
        2
    );
    Ok(())
}

#[test]
fn malformed_numeric_shapes_are_not_rescued_by_loss_authority() -> TestResult<()> {
    let cases = [
        (
            "v 1 2 3 0",
            ObjFailureCode::InvalidStatement,
            "zero-homogeneous-weight",
        ),
        (
            "v 1 2 3 -0",
            ObjFailureCode::InvalidStatement,
            "zero-homogeneous-weight",
        ),
        ("v 1 2 3 NaN", ObjFailureCode::NonFiniteNumber, "non-finite"),
        ("v 1 2 3 inf", ObjFailureCode::NonFiniteNumber, "non-finite"),
        (
            "v 1 2 3 nope",
            ObjFailureCode::InvalidNumber,
            "invalid-float",
        ),
        (
            "v 3e38 0 0 0.1",
            ObjFailureCode::NonFiniteNumber,
            "non-finite",
        ),
        (
            "v 1 2 3 0.5 1",
            ObjFailureCode::InvalidStatement,
            "unsupported-component-count",
        ),
        (
            "v 1 2 3 1 0 0 1 1",
            ObjFailureCode::InvalidStatement,
            "unsupported-component-count",
        ),
        (
            "v 1 2 3 0.5 NaN 1",
            ObjFailureCode::NonFiniteNumber,
            "non-finite",
        ),
        (
            "v 1 2 3 0.5 1 0 nope",
            ObjFailureCode::InvalidNumber,
            "invalid-float",
        ),
        ("vt 0 1 inf", ObjFailureCode::NonFiniteNumber, "non-finite"),
        (
            "vt 0 1 nope",
            ObjFailureCode::InvalidNumber,
            "invalid-float",
        ),
        (
            "vt 0 1 2 3",
            ObjFailureCode::InvalidStatement,
            "unsupported-component-count",
        ),
        (
            "vn 0 0 1 2",
            ObjFailureCode::InvalidStatement,
            "unsupported-component-count",
        ),
    ];
    for (statement, code, reason) in cases {
        let document = format!("{statement}\n");
        let failure = expected_failure(import_bytes_result_with_loss_policy(
            "invalid-record.obj",
            document.as_bytes(),
            ResourceLimits::default(),
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        )?);
        assert!(failure.loss_evidence().is_empty(), "{statement}");
        assert_eq!(failure.ledger().committed_canonical_members(), 0);
        let ImportCause::Format(envelope) = failure.cause() else {
            panic!(
                "expected format failure for {statement}, got {:?}",
                failure.cause()
            );
        };
        assert_eq!(envelope.code(), code.as_str(), "{statement}");
        assert!(envelope.details().iter().any(|detail| detail.key() == "reason" && matches!(
            detail.value(), FormatFailureDetailValue::String(value) if value.as_str() == reason
        )), "{statement}");
        assert!(
            envelope
                .details()
                .iter()
                .any(|detail| detail.key() == "line"
                    && matches!(detail.value(), FormatFailureDetailValue::Unsigned(1))),
            "{statement}"
        );
    }
    Ok(())
}
