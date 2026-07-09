mod common;

use baozi_core::{BaoziError, PrimitiveTopology, Result, Vec2, Vec3, VertexAttributeData};
use common::{
    ascii_point_cloud, ascii_quad, ascii_triangle, binary_triangle, expected_error, import_bytes,
    import_bytes_result,
};

#[test]
fn imports_ascii_triangle_with_common_vertex_streams() -> Result<()> {
    let (scene, diagnostics) = import_bytes("triangle.ply", ascii_triangle())?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.nodes.len(), 2);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("PLY"));

    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name.as_deref(), Some("PLY Mesh"));
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(
        mesh.positions,
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ]
    );
    assert_eq!(mesh.normals, vec![Vec3::new(0.0, 0.0, 1.0); 3]);
    assert_eq!(mesh.texcoords[0][1], Vec2::new(1.0, 0.0));
    assert_eq!(mesh.colors[0][0].r, 1.0);
    assert_eq!(mesh.colors[0][1].g, 1.0);
    assert_eq!(mesh.colors[0][2].b, 1.0);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert!(mesh.face_vertex_counts.is_empty());
    assert_eq!(
        mesh.metadata.get("ply.format"),
        Some(&baozi_core::MetadataValue::String("ascii".to_owned()))
    );
    assert_eq!(
        mesh.metadata.get("ply.comment.0"),
        Some(&baozi_core::MetadataValue::String(
            "fixture triangle".to_owned()
        ))
    );
    assert_eq!(mesh.custom_attributes.len(), 1);
    assert_eq!(mesh.custom_attributes[0].name, "ply:temperature");
    assert!(matches!(
        &mesh.custom_attributes[0].data,
        VertexAttributeData::F32(values)
            if values == &vec![0.5, 0.75, 1.0]
    ));
    Ok(())
}

#[test]
fn imports_ascii_point_cloud_without_faces() -> Result<()> {
    let (scene, diagnostics) = import_bytes("points.ply", ascii_point_cloud())?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Points);
    assert_eq!(mesh.positions.len(), 2);
    assert!(mesh.indices.is_empty());
    assert!(mesh.face_vertex_counts.is_empty());
    Ok(())
}

#[test]
fn imports_ascii_quad_as_polygon() -> Result<()> {
    let (scene, diagnostics) = import_bytes("quad.ply", ascii_quad())?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn imports_binary_little_endian_triangle() -> Result<()> {
    let bytes = binary_triangle(true);
    let (scene, diagnostics) = import_bytes("triangle-le.ply", &bytes)?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(
        mesh.metadata.get("ply.format"),
        Some(&baozi_core::MetadataValue::String(
            "binary_little_endian".to_owned()
        ))
    );
    Ok(())
}

#[test]
fn imports_binary_big_endian_triangle() -> Result<()> {
    let bytes = binary_triangle(false);
    let (scene, diagnostics) = import_bytes("triangle-be.ply", &bytes)?;

    assert!(diagnostics.is_empty());
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions[2], Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(
        mesh.metadata.get("ply.format"),
        Some(&baozi_core::MetadataValue::String(
            "binary_big_endian".to_owned()
        ))
    );
    Ok(())
}

#[test]
fn malformed_header_is_parse_error() -> Result<()> {
    let (result, diagnostics) = import_bytes_result(
        "bad.ply",
        b"not-ply\nformat ascii 1.0\nend_header\n",
        Default::default(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("must start"));
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_header() -> Result<()> {
    let mut options = baozi_import::ImportOptions::memory();
    options.limits.max_vertices = 1;
    let (result, diagnostics) = import_bytes_result("points.ply", ascii_point_cloud(), options)?;
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
