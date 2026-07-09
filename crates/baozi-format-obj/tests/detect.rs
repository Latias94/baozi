use baozi_core::Result;
use baozi_format_obj::ObjImporter;
use baozi_import::{FormatImporter, ReadConfidence, ReadHint};
use baozi_io::AssetPath;
use std::io::{Cursor, Seek};

fn can_read(source: &str, bytes: &[u8]) -> Result<ReadConfidence> {
    let path = AssetPath::new(source)?;
    let hint = ReadHint::from_source(path);
    let mut input = Cursor::new(bytes.to_vec());
    ObjImporter.can_read(&mut input, &hint)
}

#[test]
fn obj_extension_is_candidate() -> Result<()> {
    let confidence = can_read("mesh.obj", b"not enough content")?;

    assert_eq!(confidence, ReadConfidence::Maybe);
    Ok(())
}

#[test]
fn obj_like_content_is_likely() -> Result<()> {
    let confidence = can_read("mesh.txt", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")?;

    assert_eq!(confidence, ReadConfidence::Likely);
    Ok(())
}

#[test]
fn random_text_is_not_obj_without_extension() -> Result<()> {
    let confidence = can_read("notes.txt", b"this is not a model\n")?;

    assert_eq!(confidence, ReadConfidence::No);
    Ok(())
}

#[test]
fn can_read_rewinds_input() -> Result<()> {
    let path = AssetPath::new("mesh.obj")?;
    let hint = ReadHint::from_source(path);
    let mut input = Cursor::new(b"v 0 0 0\nf 1 1 1\n".to_vec());
    input.set_position(2);

    let confidence = ObjImporter.can_read(&mut input, &hint)?;

    assert_eq!(confidence, ReadConfidence::Likely);
    assert_eq!(input.stream_position().unwrap(), 2);
    Ok(())
}
