mod common;

use baozi_core::Color;
use baozi_format_stl::StlFailureCode;
use baozi_import::{
    DetectionOutcome, ImportCause, ImportDimension, ReadConfidence, ResourceLimits,
};
use common::{
    BinaryFacet, binary_stl, expected_stl_failure, import_bytes, import_bytes_with_limits,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn imports_one_triangle_binary_stl_through_session() -> TestResult {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let report = import_bytes("triangle.stl", &bytes)?;
    let scene = report.scene();

    assert_eq!(scene.nodes().len(), 2);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].name.as_deref(), Some("<STL_BINARY>"));
    assert_eq!(scene.meshes()[0].positions[1].x, 1.0);
    assert_eq!(scene.meshes()[0].normals[0].z, 1.0);
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::ScratchBytes)
            .expect("STL binary scratch storage")
            .peak_live_bytes()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .expect("STL binary scene storage")
            .peak_live_bytes()
            > 0
    );
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::DecodeWork)
            .expect("binary decode work")
            .consumed(),
        bytes.len() as u64
    );
    Ok(())
}

#[test]
fn binary_with_solid_magic_and_arbitrary_bytes_bypasses_ascii_profile() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0xffff;
    let bytes = binary_stl(b"solid\xff\xfe\0binary", &[facet]);
    let report = import_bytes("arbitrary-bytes.stl", &bytes)?;

    let evidence = report
        .detection_evidence()
        .iter()
        .find(|evidence| evidence.format().as_str() == "org.baozi.stl")
        .expect("successful binary import must retain STL detector evidence");
    assert_eq!(
        evidence.outcome(),
        &DetectionOutcome::Match(ReadConfidence::Certain)
    );
    assert_eq!(report.scene().meshes()[0].positions.len(), 3);
    Ok(())
}

#[test]
fn zero_facet_binary_stl_is_typed_format_failure() {
    let bytes = binary_stl(b"binary", &[]);
    let failure = import_bytes_with_limits("empty.stl", &bytes, ResourceLimits::default())
        .expect_err("empty binary STL must fail");

    assert_eq!(
        expected_stl_failure(&failure).code(),
        StlFailureCode::EmptyGeometry.as_str()
    );
}

#[test]
fn declared_count_must_match_file_size() {
    let mut bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    bytes.truncate(bytes.len() - 1);
    let failure = import_bytes_with_limits("truncated.stl", &bytes, ResourceLimits::default())
        .expect_err("truncated binary STL must fail");
    assert!(matches!(
        failure.cause(),
        ImportCause::Format(_) | ImportCause::UnsupportedFormat | ImportCause::Detection(_)
    ));
}

#[test]
fn non_finite_binary_float_reaches_canonical_validation() {
    let mut facet = BinaryFacet::unit_triangle();
    facet.vertices[1][0] = f32::NAN;
    let bytes = binary_stl(b"binary", &[facet]);
    let failure = import_bytes_with_limits("nan.stl", &bytes, ResourceLimits::default())
        .expect_err("non-finite geometry must fail validation");

    assert!(matches!(failure.cause(), ImportCause::Validation(_)));
}

#[test]
fn face_limit_is_checked_before_binary_allocation() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut limits = ResourceLimits::default();
    limits.max_faces = 0;
    let failure =
        import_bytes_with_limits("limited.stl", &bytes, limits).expect_err("face limit must fail");

    assert!(matches!(failure.cause(), ImportCause::Scope(_)));
}

#[test]
fn materialise_header_color_maps_to_default_material() -> TestResult {
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[BinaryFacet::unit_triangle()]);
    let report = import_bytes("materialise-color.stl", &bytes)?;
    assert_color_close(
        report.scene().materials()[0].base_color,
        Color::linear_rgba(32.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0, 1.0),
    );
    Ok(())
}

#[test]
fn binary_facet_colors_expand_to_vertex_color_channel() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | (31 << 10);
    let bytes = binary_stl(b"binary", &[facet]);
    let report = import_bytes("colored.stl", &bytes)?;
    let colors = &report.scene().meshes()[0].colors[0];

    assert_eq!(colors.len(), 3);
    assert_color_close(colors[0], Color::linear_rgba(1.0, 0.0, 0.0, 1.0));
    Ok(())
}

#[test]
fn materialise_facet_color_uses_materialise_channel_order() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | 31;
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[facet]);
    let report = import_bytes("materialise-facet-color.stl", &bytes)?;

    assert_color_close(
        report.scene().meshes()[0].colors[0][0],
        Color::linear_rgba(1.0, 0.0, 0.0, 1.0),
    );
    Ok(())
}

fn assert_color_close(actual: Color, expected: Color) {
    let epsilon = 0.000_001;
    assert!((actual.r - expected.r).abs() < epsilon);
    assert!((actual.g - expected.g).abs() < epsilon);
    assert!((actual.b - expected.b).abs() < epsilon);
    assert!((actual.a - expected.a).abs() < epsilon);
}
