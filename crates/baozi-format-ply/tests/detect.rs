use baozi_format_ply::PlyImporter;
use baozi_import::{FormatImporter, ReadConfidence, ReadHint};
use std::io::Cursor;

#[test]
fn detects_ply_magic() {
    let mut input = Cursor::new(b"ply\nformat ascii 1.0\n".to_vec());
    let confidence = PlyImporter
        .can_read(&mut input, &ReadHint::default())
        .expect("detection succeeds");

    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn extension_is_maybe_without_magic() {
    let mut input = Cursor::new(b"not-ply".to_vec());
    let confidence = PlyImporter
        .can_read(
            &mut input,
            &ReadHint {
                source: None,
                extension: Some("ply".to_owned()),
            },
        )
        .expect("detection succeeds");

    assert_eq!(confidence, ReadConfidence::Maybe);
}
