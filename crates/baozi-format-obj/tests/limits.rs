mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use common::{expected_error, import_bytes_result};

#[test]
fn vertex_limit_is_enforced() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_vertices = 2;
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (result, _) = import_bytes_result("vertex-limit.obj", bytes, options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertices"
        }
    ));
    Ok(())
}

#[test]
fn face_limit_is_enforced() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_faces = 0;
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (result, _) = import_bytes_result("face-limit.obj", bytes, options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded { limit: "max_faces" }
    ));
    Ok(())
}

#[test]
fn mesh_limit_is_enforced_after_group_split() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_meshes = 1;
    let bytes = b"g left\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\ng right\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n";

    let (result, _) = import_bytes_result("mesh-limit.obj", bytes, options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_meshes"
        }
    ));
    Ok(())
}

#[test]
fn diagnostic_limit_caps_unsupported_statement_warnings() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_diagnostics = 1;
    let bytes =
        b"curv 0 1 1 2\nsurf 0 1 0 1 1 2 3 4\nparm u 0 1\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (result, diagnostics) =
        common::import_bytes_result("diagnostic-limit.obj", bytes, options)?;
    let scene = result?;

    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.0, "obj.unsupported_statement");
    Ok(())
}
