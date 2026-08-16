use baozi_import::{
    DetectionInconclusive, DetectionOutcome, DetectionResult, DetectionScope, Probe,
    ReadConfidence, ReadHint,
};

pub(crate) fn can_read(
    probe: &Probe<'_>,
    hint: &ReadHint,
    scope: &DetectionScope<'_>,
) -> DetectionResult<DetectionOutcome> {
    let extension_match = hint
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("obj"));

    if looks_like_obj(probe.bytes(), probe.is_complete(), scope)? {
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

fn looks_like_obj(
    bytes: &[u8],
    probe_is_complete: bool,
    scope: &DetectionScope<'_>,
) -> DetectionResult<bool> {
    let Some((text, bytes_are_valid_utf8)) = valid_utf8_prefix(bytes, scope)? else {
        return Ok(false);
    };
    let text = strip_bom_accounted(text, scope)?;
    let text = if probe_is_complete && bytes_are_valid_utf8 {
        text
    } else {
        terminated_lines(text, scope)?
    };

    account_detection_work(scope, text.len(), || scan_obj_text(text))
}

fn strip_bom_accounted<'text>(
    text: &'text str,
    scope: &DetectionScope<'_>,
) -> DetectionResult<&'text str> {
    const UTF8_BOM: &str = "\u{feff}";
    let inspected = text.len().min(UTF8_BOM.len());
    if inspected == 0 {
        return Ok(text);
    }
    scope.with_work(inspected as u64, || {
        Ok(text.strip_prefix(UTF8_BOM).unwrap_or(text))
    })
}

fn scan_obj_text(text: &str) -> bool {
    let mut position_count = 0usize;
    let mut has_face = false;

    for raw_line in text.lines().take(64) {
        let line = raw_line.split_once('#').map_or(raw_line, |(head, _)| head);
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else {
            continue;
        };
        match first {
            "v" if has_float_components(&mut tokens, 3) => position_count += 1,
            "f" if has_valid_face(&mut tokens) => has_face = true,
            _ => {}
        }

        if (position_count > 0 && has_face) || position_count >= 3 {
            return true;
        }
    }

    false
}

fn valid_utf8_prefix<'bytes>(
    bytes: &'bytes [u8],
    scope: &DetectionScope<'_>,
) -> DetectionResult<Option<(&'bytes str, bool)>> {
    let decoded = account_detection_work(scope, bytes.len(), || std::str::from_utf8(bytes))?;
    Ok(match decoded {
        Ok(text) => Some((text, true)),
        Err(error) if error.error_len().is_none() => {
            let prefix = &bytes[..error.valid_up_to()];
            account_detection_work(scope, prefix.len(), || std::str::from_utf8(prefix))?
                .ok()
                .map(|text| (text, false))
        }
        Err(_) => None,
    })
}

fn terminated_lines<'text>(
    text: &'text str,
    scope: &DetectionScope<'_>,
) -> DetectionResult<&'text str> {
    let newline = account_detection_work(scope, text.len(), || text.rfind('\n'))?;
    Ok(newline.map_or("", |newline| &text[..=newline]))
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

fn has_float_components<'a>(mut tokens: impl Iterator<Item = &'a str>, minimum: usize) -> bool {
    (0..minimum).all(|_| {
        tokens
            .next()
            .is_some_and(|token| token.parse::<f32>().is_ok_and(f32::is_finite))
    })
}

fn has_valid_face<'a>(tokens: impl Iterator<Item = &'a str>) -> bool {
    let mut count = 0usize;
    for token in tokens {
        if !is_face_vertex(token) {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn is_face_vertex(token: &str) -> bool {
    let mut parts = token.split('/');
    let Some(position) = parts.next() else {
        return false;
    };
    let texcoord = parts.next();
    let normal = parts.next();

    parts.next().is_none()
        && is_nonzero_index(position)
        && texcoord.is_none_or(|index| index.is_empty() || is_nonzero_index(index))
        && normal.is_none_or(|index| index.is_empty() || is_nonzero_index(index))
}

fn is_nonzero_index(token: &str) -> bool {
    token.parse::<i64>().is_ok_and(|index| index != 0)
}
