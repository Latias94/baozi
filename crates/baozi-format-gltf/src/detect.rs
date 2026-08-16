use baozi_import::{
    DetectionInconclusive, DetectionOutcome, DetectionResult, DetectionScope, Probe,
    ReadConfidence, ReadHint,
};

pub(crate) fn can_read(
    probe: &Probe<'_>,
    hint: &ReadHint,
    scope: &DetectionScope<'_>,
) -> DetectionResult<DetectionOutcome> {
    let bytes = probe.bytes();
    let extension_match = hint.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("gltf") || extension.eq_ignore_ascii_case("glb")
    });

    if starts_with_accounted(scope, bytes, b"glTF")? {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    } else if looks_like_gltf_json(bytes, scope)? {
        Ok(DetectionOutcome::Match(ReadConfidence::Likely))
    } else if extension_match {
        Ok(DetectionOutcome::Match(ReadConfidence::Maybe))
    } else if !probe.is_complete() {
        Ok(DetectionOutcome::Inconclusive(
            DetectionInconclusive::ProbeExhausted,
        ))
    } else {
        Ok(DetectionOutcome::NoMatch)
    }
}

fn looks_like_gltf_json(bytes: &[u8], scope: &DetectionScope<'_>) -> DetectionResult<bool> {
    let Some(text) = valid_utf8_prefix(bytes, scope)? else {
        return Ok(false);
    };
    let text = strip_bom_accounted(text, scope)?;
    let text = account_detection_work(scope, text.len(), || text.trim_start())?;
    if !starts_with_accounted(scope, text.as_bytes(), b"{")? {
        return Ok(false);
    }
    for needle in ["\"asset\"", "\"version\"", "\"2.0\""] {
        let contains = account_detection_work(scope, text.len(), || text.contains(needle))?;
        if !contains {
            return Ok(false);
        }
    }
    Ok(true)
}

fn valid_utf8_prefix<'bytes>(
    bytes: &'bytes [u8],
    scope: &DetectionScope<'_>,
) -> DetectionResult<Option<&'bytes str>> {
    let decoded = account_detection_work(scope, bytes.len(), || std::str::from_utf8(bytes))?;
    Ok(match decoded {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            let prefix = &bytes[..error.valid_up_to()];
            account_detection_work(scope, prefix.len(), || std::str::from_utf8(prefix))?.ok()
        }
        Err(_) => None,
    })
}

fn strip_bom_accounted<'text>(
    text: &'text str,
    scope: &DetectionScope<'_>,
) -> DetectionResult<&'text str> {
    const UTF8_BOM: &str = "\u{feff}";
    let inspected = if text.len() >= UTF8_BOM.len() {
        UTF8_BOM.len()
    } else {
        0
    };
    account_detection_work(scope, inspected, || {
        text.strip_prefix(UTF8_BOM).unwrap_or(text)
    })
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
    account_detection_work(scope, inspected, || bytes.starts_with(prefix))
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
