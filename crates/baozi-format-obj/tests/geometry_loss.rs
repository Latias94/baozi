mod common;

use baozi_import::{
    ConformancePolicy, ImportCause, ImportLossDisposition, ImportLossKind, ImportLossSubject,
    ImportOperationControl, LossPolicy, ResourceLimits, ScopeAbortCause, SourceScopeEntityKind,
};
use common::{TestResult, expected_failure, expected_success, import_assets_result_with_policies};

const FACE: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nvt 0 0\nvt 1 1\nf 1 2 3\n";

#[test]
fn point_and_line_omissions_require_semantic_authority() -> TestResult<()> {
    for (statement, code) in [
        ("p 1 -1", "obj.point_geometry_omitted"),
        ("l 1 2 -1", "obj.line_geometry_omitted"),
        ("l 1/1 2/2", "obj.line_geometry_omitted"),
    ] {
        let document = format!("{FACE}{statement}\n");
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            for policy in [
                LossPolicy::preserve_or_fail(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
            ] {
                let failure = expected_failure(import_assets_result_with_policies(
                    "geometry.obj",
                    [("geometry.obj", document.as_bytes())],
                    ResourceLimits::default(),
                    None,
                    conformance,
                    policy,
                    ImportOperationControl::default(),
                )?);
                assert_eq!(failure.loss_evidence().len(), 1);
                let loss = &failure.loss_evidence()[0];
                assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
                assert_eq!(loss.code().as_str(), code);
                assert_eq!(
                    loss.disposition(),
                    ImportLossDisposition::RejectedByLossPolicy
                );
                assert_eq!(failure.ledger().committed_canonical_members(), 0);
                assert!(
                    matches!(failure.cause(), ImportCause::Scope(abort) if matches!(
                        abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { .. }
                    ))
                );
            }
            let report = expected_success(import_assets_result_with_policies(
                "geometry.obj",
                [("geometry.obj", document.as_bytes())],
                ResourceLimits::default(),
                None,
                conformance,
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
                ImportOperationControl::default(),
            )?);
            assert_eq!(report.loss_evidence().len(), 1);
            let loss = &report.loss_evidence()[0];
            assert_eq!(loss.code().as_str(), code);
            assert_eq!(loss.disposition(), ImportLossDisposition::Applied);
            assert_eq!(
                loss.subject(),
                ImportLossSubject::SourceEntity(SourceScopeEntityKind::Primitive)
            );
            assert_eq!(loss.source().as_str(), "geometry.obj");
            let path = if statement.starts_with('p') {
                "/records/8/p"
            } else {
                "/records/8/l"
            };
            assert_eq!(report.diagnostics()[0].object_path(), Some(path));
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            assert_eq!(primitive.vertex_count(), 3);
            assert_eq!(primitive.face_count(), 1);
            assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
        }
    }
    Ok(())
}

#[test]
fn mixed_faces_lines_and_points_record_separate_authorized_omissions() -> TestResult<()> {
    let document = format!("{FACE}l 1/1 -1/2\np 2 3\nf 1 3 4\n");
    let report = expected_success(import_assets_result_with_policies(
        "mixed.obj",
        [("mixed.obj", document.as_bytes())],
        ResourceLimits::default(),
        None,
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        ImportOperationControl::default(),
    )?);
    assert_eq!(report.loss_evidence().len(), 2);
    assert_eq!(
        report.loss_evidence()[0].code().as_str(),
        "obj.line_geometry_omitted"
    );
    assert_eq!(
        report.loss_evidence()[1].code().as_str(),
        "obj.point_geometry_omitted"
    );
    assert!(
        report
            .loss_evidence()
            .iter()
            .all(|loss| { loss.disposition() == ImportLossDisposition::Applied })
    );
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.vertex_count(), 4);
    assert_eq!(primitive.face_count(), 2);
    assert_eq!(primitive.indices(), Some(&[0, 1, 2, 0, 2, 3][..]));
    Ok(())
}

#[test]
fn malformed_point_or_line_records_remain_format_errors() -> TestResult<()> {
    for statement in [
        "p",
        "p 0",
        "p 5",
        "p nope",
        "p 1/1",
        "p 1//1",
        "p 1/1/1",
        "l",
        "l 1",
        "l 1 0",
        "l 1 5",
        "l 1/0 2/1",
        "l 1/3 2/1",
        "l 1/ 2/1",
        "l 1//1 2//1",
        "l 1/1/1 2/1/1",
    ] {
        let document = format!("{FACE}{statement}\n");
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            let failure = expected_failure(import_assets_result_with_policies(
                "malformed.obj",
                [("malformed.obj", document.as_bytes())],
                ResourceLimits::default(),
                None,
                conformance,
                LossPolicy::preserve_or_fail()
                    .allow(ImportLossKind::SourceSemanticLoss)
                    .allow(ImportLossKind::FormatRecovery),
                ImportOperationControl::default(),
            )?);
            assert!(
                matches!(failure.cause(), ImportCause::Format(_)),
                "{statement}"
            );
            assert!(failure.loss_evidence().is_empty(), "{statement}");
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
        }
    }
    Ok(())
}
