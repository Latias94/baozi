use baozi_import::{DetectionInconclusive, DetectionOutcome, Probe, ReadConfidence, ReadHint};

pub(crate) fn can_read(probe: &Probe<'_>, hint: &ReadHint) -> DetectionOutcome {
    let extension_match = hint
        .extension
        .as_deref()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("obj"));

    if looks_like_obj(probe.bytes(), probe.is_complete()) {
        DetectionOutcome::Match(ReadConfidence::Likely)
    } else if extension_match {
        DetectionOutcome::Match(ReadConfidence::Maybe)
    } else if !probe.is_complete() {
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    } else {
        DetectionOutcome::NoMatch
    }
}

fn looks_like_obj(bytes: &[u8], probe_is_complete: bool) -> bool {
    let Some((text, bytes_are_valid_utf8)) = valid_utf8_prefix(bytes) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = if probe_is_complete && bytes_are_valid_utf8 {
        text
    } else {
        terminated_lines(text)
    };

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

fn valid_utf8_prefix(bytes: &[u8]) -> Option<(&str, bool)> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some((text, true)),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .ok()
                .map(|text| (text, false))
        }
        Err(_) => None,
    }
}

fn terminated_lines(text: &str) -> &str {
    text.rfind('\n').map_or("", |newline| &text[..=newline])
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
