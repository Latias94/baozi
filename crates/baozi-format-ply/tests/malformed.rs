mod common;

use baozi_format_ply::PlyFailureCode;
use baozi_import::{DiagnosticDetailValue, ImportDiagnostic, ImportProgressPhase, ResourceLimits};
use common::{
    TestResult, ascii_triangle, assert_resource_limit, expected_error, expected_ply_failure,
    expected_report, import_bytes_result, ply_string_detail, ply_unsigned_detail, scene_snapshot,
    snapshot_line, snapshot_line_containing,
};

fn memory_limits() -> ResourceLimits {
    ResourceLimits::default()
}

fn diagnostic_has_code(diagnostic: &ImportDiagnostic, code: &str) -> bool {
    diagnostic.code().as_str() == code
}

#[test]
fn missing_end_header_is_parse_error() -> TestResult<()> {
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\n";
    let result = import_bytes_result("missing-header.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidHeader);
    assert_eq!(envelope.details().len(), 1);
    Ok(())
}

#[test]
fn face_index_out_of_range_is_parse_error() -> TestResult<()> {
    let bytes = std::str::from_utf8(ascii_triangle())
        .expect("fixture is utf-8")
        .replace("3 0 1 2", "3 0 1 3");
    let result = import_bytes_result("bad-index.ply", bytes.as_bytes(), memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidTopology);
    assert_eq!(envelope.details().len(), 1);
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_header() -> TestResult<()> {
    let mut limits = memory_limits();
    limits.max_vertices = 2;
    let result = import_bytes_result("vertex-limit.ply", ascii_triangle(), limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_vertices");
    Ok(())
}

#[test]
fn primitive_limit_is_enforced_before_publication() -> TestResult<()> {
    let mut limits = memory_limits();
    limits.max_primitives = 0;
    let result = import_bytes_result("primitive-limit.ply", ascii_triangle(), limits)?;
    let error = expected_error(result)?;

    assert!(error.diagnostics().is_empty());
    assert_resource_limit(&error, "max_primitives");
    Ok(())
}

#[test]
fn unsupported_format_version_is_parse_error() -> TestResult<()> {
    let bytes = b"ply
format ascii 2.0
element vertex 1
property float x
property float y
property float z
end_header
0 0 0
";
    let result = import_bytes_result("version-2.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::UnsupportedEncoding);
    assert_eq!(
        ply_string_detail(envelope, "context"),
        Some("format-version")
    );
    assert_eq!(ply_unsigned_detail(envelope, "byte_offset"), Some(4));
    assert_eq!(envelope.details().len(), 3);
    Ok(())
}

#[test]
fn duplicate_vertex_element_is_parse_error() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
element vertex 1
property float x
property float y
property float z
end_header
0 0 0
1 1 1
";
    let result = import_bytes_result("duplicate-vertex.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidSchema);
    assert!(ply_unsigned_detail(envelope, "byte_offset").is_some());
    assert_eq!(envelope.details().len(), 2);
    Ok(())
}

#[test]
fn duplicate_face_element_is_parse_error() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2
3 0 1 2
";
    let result = import_bytes_result("duplicate-face.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidSchema);
    assert!(ply_unsigned_detail(envelope, "byte_offset").is_some());
    assert_eq!(envelope.details().len(), 2);
    Ok(())
}

#[test]
fn duplicate_face_topology_aliases_are_parse_error() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
property list uchar uint vertex_index
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2 3 0 1 2
";
    let result = import_bytes_result("duplicate-topology.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidSchema);
    assert_eq!(envelope.details().len(), 1);
    Ok(())
}

#[test]
fn zero_count_face_with_topology_schema_preserves_point_cloud() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
element face 0
property list uchar uint vertex_indices
end_header
0 0 0
";
    let scene = import_bytes_result("zero-face.ply", bytes, memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert!(diagnostics.is_empty());
    assert_eq!(snapshot.primitive_index_count(), 0);
    assert_eq!(snapshot.polygon_boundary_count(), 0);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("topology=points vertices=1"));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("indices(absent,total=0)=[]"));
    Ok(())
}

#[test]
fn custom_attribute_cell_limit_is_enforced_before_body_read() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float temperature
end_header
0 0 0 1
1 0 0 2
0 1 0 3
";
    let mut limits = memory_limits();
    limits.max_total_vertex_attribute_cells = 5;
    let result = import_bytes_result("custom-cell-limit.ply", bytes, limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_total_vertex_attribute_cells");
    Ok(())
}

#[test]
fn custom_attribute_stream_limit_is_enforced_before_allocation() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property float temperature
property float density
end_header
0 0 0 1 2
";
    let mut limits = memory_limits();
    limits.max_vertex_attribute_streams_per_mesh = 2;
    let result = import_bytes_result("custom-stream-limit.ply", bytes, limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_vertex_attribute_streams_per_mesh");
    Ok(())
}

#[test]
fn custom_attribute_slots_preserve_values_across_canonical_order() -> TestResult<()> {
    const ROW_COUNT: usize = 64;

    let mut document = format!(
        "ply\nformat ascii 1.0\nelement vertex {ROW_COUNT}\n\
         property float x\nproperty float y\nproperty float z\n\
         property float temperature\nproperty int sample_id\nproperty uint flags\nend_header\n"
    );
    for row in 0..ROW_COUNT {
        let signed = -(row as i32);
        document.push_str(&format!("{row} 0 0 {} {signed} {row}\n", row as f32 + 0.5));
    }

    let scene = import_bytes_result("custom-slots.ply", document.as_bytes(), memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert!(diagnostics.is_empty());
    assert_eq!(snapshot.attribute_count(), 4);
    assert_eq!(snapshot.f32_scalar_count(), (ROW_COUNT * 4) as u64);
    assert_eq!(snapshot.i32_scalar_count(), ROW_COUNT as u64);
    assert_eq!(snapshot.u32_scalar_count(), ROW_COUNT as u64);

    let temperatures = snapshot_line_containing(&snapshot, "semantic=ply:temperature");
    assert!(
        temperatures
            .contains("semantic=ply:temperature semantic_set=0 domain=vertex scalar=f32 lanes=1")
    );
    let expected_temperatures = (0..ROW_COUNT)
        .map(|row| format!("0x{:08x}", (row as f32 + 0.5).to_bits()))
        .collect::<Vec<_>>()
        .join(",");
    assert!(temperatures.contains(&format!(
        "values(present,total={ROW_COUNT})=[{expected_temperatures}]"
    )));

    let sample_ids = snapshot_line_containing(&snapshot, "semantic=ply:sample_id");
    assert!(
        sample_ids
            .contains("semantic=ply:sample_id semantic_set=0 domain=vertex scalar=i32 lanes=1")
    );
    let expected_sample_ids = (0..ROW_COUNT)
        .map(|row| (-(row as i32)).to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert!(sample_ids.contains(&format!(
        "values(present,total={ROW_COUNT})=[{expected_sample_ids}]"
    )));

    let flags = snapshot_line_containing(&snapshot, "semantic=ply:flags");
    assert!(flags.contains("semantic=ply:flags semantic_set=0 domain=vertex scalar=u32 lanes=1"));
    let expected_flags = (0..ROW_COUNT)
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert!(flags.contains(&format!(
        "values(present,total={ROW_COUNT})=[{expected_flags}]"
    )));
    Ok(())
}

#[test]
fn topology_cell_limit_is_checked_before_face_indices_are_stored() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2
";

    let mut exact_limits = memory_limits();
    exact_limits.max_total_topology_cells = 3;
    let result = import_bytes_result("topology-exact.ply", bytes, exact_limits)?;
    let report = expected_report(result)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;
    assert!(diagnostics.is_empty());
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.polygon_boundary_count(), 0);

    let mut excess_limits = memory_limits();
    excess_limits.max_total_topology_cells = 2;
    let result = import_bytes_result("topology-excess.ply", bytes, excess_limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();
    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_total_topology_cells");
    Ok(())
}

#[test]
fn polygon_topology_limit_counts_indices_and_face_cardinality() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
4 0 1 2 3
";

    let mut exact_limits = memory_limits();
    exact_limits.max_total_topology_cells = 5;
    let result = import_bytes_result("polygon-topology-exact.ply", bytes, exact_limits)?;
    let report = expected_report(result)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;
    assert!(diagnostics.is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(
        snapshot_line(&snapshot, "primitive[0]")
            .contains("polygon_boundaries(present,total=1)=[4] indices(present,total=4)=[0,1,2,3]")
    );

    let malformed_excess = std::str::from_utf8(bytes)
        .expect("fixture is valid UTF-8")
        .replace("4 0 1 2 3", "4 invalid 1 2 3");
    let mut excess_limits = memory_limits();
    excess_limits.max_total_topology_cells = 4;
    let result = import_bytes_result(
        "polygon-topology-excess.ply",
        malformed_excess.as_bytes(),
        excess_limits,
    )?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();
    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_total_topology_cells");
    Ok(())
}

#[test]
fn mixed_topology_limit_is_order_independent() -> TestResult<()> {
    let header = "ply\nformat ascii 1.0\nelement vertex 4\n\
                  property float x\nproperty float y\nproperty float z\n\
                  element face 2\nproperty list uchar uint vertex_indices\nend_header\n\
                  0 0 0\n1 0 0\n1 1 0\n0 1 0\n";
    for (label, first, second, invalid_second, expected_counts) in [
        (
            "triangle-quad",
            "3 0 1 2",
            "4 0 1 2 3",
            "4 invalid 1 2 3",
            [3_u32, 4],
        ),
        (
            "quad-triangle",
            "4 0 1 2 3",
            "3 0 1 2",
            "3 invalid 1 2",
            [4, 3],
        ),
    ] {
        let document = format!("{header}{first}\n{second}\n");
        let mut exact_limits = memory_limits();
        exact_limits.max_total_topology_cells = 9;
        let scene = import_bytes_result(
            &format!("mixed-{label}-exact.ply"),
            document.as_bytes(),
            exact_limits,
        )?;
        let report = expected_report(scene)?;
        let diagnostics = report.diagnostics();
        let snapshot = scene_snapshot(&report)?;
        assert!(diagnostics.is_empty());
        assert_eq!(snapshot.primitive_index_count(), 7);
        assert_eq!(snapshot.polygon_boundary_count(), 2);
        let expected_boundaries = expected_counts
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        assert!(snapshot_line(&snapshot, "primitive[0]").contains(&format!(
            "polygon_boundaries(present,total=2)=[{expected_boundaries}]"
        )));

        let malformed_excess = format!("{header}{first}\n{invalid_second}\n");
        let mut excess_limits = memory_limits();
        excess_limits.max_total_topology_cells = 8;
        let result = import_bytes_result(
            &format!("mixed-{label}-excess.ply"),
            malformed_excess.as_bytes(),
            excess_limits,
        )?;
        let error = expected_error(result)?;
        let diagnostics = error.diagnostics();
        assert!(diagnostics.is_empty());
        assert_resource_limit(&error, "max_total_topology_cells");
    }
    Ok(())
}

#[test]
fn binary_polygon_topology_limit_precedes_item_reads() -> TestResult<()> {
    let mut exact_bytes = b"ply
format binary_little_endian 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
"
    .to_vec();
    for position in [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        for value in position {
            exact_bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    exact_bytes.push(4);
    let topology_start = exact_bytes.len();
    for index in [0u32, 1, 2, 3] {
        exact_bytes.extend_from_slice(&index.to_le_bytes());
    }

    let mut exact_limits = memory_limits();
    exact_limits.max_total_topology_cells = 5;
    let scene = import_bytes_result("binary-polygon-exact.ply", &exact_bytes, exact_limits)?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;
    assert!(diagnostics.is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(
        snapshot_line(&snapshot, "primitive[0]")
            .contains("polygon_boundaries(present,total=1)=[4] indices(present,total=4)=[0,1,2,3]")
    );

    let truncated_excess = &exact_bytes[..topology_start];
    let mut excess_limits = memory_limits();
    excess_limits.max_total_topology_cells = 4;
    let result = import_bytes_result("binary-polygon-excess.ply", truncated_excess, excess_limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();
    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_total_topology_cells");
    Ok(())
}

#[test]
fn binary_unknown_element_without_properties_is_rejected_before_rows() -> TestResult<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 4294967295
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let result = import_bytes_result("zero-width-records.ply", &bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidSchema);
    assert_eq!(
        ply_string_detail(envelope, "context"),
        Some("zero-width-binary-element")
    );
    assert_eq!(envelope.details().len(), 2);
    Ok(())
}

#[test]
fn binary_element_records_are_preflighted_against_the_aggregate_payload() -> TestResult<()> {
    let bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar first
element metadata_extra 2
property uchar second
element vertex 1
property float x
property float y
property float z
end_header
\x01\x02\x03\x04
";

    let result = import_bytes_result("aggregate-records.ply", bytes, memory_limits())?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::TruncatedPayload);
    assert!(ply_unsigned_detail(envelope, "byte_offset").is_some());
    assert_eq!(envelope.details().len(), 2);
    Ok(())
}

#[test]
fn aggregate_binary_record_limit_is_checked_before_body_iteration() -> TestResult<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar first
element metadata_extra 1
property uchar second
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.extend_from_slice(&[1, 2, 3]);
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut limits = memory_limits();
    limits.max_records = 3;

    let result = import_bytes_result("record-limit.ply", &bytes, limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_records");
    Ok(())
}

#[test]
fn aggregate_ascii_record_limit_is_checked_before_body_iteration() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element metadata 2
element vertex 1
property float x
property float y
property float z
end_header
";
    let mut limits = memory_limits();
    limits.max_records = 2;

    let result = import_bytes_result("ascii-record-limit.ply", bytes, limits)?;
    let error = expected_error(result)?;
    let diagnostics = error.diagnostics();

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_records");
    Ok(())
}

#[test]
fn exact_ascii_record_budget_allows_zero_property_rows_and_advances_lines() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element metadata 2
element vertex 1
property float x
property float y
property float z
end_header


1 2 3
";
    let mut limits = memory_limits();
    limits.max_records = 3;

    let scene = import_bytes_result("ascii-record-exact.ply", bytes, limits)?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 1);
    assert!(
        snapshot_line(&snapshot, "attribute[0]")
            .contains("values(present,total=3)=[0x3f800000,0x40000000,0x40400000]")
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_has_code(diagnostic, "ply.element_ignored"))
            .count(),
        1
    );
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic_has_code(diagnostic, "ply.element_ignored"))
        .expect("ignored element diagnostic must be retained");
    assert_eq!(diagnostic.phase(), ImportProgressPhase::Decode);
    assert_eq!(diagnostic.source().as_str(), "ascii-record-exact.ply");
    let location = diagnostic
        .line_column()
        .expect("ignored element diagnostic must derive a source location");
    assert_eq!((location.line(), location.column()), (3, 1));
    let range = diagnostic
        .byte_range()
        .expect("ignored element diagnostic must retain its byte range");
    assert_eq!(range.start(), range.end());
    assert!(diagnostic.details().iter().any(|detail| {
        detail.key() == "format"
            && matches!(detail.value(), DiagnosticDetailValue::String(value) if value == "ply")
    }));
    Ok(())
}

#[test]
fn exact_binary_record_budget_is_allowed() -> TestResult<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 1
property uchar value
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.push(7);
    for value in [1.0f32, 2.0, 3.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut limits = memory_limits();
    limits.max_records = 2;

    let scene = import_bytes_result("binary-record-exact.ply", &bytes, limits)?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 1);
    assert!(
        snapshot_line(&snapshot, "attribute[0]")
            .contains("values(present,total=3)=[0x3f800000,0x40000000,0x40400000]")
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_has_code(diagnostic, "ply.element_ignored"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn unrepresentable_record_envelope_is_rejected_before_session_start() -> TestResult<()> {
    let bytes = format!(
        "ply\nformat ascii 1.0\nelement metadata {}\nproperty uchar value\nelement metadata_extra 1\nproperty uchar value\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
        usize::MAX
    );
    let mut limits = memory_limits();
    limits.max_records = usize::MAX;

    let result = import_bytes_result("record-overflow.ply", bytes.as_bytes(), limits)?;
    let error = result.expect_err("an unrepresentable retained-store envelope must fail closed");

    assert!(matches!(
        error,
        baozi_import::ImportRunError::Initialization {
            source: baozi_import::ImportInitializationError::RetainedStore {
                code: "retained-store-bootstrap-rejected"
            }
        }
    ));
    Ok(())
}

#[test]
fn zero_width_record_fuzz_corpus_terminates_with_expected_outcomes() -> TestResult<()> {
    let ascii = include_bytes!("../../../fuzz/corpus/ply_import/ascii_zero_width_records.ply");
    let ascii_result = import_bytes_result("ascii-zero-width.ply", ascii, memory_limits())?;
    let report = expected_report(ascii_result)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=1"));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_has_code(diagnostic, "ply.element_ignored"))
            .count(),
        1
    );

    let binary = include_bytes!("../../../fuzz/corpus/ply_import/binary_zero_width_records.ply");
    let binary_result = import_bytes_result("binary-zero-width.ply", binary, memory_limits())?;
    let error = expected_error(binary_result)?;
    let diagnostics = error.diagnostics();
    assert!(diagnostics.is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidSchema);
    assert_eq!(
        ply_string_detail(envelope, "context"),
        Some("zero-width-binary-element")
    );
    assert_eq!(envelope.details().len(), 2);
    Ok(())
}

#[test]
fn ignored_binary_element_emits_one_diagnostic_per_element() -> TestResult<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar value
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.extend_from_slice(&[1, 2]);
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let scene = import_bytes_result("bounded-diagnostics.ply", &bytes, memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=1"));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_has_code(diagnostic, "ply.element_ignored"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn partial_normal_properties_emit_diagnostics_and_omit_channel() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float nx
element face 1
property list uchar uint vertex_indices
end_header
0 0 0 0
1 0 0 0
0 1 0 0
3 0 1 2
";
    let scene = import_bytes_result("partial-normal.ply", bytes, memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 1);
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_has_code(diagnostic, "ply.partial_normals_ignored"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn vertex_schema_warnings_are_emitted_once_instead_of_once_per_row() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float nx
property float s
property uchar red
property uchar green
property list uchar int neighbors
end_header
0 0 0 1 0 255 128 0
1 0 0 1 0 255 128 0
0 1 0 1 0 255 128 0
";

    let scene = import_bytes_result("vertex-schema-warnings.ply", bytes, memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 1);
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=3"));
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert!(!snapshot.preview().contains("semantic=texcoord"));
    assert!(!snapshot.preview().contains("semantic=color"));
    for code in [
        "ply.vertex_list_property_ignored",
        "ply.partial_normals_ignored",
        "ply.partial_texcoords_ignored",
        "ply.partial_colors_ignored",
    ] {
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic_has_code(diagnostic, code))
                .count(),
            1,
            "diagnostic {code} must be emitted once per vertex schema"
        );
    }
    assert_eq!(diagnostics.len(), 4);
    Ok(())
}

#[test]
fn face_ignored_property_warnings_are_emitted_once_per_schema() -> TestResult<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar uint vertex_indices
property int material_id
property list uchar float texcoord
end_header
0 0 0
1 0 0
1 1 0
0 1 0
3 0 1 2 7 6 0 0 1 0 1 1
3 0 2 3 8 6 0 0 1 1 0 1
";

    let scene = import_bytes_result("face-schema-warnings.ply", bytes, memory_limits())?;
    let report = expected_report(scene)?;
    let diagnostics = report.diagnostics();
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.primitive_index_count(), 6);
    assert_eq!(snapshot.polygon_boundary_count(), 0);
    assert!(
        snapshot_line(&snapshot, "primitive[0]")
            .contains("topology=triangle_list vertices=4 corners=6 faces=2")
    );
    for code in [
        "ply.face_scalar_property_ignored",
        "ply.face_list_property_ignored",
    ] {
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic_has_code(diagnostic, code))
                .count(),
            1,
            "diagnostic {code} must be emitted once per face schema"
        );
    }
    assert_eq!(diagnostics.len(), 2);
    Ok(())
}
