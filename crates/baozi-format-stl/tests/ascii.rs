mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use baozi_test_support::SceneSnapshot;
use common::{ascii_triangle, expected_error, import_bytes, import_bytes_result};

#[test]
fn imports_one_triangle_ascii_stl() -> Result<()> {
    let bytes = ascii_triangle("triangle");
    let (scene, diagnostics) = import_bytes("triangle.stl", bytes.as_bytes())?;
    let snapshot = SceneSnapshot::from_scene_with_diagnostics(&scene, &diagnostics);
    let text = snapshot.as_str();

    assert!(text.contains("scene nodes=2 meshes=1 materials=1"));
    assert!(text.contains("node 1 name=triangle parent=0 children=[] meshes=[0] metadata=[]"));
    assert!(text.contains("mesh 0 name=triangle topology=Triangles vertices=3 indices=3 material=0 metadata=[stl.source,stl.storage]"));
    assert!(text.contains("indices=[0,1,2]"));
    assert!(text.contains("diagnostics count=0"));
    Ok(())
}

#[test]
fn imports_multiple_ascii_solids_as_multiple_meshes() -> Result<()> {
    let bytes = format!("{}{}", ascii_triangle("left"), ascii_triangle("right"));
    let (scene, _) = import_bytes("multi.stl", bytes.as_bytes())?;

    assert_eq!(scene.meshes.len(), 2);
    assert_eq!(scene.nodes.len(), 3);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("left"));
    assert_eq!(scene.nodes[2].name.as_deref(), Some("right"));
    Ok(())
}

#[test]
fn empty_ascii_solid_emits_diagnostic_and_fails_without_meshes() -> Result<()> {
    let bytes = b"solid empty\nendsolid empty\n";
    let (result, diagnostics) =
        import_bytes_result("empty-ascii.stl", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.0, "stl.empty_solid");
    Ok(())
}

#[test]
fn facet_with_too_few_vertices_is_parse_error() -> Result<()> {
    let bytes = b"solid bad\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nendloop\nendfacet\nendsolid bad\n";
    let (result, _) = import_bytes_result("bad-vertices.stl", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn missing_endsolid_is_parse_error() -> Result<()> {
    let bytes = b"solid missing\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\n";
    let (result, _) = import_bytes_result("missing-endsolid.stl", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn invalid_float_is_parse_error() -> Result<()> {
    let bytes = b"solid bad\nfacet normal 0 0 nope\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid bad\n";
    let (result, _) = import_bytes_result("invalid-float.stl", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[test]
fn line_limit_is_enforced() -> Result<()> {
    let bytes = ascii_triangle("triangle");
    let mut options = ImportOptions::memory();
    options.limits.max_line_bytes = 5;
    let (result, _) = import_bytes_result("line-limit.stl", bytes.as_bytes(), options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_line_bytes"
        }
    ));
    Ok(())
}

#[test]
fn token_limit_is_enforced() -> Result<()> {
    let bytes = ascii_triangle("triangle");
    let mut options = ImportOptions::memory();
    options.limits.max_token_bytes = 3;
    let (result, _) = import_bytes_result("token-limit.stl", bytes.as_bytes(), options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_token_bytes"
        }
    ));
    Ok(())
}

#[test]
fn face_limit_is_enforced() -> Result<()> {
    let bytes = ascii_triangle("triangle");
    let mut options = ImportOptions::memory();
    options.limits.max_faces = 0;
    let (result, _) = import_bytes_result("face-limit.stl", bytes.as_bytes(), options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded { limit: "max_faces" }
    ));
    Ok(())
}

#[test]
fn solid_limit_is_enforced() -> Result<()> {
    let bytes = format!("{}{}", ascii_triangle("left"), ascii_triangle("right"));
    let mut options = ImportOptions::memory();
    options.limits.max_solids = 1;
    let (result, _) = import_bytes_result("solid-limit.stl", bytes.as_bytes(), options)?;
    let error = expected_error(result)?;

    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_solids"
        }
    ));
    Ok(())
}
