#![allow(
    clippy::result_large_err,
    reason = "facade tests preserve complete bounded terminal evidence by value"
)]
#![cfg(feature = "format-obj")]

mod common;

#[cfg(any(feature = "format-stl", feature = "format-ply"))]
use baozi::{Importer, Result};
#[cfg(any(feature = "format-stl", feature = "format-ply"))]
use common::scene_snapshot;

#[cfg(all(feature = "format-obj", feature = "format-stl"))]
fn large_binary_stl(facet_count: u32) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes.extend_from_slice(&facet_count.to_le_bytes());
    for _ in 0..facet_count {
        for value in [0.0_f32, 0.0, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for vertex in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in vertex {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes
}

#[cfg(all(feature = "format-obj", feature = "format-stl"))]
#[test]
fn greedy_obj_detection_does_not_block_large_binary_stl() -> Result<()> {
    let bytes = large_binary_stl(100);
    assert!(bytes.len() > 4096);

    let report = Importer::new()?.read_bytes("large.mesh", bytes)?;
    let snapshot = scene_snapshot(&report);

    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert_eq!(snapshot.logical_mesh_count(), 1);
    Ok(())
}

#[cfg(all(feature = "format-obj", feature = "format-ply"))]
#[test]
fn greedy_obj_detection_does_not_block_large_ply() -> Result<()> {
    let mut bytes = b"ply\nformat ascii 1.0\n".to_vec();
    for _ in 0..400 {
        bytes.extend_from_slice(b"comment detection-padding\n");
    }
    bytes.extend_from_slice(
        b"element vertex 3\n\
property float x\n\
property float y\n\
property float z\n\
element face 1\n\
property list uchar uint vertex_indices\n\
end_header\n\
0 0 0\n\
1 0 0\n\
0 1 0\n\
3 0 1 2\n",
    );
    assert!(bytes.len() > 4096);

    let report = Importer::new()?.read_bytes("large.mesh", bytes)?;
    let snapshot = scene_snapshot(&report);

    assert_eq!(report.format().id().as_str(), "org.baozi.ply");
    assert_eq!(snapshot.logical_mesh_count(), 1);
    Ok(())
}
