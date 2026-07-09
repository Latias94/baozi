mod common;

use baozi_core::{BaoziError, Color, Result};
use baozi_import::ImportOptions;
use baozi_test_support::SceneSnapshot;
use common::{BinaryFacet, binary_stl, expected_error, import_bytes, import_bytes_result};

#[test]
fn imports_one_triangle_binary_stl() -> Result<()> {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let (scene, diagnostics) = import_bytes("triangle.stl", &bytes)?;
    let snapshot = SceneSnapshot::from_scene_with_diagnostics(&scene, &diagnostics);
    let text = snapshot.as_str();

    assert!(text.contains("scene nodes=2 meshes=1 materials=1"));
    assert!(text.contains("mesh 0 name=<STL_BINARY> topology=Triangles"));
    assert!(text.contains("vertices=3 indices=3 faces=<fixed> material=0"));
    assert!(text.contains("metadata=[stl.source,stl.storage]"));
    assert!(text.contains("face_vertex_counts=[]"));
    assert!(text.contains("positions[1]=(1.000000,0.000000,0.000000)"));
    assert!(text.contains("normals[0]=(0.000000,0.000000,1.000000)"));
    assert!(text.contains("diagnostics count=0"));
    Ok(())
}

#[test]
fn zero_facet_binary_stl_is_parse_error() -> Result<()> {
    let bytes = binary_stl(b"binary", &[]);
    let (result, _) = import_bytes_result("empty.stl", &bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn declared_count_must_match_file_size() -> Result<()> {
    let mut bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    bytes.truncate(bytes.len() - 1);
    let (result, _) = import_bytes_result("truncated.stl", &bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn non_finite_binary_float_fails_validation() -> Result<()> {
    let mut facet = BinaryFacet::unit_triangle();
    facet.vertices[1][0] = f32::NAN;
    let bytes = binary_stl(b"binary", &[facet]);
    let (result, _) = import_bytes_result("nan.stl", &bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::InvalidScene { .. }));
    Ok(())
}

#[test]
fn face_limit_is_checked_before_allocation() -> Result<()> {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut options = ImportOptions::memory();
    options.limits.max_faces = 0;
    let (result, _) = import_bytes_result("limited.stl", &bytes, options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded { limit: "max_faces" }
    ));
    Ok(())
}

#[test]
fn materialise_header_color_maps_to_default_material() -> Result<()> {
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[BinaryFacet::unit_triangle()]);
    let (scene, _) = import_bytes("materialise-color.stl", &bytes)?;
    let color = scene.materials[0].base_color;

    assert_color_close(
        color,
        Color::linear_rgba(32.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0, 1.0),
    );
    Ok(())
}

#[test]
fn binary_facet_colors_expand_to_vertex_color_channel() -> Result<()> {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | (31 << 10);
    let bytes = binary_stl(b"binary", &[facet]);
    let (scene, _) = import_bytes("colored.stl", &bytes)?;

    assert_eq!(scene.meshes[0].colors.len(), 1);
    assert_eq!(scene.meshes[0].colors[0].len(), 3);
    assert_color_close(
        scene.meshes[0].colors[0][0],
        Color::linear_rgba(1.0, 0.0, 0.0, 1.0),
    );
    Ok(())
}

#[test]
fn materialise_facet_color_uses_materialise_channel_order() -> Result<()> {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | 31;
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[facet]);
    let (scene, _) = import_bytes("materialise-facet-color.stl", &bytes)?;

    assert_eq!(scene.meshes[0].colors.len(), 1);
    assert_color_close(
        scene.meshes[0].colors[0][0],
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
