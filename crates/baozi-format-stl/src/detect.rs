use crate::ascii;
use baozi_import::{
    DetectionInconclusive, DetectionOutcome, DetectionResult, DetectionScope, Probe,
    ReadConfidence, ReadHint,
};

pub(crate) const HEADER_BYTES: usize = 80;
pub(crate) const FACET_COUNT_BYTES: usize = 4;
pub(crate) const BINARY_PREAMBLE_BYTES: usize = HEADER_BYTES + FACET_COUNT_BYTES;
pub(crate) const FACET_BYTES: usize = 50;

pub(crate) fn detect(
    probe: &Probe<'_>,
    hint: &ReadHint,
    scope: &DetectionScope<'_>,
) -> DetectionResult<DetectionOutcome> {
    let binary = account_detection_work(scope, binary_probe_work_bytes(probe), || {
        binary_facets(probe.bytes(), probe.source_len())
    })?;
    if binary.is_some() {
        return Ok(DetectionOutcome::Match(ReadConfidence::Certain));
    }
    if ascii::validate_text_profile_for_detection(scope, probe.bytes())?.is_some() {
        let starts_with_solid = account_detection_work(scope, probe.bytes().len(), || {
            starts_with_ascii_solid(probe.bytes(), probe.is_complete())
        })?;
        if starts_with_solid {
            return Ok(DetectionOutcome::Match(ReadConfidence::Likely));
        }
    }
    if !probe.is_complete() {
        return Ok(DetectionOutcome::Inconclusive(
            DetectionInconclusive::ProbeExhausted,
        ));
    }
    if hint
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("stl"))
    {
        return Ok(DetectionOutcome::Match(ReadConfidence::Maybe));
    }
    Ok(DetectionOutcome::NoMatch)
}

fn binary_probe_work_bytes(probe: &Probe<'_>) -> usize {
    if probe.source_len() >= BINARY_PREAMBLE_BYTES as u64
        && probe.bytes().len() >= BINARY_PREAMBLE_BYTES
    {
        FACET_COUNT_BYTES
    } else {
        0
    }
}

fn account_detection_work<T>(
    scope: &DetectionScope<'_>,
    byte_count: usize,
    work: impl FnOnce() -> T,
) -> DetectionResult<T> {
    if byte_count == 0 {
        return Ok(work());
    }
    scope.with_work(byte_count as u64, || Ok(work()))
}

pub(crate) fn exact_binary_facets(bytes: &[u8]) -> Option<u32> {
    binary_facets(bytes, bytes.len() as u64)
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
