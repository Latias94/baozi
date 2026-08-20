mod common;

use baozi_format_stl::StlFailureCode;
use baozi_import::{
    ImportCause, ImportDimension, ImportTerminalCause, ImportTerminalFailureClass, ResourceLimits,
    ScopeAbortCause,
};
use common::{
    ascii_triangle, expected_stl_failure, import_bytes, import_bytes_with_limits, scene_snapshot,
    snapshot_line, stl_string_detail, stl_unsigned_detail,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn imports_one_triangle_ascii_stl_through_session() -> TestResult {
    let bytes = ascii_triangle("triangle");
    let report = import_bytes("triangle.stl", bytes.as_bytes())?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.material_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 2);
    assert_eq!(snapshot.mesh_instance_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 18);
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "attribute[0]").contains(concat!(
        "semantic=position semantic_set=0 domain=vertex scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3"
    )));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("indices(present,total=3)=[0,1,2]"));
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::ScratchBytes)
            .expect("STL normalization storage")
            .peak_live_bytes()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .expect("STL scene storage")
            .peak_live_bytes()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::DecodeWork)
            .expect("STL decode work")
            .consumed()
            >= bytes.len() as u64
    );
    for (dimension, consumed) in [
        (ImportDimension::Solids, 1),
        (ImportDimension::Primitives, 1),
        (ImportDimension::AttributeStreams, 2),
        (ImportDimension::GraphNodes, 1),
        (ImportDimension::GraphEdges, 1),
    ] {
        assert_eq!(
            report
                .ledger()
                .dimension(dimension)
                .expect("STL work dimension")
                .consumed(),
            consumed
        );
    }
    Ok(())
}

#[test]
fn imports_multiple_ascii_solids_as_multiple_meshes() -> TestResult {
    let bytes = format!("{}{}", ascii_triangle("left"), ascii_triangle("right"));
    let report = import_bytes("multi.stl", bytes.as_bytes())?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.root_count(), 2);
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=left "));
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=left "));
    assert!(snapshot_line(&snapshot, "node[1]").contains(" name=right "));
    assert!(snapshot_line(&snapshot, "logical_mesh[1]").contains(" name=right "));
    Ok(())
}

#[test]
fn empty_ascii_solid_preserves_diagnostic_and_typed_failure() {
    let failure = import_bytes_with_limits(
        "empty-ascii.stl",
        b"\n  solid empty\nendsolid empty\n",
        ResourceLimits::default(),
    )
    .expect_err("empty STL must fail");

    assert_eq!(
        expected_stl_failure(&failure).code(),
        StlFailureCode::EmptyGeometry.as_str()
    );
    assert!(matches!(failure.cause(), ImportCause::Format(_)));
    assert_eq!(failure.diagnostics().len(), 1);
    assert_eq!(failure.diagnostics()[0].code().as_str(), "stl.empty_solid");
    assert_eq!(
        failure.diagnostics()[0].source().as_str(),
        "empty-ascii.stl"
    );
    assert_eq!(
        failure.diagnostics()[0]
            .line_column()
            .expect("empty solid location")
            .line(),
        2
    );
    assert_eq!(
        failure.diagnostics()[0]
            .line_column()
            .expect("empty solid location")
            .column(),
        3
    );
    let byte_range = failure.diagnostics()[0]
        .byte_range()
        .expect("empty solid byte range");
    assert_eq!((byte_range.start(), byte_range.end()), (3, 8));
    assert_eq!(failure.diagnostics()[0].details()[0].key(), "facet_count");
}

#[test]
fn malformed_ascii_preserves_stl_owned_location() {
    let bytes = b"solid bad\nfacet normal 0 0 nope\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid bad\n";
    let failure = import_bytes_with_limits("invalid-float.stl", bytes, ResourceLimits::default())
        .expect_err("invalid float must fail");
    let envelope = expected_stl_failure(&failure);

    assert_eq!(envelope.format().as_str(), "org.baozi.stl");
    assert_eq!(envelope.code(), StlFailureCode::InvalidNumber.as_str());
    assert_eq!(stl_unsigned_detail(envelope, "line"), Some(2));
    assert_eq!(stl_unsigned_detail(envelope, "column"), Some(18));
    assert_eq!(stl_string_detail(envelope, "reason"), Some("invalid-float"));
}

#[test]
fn ascii_numeric_grammar_uses_dot_and_rejects_locale_decimal_comma() {
    let dotted = ascii_triangle("dotted").replace("vertex 1 0 0", "vertex 1.5 0 0");
    let report =
        import_bytes_with_limits("dotted.stl", dotted.as_bytes(), ResourceLimits::default())
            .expect("Rust ASCII float grammar must accept a decimal point");
    let snapshot = scene_snapshot(&report).expect("dotted STL semantic snapshot");
    assert!(snapshot_line(&snapshot, "attribute[0]").contains(concat!(
        "values(present,total=9)=[",
        "0x00000000,0x00000000,0x00000000,0x3fc00000"
    )));

    let comma = ascii_triangle("comma").replace("vertex 1 0 0", "vertex 1,5 0 0");
    let failure =
        import_bytes_with_limits("comma.stl", comma.as_bytes(), ResourceLimits::default())
            .expect_err("Rust ASCII float grammar must reject a locale decimal comma");
    assert_eq!(
        expected_stl_failure(&failure).code(),
        StlFailureCode::InvalidNumber.as_str()
    );
}

#[test]
fn facet_with_too_few_vertices_is_typed_format_failure() {
    let bytes = b"solid bad\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nendloop\nendfacet\nendsolid bad\n";
    let failure = import_bytes_with_limits("bad-vertices.stl", bytes, ResourceLimits::default())
        .expect_err("short facet must fail");

    assert_eq!(
        expected_stl_failure(&failure).code(),
        StlFailureCode::InvalidStructure.as_str()
    );
}

#[test]
fn missing_endsolid_is_typed_format_failure() {
    let bytes = b"solid missing\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\n";
    let failure =
        import_bytes_with_limits("missing-endsolid.stl", bytes, ResourceLimits::default())
            .expect_err("unterminated solid must fail");

    let envelope = expected_stl_failure(&failure);
    assert_eq!(
        stl_string_detail(envelope, "reason"),
        Some("ended-before-endsolid")
    );
}

#[test]
fn typed_line_and_token_limits_are_session_owned() {
    let bytes = ascii_triangle("triangle");
    for (resource, line_limit, token_limit) in [
        ("line-bytes", Some(5), None),
        ("token-bytes", None, Some(3)),
    ] {
        let mut limits = ResourceLimits::default();
        if let Some(limit) = line_limit {
            limits.max_line_bytes = limit;
        }
        if let Some(limit) = token_limit {
            limits.max_token_bytes = limit;
        }
        let failure = import_bytes_with_limits("limited.stl", bytes.as_bytes(), limits)
            .expect_err("parser work limit must fail");
        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("expected scope-owned resource failure")
        };
        assert!(matches!(
            abort.cause(),
            ScopeAbortCause::ResourceLimit { resource: actual, .. } if actual == &resource
        ));
    }
}

#[test]
fn decode_work_is_admitted_before_ascii_reclassification() {
    let bytes = ascii_triangle("triangle");
    let mut limits = ResourceLimits::default();
    limits.max_decode_work = bytes.len() as u64 - 1;
    let failure = import_bytes_with_limits("decode-limit.stl", bytes.as_bytes(), limits)
        .expect_err("decode work limit must fail before parser scanning");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    let decode = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .expect("decode dimension");
    assert_eq!(decode.attempted(), bytes.len() as u64);
    assert_eq!(decode.consumed(), 0);
}

#[test]
fn face_limit_is_charged_before_geometry_growth() {
    let bytes = ascii_triangle("triangle");
    let mut limits = ResourceLimits::default();
    limits.max_faces = 0;
    let failure = import_bytes_with_limits("face-limit.stl", bytes.as_bytes(), limits)
        .expect_err("face limit must fail");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    let faces = failure
        .ledger()
        .dimension(ImportDimension::Faces)
        .expect("face dimension");
    assert_eq!(faces.attempted(), 1);
    assert_eq!(faces.consumed(), 0);
}

#[test]
fn topology_cells_are_charged_before_index_allocation() -> TestResult {
    let bytes = ascii_triangle("triangle");
    let mut exact = ResourceLimits::default();
    exact.max_total_topology_cells = 3;
    let report = import_bytes_with_limits("topology-exact.stl", bytes.as_bytes(), exact)
        .expect("the exact topology limit must succeed");
    assert_eq!(scene_snapshot(&report)?.primitive_index_count(), 3);

    let mut excess = ResourceLimits::default();
    excess.max_total_topology_cells = 2;
    let failure = import_bytes_with_limits("topology-excess.stl", bytes.as_bytes(), excess)
        .expect_err("topology limit must fail");
    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
    Ok(())
}

#[test]
fn solid_limit_is_checked_before_second_name_allocation() {
    let bytes = format!("{}{}", ascii_triangle("left"), ascii_triangle("right"));
    let mut limits = ResourceLimits::default();
    limits.max_solids = 1;
    let failure = import_bytes_with_limits("solid-limit.stl", bytes.as_bytes(), limits)
        .expect_err("solid limit must fail");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
}

#[test]
fn live_byte_limit_is_exact_across_parse_and_scene_handoff() -> TestResult {
    let bytes = ascii_triangle("bounded");
    let source = "live-limit.stl";
    let baseline = import_bytes(source, bytes.as_bytes())?;
    let exact_peak = baseline.ledger().peak_live_bytes();
    assert!(exact_peak > bytes.len() as u64);
    assert_eq!(baseline.ledger().current_live_bytes(), 0);

    let mut exact = ResourceLimits::default();
    exact.max_live_bytes = exact_peak;
    let exact_report = import_bytes_with_limits(source, bytes.as_bytes(), exact)
        .expect("the exact live-byte limit must succeed");
    assert_eq!(exact_report.ledger().peak_live_bytes(), exact_peak);

    let excess_limit = exact_peak - 1;
    let mut excess = ResourceLimits::default();
    excess.max_live_bytes = excess_limit;
    let failure = import_bytes_with_limits(source, bytes.as_bytes(), excess)
        .expect_err("one byte below the observed peak must fail");
    let ImportCause::OperationTerminal(terminal) = failure.cause() else {
        panic!(
            "expected live-byte terminal rejection, got {:?}",
            failure.cause()
        )
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    assert_eq!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(*terminal)
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}
