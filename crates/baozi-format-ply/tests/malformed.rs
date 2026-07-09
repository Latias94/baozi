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
fn unsupported_format_version_is_parse_error() -> Result<()> {
    let bytes = b"ply
format ascii 2.0
element vertex 1
property float x
property float y
property float z
end_header
0 0 0
";
    let (result, diagnostics) =
        import_bytes_result("version-2.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("unsupported PLY version"));
    Ok(())
}

#[test]
fn duplicate_vertex_element_is_parse_error() -> Result<()> {
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
    let (result, diagnostics) =
        import_bytes_result("duplicate-vertex.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("multiple PLY vertex elements"));
    Ok(())
}

#[test]
fn duplicate_face_element_is_parse_error() -> Result<()> {
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
    let (result, diagnostics) =
        import_bytes_result("duplicate-face.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("multiple PLY face elements"));
    Ok(())
}

#[test]
fn custom_attribute_cell_limit_is_enforced_before_body_read() -> Result<()> {
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
    let mut options = ImportOptions::memory();
    options.limits.max_vertex_attribute_cells = 2;
    let (result, diagnostics) = import_bytes_result("custom-cell-limit.ply", bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertex_attribute_cells"
        }
    ));
    Ok(())
}

#[test]
fn custom_attribute_stream_limit_is_enforced_before_allocation() -> Result<()> {
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
    let mut options = ImportOptions::memory();
    options.limits.max_vertex_attribute_streams = 1;
    let (result, diagnostics) = import_bytes_result("custom-stream-limit.ply", bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertex_attribute_streams"
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
