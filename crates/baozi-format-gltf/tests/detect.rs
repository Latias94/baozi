mod common;

use baozi_core::{BaoziErrorKind, Result};
use baozi_format_gltf::GltfImporter;
use baozi_import::{DetectionOptions, FormatImporter, ImporterRegistry, ReadConfidence, ReadHint};
use baozi_io::AssetPath;
use common::triangle_gltf;
use std::io::{Cursor, Seek};

#[test]
fn detects_gltf_json() -> Result<()> {
    let source = AssetPath::new("scene.gltf")?;
    let hint = ReadHint::from_source(source);
    let mut input = Cursor::new(triangle_gltf());

    let confidence = GltfImporter.can_read(&mut input, &hint)?;

    assert_eq!(confidence, ReadConfidence::Likely);
    Ok(())
}

#[test]
fn detects_glb_magic() -> Result<()> {
    let source = AssetPath::new("scene.glb")?;
    let hint = ReadHint::from_source(source);
    let mut input = Cursor::new(b"glTF\x02\0\0\0".to_vec());

    let confidence = GltfImporter.can_read(&mut input, &hint)?;

    assert_eq!(confidence, ReadConfidence::Certain);
    Ok(())
}

#[test]
fn can_read_rewinds_input() -> Result<()> {
    let source = AssetPath::new("scene.gltf")?;
    let hint = ReadHint::from_source(source);
    let mut input = Cursor::new(triangle_gltf());
    input.set_position(7);

    let confidence = GltfImporter.can_read(&mut input, &hint)?;

    assert_eq!(confidence, ReadConfidence::Likely);
    assert_eq!(input.stream_position().unwrap(), 7);
    Ok(())
}

#[test]
fn registry_probe_limit_bounds_content_detection() -> Result<()> {
    let mut registry = ImporterRegistry::new();
    registry.register(GltfImporter)?;
    let hint = ReadHint::from_source(AssetPath::new("scene.mesh")?);

    let mut too_short = Cursor::new(triangle_gltf());
    let error = registry
        .detect_with_options(
            &mut too_short,
            &hint,
            &DetectionOptions { max_probe_bytes: 1 },
        )
        .unwrap_err();
    assert_eq!(error.kind(), BaoziErrorKind::UnsupportedFormat);

    let mut enough = Cursor::new(triangle_gltf());
    let selected = registry.detect_with_options(
        &mut enough,
        &hint,
        &DetectionOptions {
            max_probe_bytes: 4096,
        },
    )?;
    assert_eq!(selected.info.id(), "gltf");
    assert_eq!(selected.confidence, ReadConfidence::Likely);
    Ok(())
}
