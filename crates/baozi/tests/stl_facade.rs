use baozi::{BaoziError, Importer, Result};

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
    let report = Importer::new().read_bytes("facade.stl", ascii_triangle())?;

    assert_eq!(report.format().id(), "stl");
    assert_eq!(report.scene().meshes.len(), 1);
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn same_importer_reads_stl_repeatedly() -> Result<()> {
    let importer = Importer::new();
    let first = importer.read_bytes("first.stl", ascii_triangle())?;
    let second = importer.read_bytes("second.stl", ascii_triangle())?;

    assert_eq!(first.format().id(), "stl");
    assert_eq!(second.format().id(), "stl");
    assert_eq!(first.scene().meshes.len(), 1);
    assert_eq!(second.scene().meshes.len(), 1);
    Ok(())
}

#[test]
fn facade_preserves_successful_stl_diagnostics() -> Result<()> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"solid empty\nendsolid empty\n");
    bytes.extend_from_slice(ascii_triangle());

    let report = Importer::new().read_bytes("diagnostic.stl", &bytes)?;

    assert_eq!(report.scene().meshes.len(), 1);
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(report.diagnostics()[0].code.0, "stl.empty_solid");
    Ok(())
}

#[test]
fn facade_content_detection_beats_unknown_extension() -> Result<()> {
    let bytes = binary_triangle();
    let report = Importer::new().read_bytes("facade.bin", &bytes)?;

    assert_eq!(report.format().id(), "stl");
    assert_eq!(report.scene().meshes.len(), 1);
    Ok(())
}

#[test]
fn malformed_stl_bytes_return_error() -> Result<()> {
    let error = match Importer::new().read_bytes("bad.stl", b"solid bad") {
        Ok(_) => {
            return Err(BaoziError::parse(
                "test",
                None,
                "expected malformed STL to fail",
            ));
        }
        Err(error) => error,
    };

    assert!(matches!(error, BaoziError::Parse { .. }));
    Ok(())
}

#[cfg(feature = "native-fs")]
#[test]
fn facade_reads_stl_from_native_path() -> Result<()> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| BaoziError::io("clock", error.to_string()))?
        .as_nanos();
    let path = std::env::temp_dir().join(format!("baozi-facade-{suffix}.stl"));
    fs::write(&path, ascii_triangle())
        .map_err(|error| BaoziError::io("temp stl", error.to_string()))?;
    let report = Importer::new().read_path(&path);
    let cleanup = fs::remove_file(&path);
    if let Err(error) = cleanup {
        return Err(BaoziError::io("temp stl cleanup", error.to_string()));
    }

    assert_eq!(report?.scene.meshes.len(), 1);
    Ok(())
}
