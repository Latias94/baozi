mod common;

use baozi_core::Result;
use baozi_format_stl::StlImporter;
use baozi_import::{FormatImporter, ReadConfidence, ReadHint};
use baozi_io::AssetPath;
use common::{BinaryFacet, ascii_triangle, binary_stl};
use std::io::{Cursor, Seek};

fn hint(source: &str) -> Result<ReadHint> {
    Ok(ReadHint::from_source(AssetPath::new(source)?))
}

#[test]
fn binary_exact_size_is_certain() -> Result<()> {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut cursor = Cursor::new(bytes);
    let confidence = StlImporter.can_read(&mut cursor, &hint("triangle.stl")?)?;

    assert_eq!(confidence, ReadConfidence::Certain);
    Ok(())
}

#[test]
fn binary_header_starting_with_solid_is_still_binary() -> Result<()> {
    let bytes = binary_stl(b"solid binary header", &[BinaryFacet::unit_triangle()]);
    let mut cursor = Cursor::new(bytes);
    let confidence = StlImporter.can_read(&mut cursor, &hint("solid-header.stl")?)?;

    assert_eq!(confidence, ReadConfidence::Certain);
    Ok(())
}

#[test]
fn ascii_solid_after_whitespace_is_likely() -> Result<()> {
    let bytes = format!("\n\t{}", ascii_triangle("triangle"));
    let mut cursor = Cursor::new(bytes.into_bytes());
    let confidence = StlImporter.can_read(&mut cursor, &hint("triangle.stl")?)?;

    assert_eq!(confidence, ReadConfidence::Likely);
    Ok(())
}

#[test]
fn ascii_solid_after_utf8_bom_is_likely() -> Result<()> {
    let bytes = format!("\u{feff}{}", ascii_triangle("triangle"));
    let mut cursor = Cursor::new(bytes.into_bytes());
    let confidence = StlImporter.can_read(&mut cursor, &hint("triangle.stl")?)?;

    assert_eq!(confidence, ReadConfidence::Likely);
    Ok(())
}

#[test]
fn random_bytes_are_not_stl() -> Result<()> {
    let mut cursor = Cursor::new(b"baozi is not an stl".to_vec());
    let confidence = StlImporter.can_read(&mut cursor, &hint("triangle.stl")?)?;

    assert_eq!(confidence, ReadConfidence::No);
    Ok(())
}

#[test]
fn can_read_rewinds_input() -> Result<()> {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut cursor = Cursor::new(bytes);
    cursor.set_position(7);
    let _ = StlImporter.can_read(&mut cursor, &hint("triangle.stl")?)?;

    let position = cursor
        .stream_position()
        .map_err(|error| baozi_core::BaoziError::io("cursor", error.to_string()))?;
    assert_eq!(position, 7);
    Ok(())
}
