mod common;

use baozi_import::{
    ImportCause, ImportLossDisposition, ImportLossKind, LossPolicy, ResourceLimits, ScopeAbortCause,
};
use common::{TestResult, expected_error, expected_report, import_bytes_result_with_loss_policy};

fn fixture(format: &str, property: &str, scalar: &str) -> Vec<u8> {
    let mut bytes = format!(
        "ply\nformat {format} 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty {scalar} {property}\nend_header\n"
    ).into_bytes();
    if format == "ascii" {
        bytes.extend_from_slice(b"0 0 0 1\n");
    } else {
        bytes.extend_from_slice(&[0; 12]);
        if scalar == "uchar" {
            bytes.push(1);
        } else {
            bytes.extend_from_slice(&if format == "binary_little_endian" {
                1.0_f32.to_le_bytes()
            } else {
                1.0_f32.to_be_bytes()
            });
        }
    }
    bytes
}

#[test]
fn each_partial_channel_requires_semantic_loss_permission_in_every_encoding() -> TestResult<()> {
    for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
        for (property, scalar, channel, code) in [
            ("nx", "float", "normal", "ply.partial_normals_ignored"),
            ("s", "float", "texcoord", "ply.partial_texcoords_ignored"),
            ("red", "uchar", "color", "ply.partial_colors_ignored"),
        ] {
            let bytes = fixture(format, property, scalar);
            for loss_policy in [
                LossPolicy::preserve_or_fail(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
            ] {
                let failure = expected_error(import_bytes_result_with_loss_policy(
                    "partial.ply",
                    &bytes,
                    ResourceLimits::default(),
                    loss_policy,
                )?)?;
                assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                    if matches!(abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { .. })));
                assert_eq!(failure.loss_evidence().len(), 1);
                assert_eq!(failure.loss_evidence()[0].code().as_str(), code);
                assert_eq!(
                    failure.loss_evidence()[0].disposition(),
                    ImportLossDisposition::RejectedByLossPolicy
                );
                assert_eq!(failure.ledger().committed_canonical_members(), 0);
            }

            let report = expected_report(import_bytes_result_with_loss_policy(
                "partial.ply",
                &bytes,
                ResourceLimits::default(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )?)?;
            assert_eq!(report.loss_evidence().len(), 1);
            assert_eq!(
                report.loss_evidence()[0].disposition(),
                ImportLossDisposition::Applied
            );
            assert_eq!(
                report.loss_evidence()[0].kind(),
                ImportLossKind::SourceSemanticLoss
            );
            assert_eq!(report.loss_evidence()[0].code().as_str(), code);
            assert_eq!(
                report.diagnostics()[0].object_path(),
                Some(format!("/elements/vertex/properties/{channel}").as_str())
            );
            assert_eq!(
                report
                    .scene()
                    .meshes()
                    .next()
                    .unwrap()
                    .primitives()
                    .next()
                    .unwrap()
                    .attributes()
                    .len(),
                1
            );
        }
    }
    Ok(())
}

#[test]
fn omitted_partial_double_colors_do_not_report_discarded_narrowing_per_vertex() -> TestResult<()> {
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty double red\nend_header\n0 0 0 0.1\n1 0 0 0.2\n0 1 0 0.3\n";
    let mut limits = ResourceLimits::default();
    limits.max_loss_evidence = 1;
    let report = expected_report(import_bytes_result_with_loss_policy(
        "partial-double.ply",
        bytes,
        limits,
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?)?;
    assert_eq!(report.loss_evidence().len(), 1);
    assert_eq!(
        report.loss_evidence()[0].code().as_str(),
        "ply.partial_colors_ignored"
    );
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .attributes()
            .len(),
        1
    );
    Ok(())
}
