mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use common::{ascii_triangle, expected_error, import_bytes_result};

#[test]
fn missing_end_header_is_parse_error() -> Result<()> {
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\n";
    let (result, diagnostics) =
        import_bytes_result("missing-header.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("end_header"));
    Ok(())
}

#[test]
fn face_index_out_of_range_is_parse_error() -> Result<()> {
    let bytes = std::str::from_utf8(ascii_triangle())
        .expect("fixture is utf-8")
        .replace("3 0 1 2", "3 0 1 3");
    let (result, diagnostics) =
        import_bytes_result("bad-index.ply", bytes.as_bytes(), ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("missing vertex"));
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_header() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_vertices = 2;
    let (result, diagnostics) = import_bytes_result("vertex-limit.ply", ascii_triangle(), options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertices"
        }
    ));
    Ok(())
}

#[test]
fn partial_normal_properties_emit_diagnostics_and_omit_channel() -> Result<()> {
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
    let (scene, diagnostics) =
        import_bytes_result("partial-normal.ply", bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert!(scene.meshes[0].normals.is_empty());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "ply.partial_normals_ignored")
    );
    Ok(())
}
