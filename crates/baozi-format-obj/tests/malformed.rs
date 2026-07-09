mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use common::{expected_error, import_bytes_result};

#[test]
fn empty_obj_is_parse_error() -> Result<()> {
    let (result, diagnostics) = import_bytes_result("empty.obj", b"", ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(diagnostics.is_empty());
    Ok(())
}

#[test]
fn non_utf8_obj_is_parse_error() -> Result<()> {
    let (result, diagnostics) = import_bytes_result(
        "bad-utf8.obj",
        &[0xff, 0xfe, b'\n'],
        ImportOptions::memory(),
    )?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(diagnostics.is_empty());
    Ok(())
}

#[test]
fn face_with_too_few_vertices_is_parse_error() -> Result<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nf 1 2\n";

    let (result, _) = import_bytes_result("few-face-vertices.obj", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn face_index_zero_is_parse_error() -> Result<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n";

    let (result, _) = import_bytes_result("zero-index.obj", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn face_index_out_of_range_is_parse_error() -> Result<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n";

    let (result, _) = import_bytes_result("bad-index.obj", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn unsupported_obj_statement_warns_and_geometry_imports() -> Result<()> {
    let bytes = b"curv 0 1 1 2\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (result, diagnostics) =
        import_bytes_result("unsupported.obj", bytes, ImportOptions::memory())?;
    let scene = result?;

    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.0, "obj.unsupported_statement");
    Ok(())
}
