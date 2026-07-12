use baozi_import::{
    DetectionInconclusive, DetectionOutcome, DetectionResult, DetectionScope, Probe,
    ReadConfidence, ReadHint,
};

const LF_MAGIC: &[u8] = b"ply\n";
const CRLF_MAGIC: &[u8] = b"ply\r\n";

pub(crate) fn detect(
    probe: &Probe<'_>,
    hint: &ReadHint,
    scope: &DetectionScope<'_>,
) -> DetectionResult<DetectionOutcome> {
    if starts_with_accounted(scope, probe.bytes(), LF_MAGIC)?
        || starts_with_accounted(scope, probe.bytes(), CRLF_MAGIC)?
    {
        return Ok(DetectionOutcome::Match(ReadConfidence::Certain));
    }
    if hint
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ply"))
    {
        return Ok(DetectionOutcome::Match(ReadConfidence::Maybe));
    }
    if !probe.is_complete()
        && (starts_with_accounted(scope, LF_MAGIC, probe.bytes())?
            || starts_with_accounted(scope, CRLF_MAGIC, probe.bytes())?)
    {
        return Ok(DetectionOutcome::Inconclusive(
            DetectionInconclusive::ProbeExhausted,
        ));
    }
    Ok(DetectionOutcome::NoMatch)
}

fn starts_with_accounted(
    scope: &DetectionScope<'_>,
    bytes: &[u8],
    prefix: &[u8],
) -> DetectionResult<bool> {
    let inspected = if bytes.len() >= prefix.len() {
        prefix.len()
    } else {
        0
    };
    if inspected == 0 {
        return Ok(bytes.starts_with(prefix));
    }
    let permit = scope.begin_work(inspected as u64)?;
    let result = bytes.starts_with(prefix);
    permit.complete()?;
    Ok(result)
}
