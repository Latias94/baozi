mod common;

use baozi_core::{PrimitiveTopology, Result, Vec2, Vec3};
use common::import_bytes;

#[test]
fn imports_one_triangle_obj() -> Result<()> {
    let bytes = b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let (scene, diagnostics) = import_bytes("triangle.obj", bytes)?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.nodes.len(), 2);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("triangle"));
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name.as_deref(), Some("triangle"));
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert!(mesh.face_vertex_counts.is_empty());
    Ok(())
}

#[test]
fn imports_quad_as_polygon_with_face_counts() -> Result<()> {
    let bytes = b"g quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";

    let (scene, diagnostics) = import_bytes("quad.obj", bytes)?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name.as_deref(), Some("quad"));
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn remaps_separate_position_uv_normal_indices() -> Result<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\nvn 0 0 1\nvn 0 0 -1\nf 1/1/1 2/2/1 3/3/1\nf 1/4/2 3/3/2 2/2/2\n";

    let (scene, diagnostics) = import_bytes("remap.obj", bytes)?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions.len(), 6);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(mesh.texcoords[0][3], Vec2::new(1.0, 1.0));
    assert_eq!(mesh.normals[3], Vec3::new(0.0, 0.0, -1.0));
    Ok(())
}

#[test]
fn supports_negative_face_indices() -> Result<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n";

    let (scene, diagnostics) = import_bytes("negative.obj", bytes)?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn partial_uv_and_normal_channels_are_diagnosed_and_omitted() -> Result<()> {
    let bytes =
        b"g mixed\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2 3//1\n";

    let (scene, diagnostics) = import_bytes("partial.obj", bytes)?;

    assert_eq!(scene.meshes.len(), 1);
    assert!(scene.meshes[0].texcoords.is_empty());
    assert!(scene.meshes[0].normals.is_empty());
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "obj.partial_texcoord_channel")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "obj.partial_normal_channel")
    );
    Ok(())
}
