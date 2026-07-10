use baozi_import::{DetectionInconclusive, DetectionOutcome, Probe, ReadConfidence, ReadHint};

pub(crate) const HEADER_BYTES: usize = 80;
pub(crate) const FACET_COUNT_BYTES: usize = 4;
pub(crate) const BINARY_PREAMBLE_BYTES: usize = HEADER_BYTES + FACET_COUNT_BYTES;
pub(crate) const FACET_BYTES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlKind {
    Binary { facets: u32 },
    Ascii,
}

pub(crate) fn can_read(probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
    match detect_from_parts(probe.bytes(), probe.source_len()) {
        Some(StlKind::Binary { .. }) => DetectionOutcome::Match(ReadConfidence::Certain),
        Some(StlKind::Ascii) => DetectionOutcome::Match(ReadConfidence::Likely),
        None if !probe.is_complete() => {
            DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
        }
        None => DetectionOutcome::NoMatch,
    }
}

pub(crate) fn detect_bytes(bytes: &[u8]) -> Option<StlKind> {
    detect_from_parts(bytes, bytes.len() as u64)
}

fn detect_from_parts(prefix: &[u8], full_len: u64) -> Option<StlKind> {
    if let Some(facets) = binary_facets(prefix, full_len) {
        return Some(StlKind::Binary { facets });
    }

    starts_with_ascii_solid(prefix, prefix.len() as u64 == full_len).then_some(StlKind::Ascii)
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

fn starts_with_ascii_solid(prefix: &[u8], prefix_is_complete: bool) -> bool {
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

    rest.get(5)
        .map_or(prefix_is_complete, u8::is_ascii_whitespace)
}
