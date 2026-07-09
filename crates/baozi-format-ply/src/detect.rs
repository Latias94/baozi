use baozi_core::{BaoziError, Result};
use baozi_import::{ReadConfidence, ReadHint};
use baozi_io::ReadSeek;

pub(crate) fn can_read(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
    let mut probe = [0_u8; 4];
    let read = input
        .read(&mut probe)
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;
    if read >= 4 && probe == *b"ply\n" {
        return Ok(ReadConfidence::Certain);
    }
    if hint
        .extension
        .as_deref()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ply"))
    {
        return Ok(ReadConfidence::Maybe);
    }
    Ok(ReadConfidence::No)
}
