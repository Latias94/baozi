mod common;

use baozi_core::Result;
use baozi_format_gltf::GltfImporter;
use baozi_import::{FormatImporter, ReadConfidence, ReadHint};
use baozi_io::AssetPath;
use common::triangle_gltf;
use std::io::Cursor;

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
