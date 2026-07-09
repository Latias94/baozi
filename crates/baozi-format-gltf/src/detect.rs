use baozi_core::{BaoziError, Result};
use baozi_import::{ReadConfidence, ReadHint};
use baozi_io::ReadSeek;
use std::io::{Read, SeekFrom};

const PROBE_BYTES: u64 = 4096;

pub(crate) fn can_read(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
    let original = input
        .stream_position()
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    let result = detect_stream(input, hint);

    input
        .seek(SeekFrom::Start(original))
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    result
}

fn detect_stream(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
    let mut bytes = Vec::new();
    let mut limited = input.take(PROBE_BYTES);
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))?;

    let extension_match = hint.extension.as_deref().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("gltf") || extension.eq_ignore_ascii_case("glb")
    });

    if bytes.starts_with(b"glTF") {
        Ok(ReadConfidence::Certain)
    } else if looks_like_gltf_json(&bytes) {
        Ok(ReadConfidence::Likely)
    } else if extension_match {
        Ok(ReadConfidence::Maybe)
    } else {
        Ok(ReadConfidence::No)
    }
}

fn looks_like_gltf_json(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    text.starts_with('{')
        && text.contains("\"asset\"")
        && text.contains("\"version\"")
        && text.contains("\"2.0\"")
}
