use baozi_import::{DetectionInconclusive, DetectionOutcome, ReadConfidence, ReadHint};

pub(crate) fn can_read(bytes: &[u8], source_len: u64, hint: &ReadHint) -> DetectionOutcome {
    let extension_match = hint.extension.as_deref().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("gltf") || extension.eq_ignore_ascii_case("glb")
    });

    if bytes.starts_with(b"glTF") {
        DetectionOutcome::Match(ReadConfidence::Certain)
    } else if looks_like_gltf_json(bytes) {
        DetectionOutcome::Match(ReadConfidence::Likely)
    } else if extension_match {
        DetectionOutcome::Match(ReadConfidence::Maybe)
    } else if bytes.len() as u64 != source_len {
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    } else {
        DetectionOutcome::NoMatch
    }
}

fn looks_like_gltf_json(bytes: &[u8]) -> bool {
    let Some(text) = valid_utf8_prefix(bytes) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    text.starts_with('{')
        && text.contains("\"asset\"")
        && text.contains("\"version\"")
        && text.contains("\"2.0\"")
}

fn valid_utf8_prefix(bytes: &[u8]) -> Option<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}
