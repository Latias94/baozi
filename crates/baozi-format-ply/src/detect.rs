use baozi_import::{DetectionInconclusive, DetectionOutcome, Probe, ReadConfidence, ReadHint};

const LF_MAGIC: &[u8] = b"ply\n";
const CRLF_MAGIC: &[u8] = b"ply\r\n";

pub(crate) fn can_read(probe: &Probe<'_>, hint: &ReadHint) -> DetectionOutcome {
    if probe.bytes().starts_with(LF_MAGIC) || probe.bytes().starts_with(CRLF_MAGIC) {
        return DetectionOutcome::Match(ReadConfidence::Certain);
    }
    if hint
        .extension
        .as_deref()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ply"))
    {
        return DetectionOutcome::Match(ReadConfidence::Maybe);
    }
    if !probe.is_complete()
        && (LF_MAGIC.starts_with(probe.bytes()) || CRLF_MAGIC.starts_with(probe.bytes()))
    {
        return DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted);
    }
    DetectionOutcome::NoMatch
}
