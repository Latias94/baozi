#![cfg(feature = "format-ply")]

use baozi::{CapabilityStatus, FormatCapability, Importer, PrimitiveTopology, Result, Vec3};

fn triangle_ply() -> &'static [u8] {
    b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2
"
}

#[test]
fn facade_reads_ply_from_bytes() -> Result<()> {
    let report = Importer::new().read_bytes("facade.ply", triangle_ply())?;

    assert_eq!(report.format().id(), "ply");
    let mesh = &report.scene().meshes[0];
    assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
    assert_eq!(mesh.positions[1], Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn facade_reports_ply_capabilities() -> Result<()> {
    let report = Importer::new().read_bytes("facade.ply", triangle_ply())?;

    assert!(
        report
            .format()
            .capabilities()
            .contains(&(FormatCapability::Geometry, CapabilityStatus::Supported))
    );
    assert!(
        report
            .format()
            .capabilities()
            .contains(&(FormatCapability::Diagnostics, CapabilityStatus::Supported))
    );
    assert!(report.format().capabilities().contains(&(
        FormatCapability::ResourceLimits,
        CapabilityStatus::Supported
    )));
    Ok(())
}
