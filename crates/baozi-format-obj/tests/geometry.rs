mod common;

use baozi_core::{MetadataValue, PrimitiveTopology, Vec2, Vec3};
use common::{TestResult, import_bytes, scene};

#[test]
fn imports_one_triangle_obj() -> TestResult<()> {
    let bytes = b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_bytes("triangle.obj", bytes)?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.nodes().len(), 2);
    assert_eq!(scene.nodes()[1].name.as_deref(), Some("triangle"));
    let mesh = &scene.meshes()[0];
    assert_eq!(mesh.name.as_deref(), Some("triangle"));
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert!(mesh.face_vertex_counts.is_empty());
    Ok(())
}

#[test]
fn imports_quad_as_polygon_with_face_counts() -> TestResult<()> {
    let bytes = b"g quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";

    let report = import_bytes("quad.obj", bytes)?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    let mesh = &scene.meshes()[0];
    assert_eq!(mesh.name.as_deref(), Some("quad"));
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn remaps_separate_position_uv_normal_indices() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\nvn 0 0 1\nvn 0 0 -1\nf 1/1/1 2/2/1 3/3/1\nf 1/4/2 3/3/2 2/2/2\n";

    let report = import_bytes("remap.obj", bytes)?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    let mesh = &scene.meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions.len(), 6);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(mesh.texcoords[0][3], Vec2::new(1.0, 1.0));
    assert_eq!(mesh.normals[3], Vec3::new(0.0, 0.0, -1.0));
    Ok(())
}

#[test]
fn supports_negative_face_indices() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n";

    let report = import_bytes("negative.obj", bytes)?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    let mesh = &scene.meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn partial_uv_and_normal_channels_are_diagnosed_and_omitted() -> TestResult<()> {
    let bytes =
        b"g mixed\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2 3//1\n";

    let report = import_bytes("partial.obj", bytes)?;
    let scene = scene(&report);
    let diagnostics = report.diagnostics();

    assert_eq!(scene.meshes().len(), 1);
    assert!(scene.meshes()[0].texcoords.is_empty());
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.partial_texcoord_channel")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.partial_normal_channel")
    );
    Ok(())
}

#[test]
fn state_changes_split_meshes_and_preserve_material_bindings() -> TestResult<()> {
    let bytes = b"o first-object\ng first-group\nusemtl red\ns 1\n\
        v 0 0 0\nv 1 0 0\nv 0 1 0\nv 2 0 0\nv 3 0 0\nv 2 1 0\n\
        f 1 2 3\no second-object\ng second-group\nusemtl blue\ns off\nf 4 5 6\n";

    let report = import_bytes("state-switch.obj", bytes)?;
    let scene = scene(&report);

    assert_eq!(scene.meshes().len(), 2);
    assert_eq!(scene.materials().len(), 2);
    assert_eq!(report.diagnostics().len(), 2);

    let first = &scene.meshes()[0];
    let second = &scene.meshes()[1];
    assert_eq!(first.name.as_deref(), Some("first-group"));
    assert_eq!(second.name.as_deref(), Some("second-group"));
    assert_eq!(first.material.map(|id| id.index()), Some(0));
    assert_eq!(second.material.map(|id| id.index()), Some(1));
    assert_eq!(scene.materials()[0].name.as_deref(), Some("red"));
    assert_eq!(scene.materials()[1].name.as_deref(), Some("blue"));
    assert_eq!(
        first.metadata.get("obj:object"),
        Some(&MetadataValue::String("first-object".to_owned()))
    );
    assert_eq!(
        second.metadata.get("obj:smoothing"),
        Some(&MetadataValue::String("off".to_owned()))
    );
    Ok(())
}

#[test]
fn repeating_the_current_state_does_not_split_the_mesh() -> TestResult<()> {
    let bytes = b"o same\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no same\nf 1 2 3\n";

    let report = import_bytes("same-state.obj", bytes)?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].indices, vec![0, 1, 2, 0, 1, 2]);
    Ok(())
}
