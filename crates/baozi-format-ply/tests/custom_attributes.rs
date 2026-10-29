mod common;

use baozi_format_ply::PlyFailureCode;
use baozi_import::{
    ImportCause, ImportTerminalCause, ImportTerminalFailureClass, ResourceLimits, ScopeAbortCause,
};
use common::{
    TestResult, assert_requested_terminal_failure, assert_resource_limit, expected_error,
    expected_ply_failure, expected_report, import_bytes_result, ply_string_detail, scene_snapshot,
    snapshot_line_containing,
};

const F64_VALUES: [f64; 3] = [16_777_217.0, 1e100, -0.0];
const F64_ALIAS_VALUES: [f64; 3] = [-16_777_217.0, 0.5, 1e-100];
const F32_VALUES: [f32; 3] = [0.25, 0.5, 1.0];
const I32_VALUES: [i32; 3] = [-1, i32::MIN, i32::MAX];
const U32_VALUES: [u32; 3] = [0, u32::MAX, 16_777_217];

fn custom_scalar_header(format: &str) -> String {
    format!(
        "ply\nformat {format} 1.0\nelement vertex 3\n\
         property float x\nproperty float y\nproperty float z\n\
         property double temperature\nproperty int sample_id\n\
         property float64 density\nproperty float confidence\nproperty uint flags\nend_header\n"
    )
}

fn ascii_custom_scalars(values: &[f64; 3]) -> Vec<u8> {
    let mut document = custom_scalar_header("ascii");
    for (row, value) in values.iter().enumerate() {
        document.push_str(&format!(
            "0 0 0 {value} {} {} {} {}\n",
            I32_VALUES[row], F64_ALIAS_VALUES[row], F32_VALUES[row], U32_VALUES[row]
        ));
    }
    document.into_bytes()
}

fn binary_custom_scalars(little_endian: bool, values: &[f64; 3]) -> Vec<u8> {
    let format = if little_endian {
        "binary_little_endian"
    } else {
        "binary_big_endian"
    };
    let mut bytes = custom_scalar_header(format).into_bytes();
    for (row, value) in values.iter().enumerate() {
        bytes.extend_from_slice(&[0; 12]);
        if little_endian {
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&I32_VALUES[row].to_le_bytes());
            bytes.extend_from_slice(&F64_ALIAS_VALUES[row].to_le_bytes());
            bytes.extend_from_slice(&F32_VALUES[row].to_le_bytes());
            bytes.extend_from_slice(&U32_VALUES[row].to_le_bytes());
        } else {
            bytes.extend_from_slice(&value.to_be_bytes());
            bytes.extend_from_slice(&I32_VALUES[row].to_be_bytes());
            bytes.extend_from_slice(&F64_ALIAS_VALUES[row].to_be_bytes());
            bytes.extend_from_slice(&F32_VALUES[row].to_be_bytes());
            bytes.extend_from_slice(&U32_VALUES[row].to_be_bytes());
        }
    }
    bytes
}

fn assert_custom_scalars_preserved(source: &str, bytes: &[u8]) -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_vertex_attribute_streams_per_mesh = 6;
    limits.max_total_vertex_attribute_cells = 18;
    let report = expected_report(import_bytes_result(source, bytes, limits)?)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
    assert_eq!(snapshot.attribute_count(), 6);
    assert_eq!(snapshot.canonical_scalar_count(), 24);
    assert_eq!(snapshot.f32_scalar_count(), 12);
    assert_eq!(snapshot.i32_scalar_count(), 3);
    assert_eq!(snapshot.u32_scalar_count(), 3);

    let positions = snapshot_line_containing(&snapshot, "semantic=position ");
    assert!(positions.contains("domain=vertex scalar=f32 lanes=3"));

    for (semantic, values) in [
        ("ply:temperature", F64_VALUES),
        ("ply:density", F64_ALIAS_VALUES),
    ] {
        let attribute = snapshot_line_containing(&snapshot, &format!("semantic={semantic} "));
        assert!(attribute.contains(&format!(
            "semantic={semantic} semantic_set=0 domain=vertex scalar=f64 lanes=1 \
             normalization=none payload=dense elements=3"
        )));
        let expected_values = values
            .iter()
            .map(|value| format!("0x{:016x}", value.to_bits()))
            .collect::<Vec<_>>()
            .join(",");
        assert!(attribute.contains(&format!("values(present,total=3)=[{expected_values}]")));
    }
    // These are the exact f64 bits of 16_777_217.0; an f32 round trip changes them.
    assert!(
        snapshot_line_containing(&snapshot, "semantic=ply:temperature ")
            .contains("values(present,total=3)=[0x4170000010000000,")
    );

    for (semantic, scalar, values) in [
        ("ply:confidence", "f32", "0x3e800000,0x3f000000,0x3f800000"),
        ("ply:sample_id", "i32", "-1,-2147483648,2147483647"),
        ("ply:flags", "u32", "0,4294967295,16777217"),
    ] {
        let attribute = snapshot_line_containing(&snapshot, &format!("semantic={semantic} "));
        assert!(attribute.contains(&format!(
            "semantic={semantic} semantic_set=0 domain=vertex scalar={scalar} lanes=1"
        )));
        assert!(attribute.contains(&format!("values(present,total=3)=[{values}]")));
    }
    Ok(())
}

#[test]
fn preserves_ascii_custom_f64_scalars() -> TestResult<()> {
    let bytes = ascii_custom_scalars(&F64_VALUES);
    assert_custom_scalars_preserved("custom-scalars-ascii.ply", &bytes)
}

#[test]
fn preserves_binary_little_endian_custom_f64_scalars() -> TestResult<()> {
    let bytes = binary_custom_scalars(true, &F64_VALUES);
    assert_custom_scalars_preserved("custom-scalars-le.ply", &bytes)
}

#[test]
fn preserves_binary_big_endian_custom_f64_scalars() -> TestResult<()> {
    let bytes = binary_custom_scalars(false, &F64_VALUES);
    assert_custom_scalars_preserved("custom-scalars-be.ply", &bytes)
}

#[test]
fn custom_f64_attribute_budgets_are_checked_before_payload_reads() -> TestResult<()> {
    let mut stream_limits = ResourceLimits::default();
    stream_limits.max_vertex_attribute_streams_per_mesh = 5;
    let mut cell_limits = ResourceLimits::default();
    cell_limits.max_total_vertex_attribute_cells = 17;
    let invalid_values = [f64::NAN, F64_VALUES[1], F64_VALUES[2]];
    for bytes in [
        ascii_custom_scalars(&invalid_values),
        binary_custom_scalars(true, &invalid_values),
        binary_custom_scalars(false, &invalid_values),
    ] {
        for (limits, resource) in [
            (
                stream_limits.clone(),
                "max_vertex_attribute_streams_per_mesh",
            ),
            (cell_limits.clone(), "max_total_vertex_attribute_cells"),
        ] {
            // The complete binary extent is valid, but decoding the custom scalar would fail.
            // A resource rejection therefore establishes that budget preflight runs first.
            let result = import_bytes_result("custom-f64-budget.ply", &bytes, limits)?;
            let failure = expected_error(result)?;

            assert_resource_limit(&failure, resource);
            let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
                panic!("resource rejection must publish a failure terminal")
            };
            let expected_class = match failure.cause() {
                ImportCause::Scope(abort) => match abort.cause() {
                    ScopeAbortCause::Ledger(_) => ImportTerminalFailureClass::Ledger,
                    ScopeAbortCause::ResourceLimit { .. } => ImportTerminalFailureClass::Requested,
                    cause => panic!("unexpected limit cause: {cause:?}"),
                },
                cause => panic!("unexpected limit cause: {cause:?}"),
            };
            assert_eq!(terminal.class(), expected_class);
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
            assert!(failure.diagnostics().is_empty());
        }
    }
    Ok(())
}

#[test]
fn rejects_non_finite_custom_f64_scalars() -> TestResult<()> {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let values = [value, F64_VALUES[1], F64_VALUES[2]];
        for (source, bytes) in [
            ("non-finite-ascii.ply", ascii_custom_scalars(&values)),
            ("non-finite-le.ply", binary_custom_scalars(true, &values)),
            ("non-finite-be.ply", binary_custom_scalars(false, &values)),
        ] {
            let result = import_bytes_result(source, &bytes, ResourceLimits::default())?;
            let failure = expected_error(result)?;
            let envelope = expected_ply_failure(&failure, PlyFailureCode::InvalidScalar);

            assert_eq!(
                ply_string_detail(envelope, "reason"),
                Some("PLY property contains a non-finite value")
            );
            assert_eq!(ply_string_detail(envelope, "context"), Some("scalar"));
            assert!(failure.diagnostics().is_empty());
            assert!(failure.loss_evidence().is_empty());
            assert_requested_terminal_failure(&failure);
        }
    }
    Ok(())
}
