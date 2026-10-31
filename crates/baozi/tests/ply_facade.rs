#![allow(
    clippy::result_large_err,
    reason = "facade tests preserve complete bounded terminal evidence by value"
)]
#![cfg(feature = "format-ply")]

mod common;

use baozi::{CapabilityStatus, FormatCapability, Importer, Result};
use common::{scene_snapshot, snapshot_line};

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
    let report = Importer::new()?.read_bytes("facade.ply", triangle_ply())?;
    let snapshot = scene_snapshot(&report);

    assert_eq!(report.format().id().as_str(), "org.baozi.ply");
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("topology=triangle_list"));
    let position = snapshot_line(&snapshot, "attribute[0]");
    assert!(position.contains(concat!(
        "semantic=position semantic_set=0 domain=vertex scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3"
    )));
    assert!(position.contains(concat!(
        "values(present,total=9)=[",
        "0x00000000,0x00000000,0x00000000,0x3f800000"
    )));
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn facade_reports_ply_capabilities() -> Result<()> {
    let report = Importer::new()?.read_bytes("facade.ply", triangle_ply())?;

    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::Geometry),
        Some(CapabilityStatus::Supported)
    );
    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::Diagnostics),
        Some(CapabilityStatus::Supported)
    );
    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::ResourceLimits),
        Some(CapabilityStatus::Supported)
    );
    Ok(())
}
