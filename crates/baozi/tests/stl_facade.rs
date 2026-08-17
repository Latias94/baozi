#![allow(
    clippy::result_large_err,
    reason = "facade tests preserve complete bounded terminal evidence by value"
)]
#![cfg(feature = "format-stl")]

mod common;

use baozi::{
    Error, FormatFailureDetail, FormatFailureDetailValue, FormatFailureEnvelope, ImportCause,
    Importer, Result,
};
use common::scene_snapshot;

fn has_expected_structure_reason(envelope: &FormatFailureEnvelope) -> bool {
    envelope.details().iter().any(is_expected_structure_reason)
}

fn is_expected_structure_reason(detail: FormatFailureDetail<'_>) -> bool {
    detail.key() == "reason"
        && matches!(
            detail.value(),
            FormatFailureDetailValue::String(value)
                if value.as_str() == "ended-before-endsolid"
        )
}

fn ascii_triangle() -> &'static [u8] {
    b"solid facade
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 1 0 0
vertex 0 1 0
endloop
endfacet
endsolid facade
"
}

fn binary_triangle() -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    for value in [0.0_f32, 0.0, 1.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for vertex in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in vertex {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes
}

#[test]
fn facade_reads_ascii_stl_from_bytes() -> Result<()> {
    let report = Importer::new()?.read_bytes("facade.stl", ascii_triangle())?;

    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert_eq!(scene_snapshot(&report).logical_mesh_count(), 1);
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn same_importer_reads_stl_repeatedly() -> Result<()> {
    let importer = Importer::new()?;
    let first = importer.read_bytes("first.stl", ascii_triangle())?;
    let second = importer.read_bytes("second.stl", ascii_triangle())?;

    assert_eq!(first.format().id().as_str(), "org.baozi.stl");
    assert_eq!(second.format().id().as_str(), "org.baozi.stl");
    let first_snapshot = scene_snapshot(&first);
    let second_snapshot = scene_snapshot(&second);
    assert_eq!(first_snapshot.logical_mesh_count(), 1);
    assert_eq!(second_snapshot.logical_mesh_count(), 1);
    Ok(())
}

#[test]
fn facade_preserves_successful_stl_diagnostics() -> Result<()> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"solid empty\nendsolid empty\n");
    bytes.extend_from_slice(ascii_triangle());

    let report = Importer::new()?.read_bytes("diagnostic.stl", &bytes)?;

    assert_eq!(scene_snapshot(&report).logical_mesh_count(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(report.diagnostics()[0].code().as_str(), "stl.empty_solid");
    Ok(())
}

#[test]
fn facade_content_detection_beats_unknown_extension() -> Result<()> {
    let bytes = binary_triangle();
    let report = Importer::new()?.read_bytes("facade.bin", &bytes)?;

    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert_eq!(scene_snapshot(&report).logical_mesh_count(), 1);
    Ok(())
}

#[test]
fn malformed_stl_bytes_preserve_the_stable_format_envelope() {
    let error = Importer::new()
        .expect("default registry should freeze")
        .read_bytes("bad.stl", b"solid bad")
        .expect_err("malformed STL should fail");
    let Error::Import(failure) = error else {
        panic!("expected a post-session import failure");
    };
    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected an STL-owned format cause");
    };

    assert_eq!(envelope.format().as_str(), "org.baozi.stl");
    assert_eq!(envelope.code(), "stl.invalid_structure");
    assert!(has_expected_structure_reason(envelope));
}

#[cfg(feature = "native-fs-candidate")]
#[test]
fn facade_reads_stl_from_native_path() {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("baozi-facade-{suffix}.stl"));
    fs::write(&path, ascii_triangle()).expect("temporary STL should be writable");
    let report = Importer::new()
        .expect("default registry should freeze")
        .read_path(&path)
        .expect("native STL import should succeed");
    fs::remove_file(&path).expect("temporary STL should be removable");

    assert_eq!(scene_snapshot(&report).logical_mesh_count(), 1);
}
