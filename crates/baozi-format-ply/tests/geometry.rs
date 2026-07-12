mod common;

use baozi_core::{PrimitiveTopology, Vec2, Vec3, VertexAttributeData};
use baozi_format_ply::PlyFailureCode;
use baozi_import::ResourceLimits;
use common::{
    TestResult, ascii_point_cloud, ascii_quad, ascii_triangle, assert_resource_limit,
    binary_triangle, expected_error, expected_ply_failure, import_bytes, import_bytes_result,
    ply_unsigned_detail,
};

#[test]
fn imports_ascii_triangle_with_common_vertex_streams() -> TestResult<()> {
    let report = import_bytes("triangle.ply", ascii_triangle())?;
    let scene = report.scene();

    assert!(report.diagnostics().is_empty());
    assert_eq!(report.format().id().as_str(), "org.baozi.ply");
    assert_eq!(report.source().as_str(), "triangle.ply");
    assert!(report.ledger().current_live_bytes() == 0);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.nodes().len(), 2);
    assert_eq!(scene.nodes()[1].name.as_deref(), Some("PLY"));

    let mesh = &scene.meshes()[0];
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
        mesh.metadata.get("ply:format"),
        Some(&baozi_core::MetadataValue::String("ascii".to_owned()))
    );
    assert_eq!(
        mesh.metadata.get("ply:comment.0"),
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
fn imports_ascii_point_cloud_without_faces() -> TestResult<()> {
    let report = import_bytes("points.ply", ascii_point_cloud())?;

    assert!(report.diagnostics().is_empty());
    let mesh = &report.scene().meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Points);
    assert_eq!(mesh.positions.len(), 2);
    assert!(mesh.indices.is_empty());
    assert!(mesh.face_vertex_counts.is_empty());
    Ok(())
}

#[test]
fn imports_ascii_quad_as_polygon() -> TestResult<()> {
    let report = import_bytes("quad.ply", ascii_quad())?;

    assert!(report.diagnostics().is_empty());
    let mesh = &report.scene().meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Polygons);
    assert_eq!(mesh.indices, vec![0, 1, 2, 3]);
    assert_eq!(mesh.face_vertex_counts, vec![4]);
    Ok(())
}

#[test]
fn imports_binary_little_endian_triangle() -> TestResult<()> {
    let bytes = binary_triangle(true);
    let report = import_bytes("triangle-le.ply", &bytes)?;

    assert!(report.diagnostics().is_empty());
    let mesh = &report.scene().meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(
        mesh.metadata.get("ply:format"),
        Some(&baozi_core::MetadataValue::String(
            "binary_little_endian".to_owned()
        ))
    );
    Ok(())
}

#[test]
fn imports_binary_big_endian_triangle() -> TestResult<()> {
    let bytes = binary_triangle(false);
    let report = import_bytes("triangle-be.ply", &bytes)?;

    assert!(report.diagnostics().is_empty());
    let mesh = &report.scene().meshes()[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions[2], Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(
        mesh.metadata.get("ply:format"),
        Some(&baozi_core::MetadataValue::String(
            "binary_big_endian".to_owned()
        ))
    );
    Ok(())
}

#[test]
fn malformed_header_is_parse_error() -> TestResult<()> {
    let result = import_bytes_result(
        "bad.ply",
        b"not-ply\nformat ascii 1.0\nend_header\n",
        ResourceLimits::default(),
    )?;
    let error = expected_error(result)?;

    assert!(error.diagnostics().is_empty());
    let envelope = expected_ply_failure(&error, PlyFailureCode::InvalidHeader);
    assert_eq!(ply_unsigned_detail(envelope, "byte_offset"), Some(0));
    assert_eq!(envelope.details().len(), 1);
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_header() -> TestResult<()> {
    let mut limits = ResourceLimits::default();
    limits.max_vertices = 1;
    let result = import_bytes_result("points.ply", ascii_point_cloud(), limits)?;
    let error = expected_error(result)?;

    assert!(error.diagnostics().is_empty());
    assert_resource_limit(&error, "max_vertices");
    Ok(())
}
