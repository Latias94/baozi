use baozi_core::{BaoziError, Result};
use baozi_import::{ReadConfidence, ReadHint};
use baozi_io::ReadSeek;
use std::io::SeekFrom;

pub(crate) const HEADER_BYTES: usize = 80;
pub(crate) const FACET_COUNT_BYTES: usize = 4;
pub(crate) const BINARY_PREAMBLE_BYTES: usize = HEADER_BYTES + FACET_COUNT_BYTES;
pub(crate) const FACET_BYTES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlKind {
    Binary { facets: u32 },
    Ascii,
}

pub(crate) fn can_read(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
    let original = input
        .stream_position()
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;
    let result = detect_stream(input, hint);
    input
        .seek(SeekFrom::Start(original))
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    match result? {
        Some(StlKind::Binary { .. }) => Ok(ReadConfidence::Certain),
        Some(StlKind::Ascii) => Ok(ReadConfidence::Likely),
        None => Ok(ReadConfidence::No),
    }
}

pub(crate) fn detect_bytes(bytes: &[u8]) -> Option<StlKind> {
    detect_from_parts(bytes, bytes.len() as u64)
}

fn detect_stream(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<Option<StlKind>> {
    let len = input
        .seek(SeekFrom::End(0))
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    let prefix_len = if len > BINARY_PREAMBLE_BYTES as u64 {
        BINARY_PREAMBLE_BYTES
    } else {
        len as usize
    };
    let mut prefix = vec![0; prefix_len];
    input
        .read_exact(&mut prefix)
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    Ok(detect_from_parts(&prefix, len))
}

fn detect_from_parts(prefix: &[u8], full_len: u64) -> Option<StlKind> {
    if let Some(facets) = binary_facets(prefix, full_len) {
        return Some(StlKind::Binary { facets });
    }

    starts_with_ascii_solid(prefix).then_some(StlKind::Ascii)
}

fn binary_facets(prefix: &[u8], full_len: u64) -> Option<u32> {
    if full_len < BINARY_PREAMBLE_BYTES as u64 || prefix.len() < BINARY_PREAMBLE_BYTES {
        return None;
    }

    let count = u32::from_le_bytes([
        prefix[HEADER_BYTES],
        prefix[HEADER_BYTES + 1],
        prefix[HEADER_BYTES + 2],
        prefix[HEADER_BYTES + 3],
    ]);
    let expected = (BINARY_PREAMBLE_BYTES as u64)
        .checked_add((count as u64).checked_mul(FACET_BYTES as u64)?)?;

    (expected == full_len).then_some(count)
}

fn starts_with_ascii_solid(prefix: &[u8]) -> bool {
    let mut index = 0;
    if prefix.starts_with(&[0xef, 0xbb, 0xbf]) {
        index = 3;
    }
    while prefix.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }

    let rest = &prefix[index..];
    if rest.len() < 5 || !rest[..5].eq_ignore_ascii_case(b"solid") {
        return false;
    }

    rest.get(5).is_none_or(u8::is_ascii_whitespace)
}
