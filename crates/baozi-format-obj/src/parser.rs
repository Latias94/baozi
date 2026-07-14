use crate::options::{ObjTextDecodingMode, obj_import_option_key};
use crate::{ObjFailure, mesh_builder, mtl, obj};
use baozi_core::SceneDraft;
use baozi_import::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticSource, ImportStorageClass, ImportWorkClass,
    MeteredDecodedText, MeteredVec, ParseFailure, ParseResult, ParseScope,
};
use std::ops::Range;
use std::str::Utf8Error;

pub(crate) const SCAN_CHUNK_BYTES: usize = 4 * 1024;

pub(crate) fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

pub(crate) struct DecodedText<'scope> {
    text: MeteredDecodedText<'scope>,
    line_count: usize,
    replacements: MeteredVec<'scope, TextReplacement>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TextReplacement {
    raw_range: DiagnosticByteRange,
    decoded_start: usize,
    decoded_end: usize,
}

pub(crate) struct DecodedLine<'text> {
    text: &'text str,
    decoded_start: usize,
    replacements: &'text [TextReplacement],
}

impl<'text> DecodedLine<'text> {
    pub(crate) const fn text(&self) -> &'text str {
        self.text
    }

    pub(crate) fn comment_start(&self) -> Option<usize> {
        self.text.find('#')
    }
}

pub(crate) enum TextDecodeFailure {
    Scope(ParseFailure),
    InvalidUtf8 { source: Utf8Error, base_offset: u64 },
}

pub(crate) fn decode_utf8<'scope>(
    scope: &'scope ParseScope<'_>,
    bytes: &[u8],
    resource: &'static str,
) -> Result<DecodedText<'scope>, TextDecodeFailure> {
    let selected_mode = scope
        .options(&obj_import_option_key())
        .map_err(TextDecodeFailure::Scope)?
        .text_decoding_mode();
    let replace_invalid = selected_mode == ObjTextDecodingMode::ReplaceInvalidUtf8;
    let input_start = usize::from(bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let plan = plan_decode(scope, bytes, input_start, replace_invalid, resource)?;

    let mut text = scope
        .metered_decoded_text(plan.decoded_bytes, resource)
        .map_err(TextDecodeFailure::Scope)?;
    let mut replacements =
        scope.metered_vec(ImportStorageClass::LineIndex, "obj-text-replacement-index");
    replacements
        .try_reserve(plan.replacement_count)
        .map_err(TextDecodeFailure::Scope)?;

    let mut offset = 0usize;
    offset = offset
        .checked_add(input_start)
        .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;

    while offset < bytes.len() {
        let end = utf8_chunk_end(bytes, offset);
        let permit = scope
            .begin_work(
                ImportWorkClass::DecodeWork,
                usize_amount(scope, end - offset, resource).map_err(TextDecodeFailure::Scope)?,
            )
            .map_err(TextDecodeFailure::Scope)?;

        while offset < end {
            match classify_utf8_unit(bytes, offset, end) {
                Utf8Unit::Valid(length) => {
                    let unit_end = offset.checked_add(length).ok_or_else(|| {
                        TextDecodeFailure::Scope(scope.allocation_failure(resource))
                    })?;
                    let unit = bytes.get(offset..unit_end).ok_or_else(|| {
                        TextDecodeFailure::Scope(scope.allocation_failure(resource))
                    })?;
                    let decoded = std::str::from_utf8(unit)
                        .map_err(|_| invalid_utf8_error(scope, bytes, offset, resource))?;
                    append_preallocated_text(&mut text, decoded)
                        .map_err(TextDecodeFailure::Scope)?;
                    offset = unit_end;
                }
                Utf8Unit::Invalid(length) => {
                    let raw_end = offset.checked_add(length).ok_or_else(|| {
                        TextDecodeFailure::Scope(scope.allocation_failure(resource))
                    })?;
                    let raw_start =
                        usize_amount(scope, offset, resource).map_err(TextDecodeFailure::Scope)?;
                    let raw_end =
                        usize_amount(scope, raw_end, resource).map_err(TextDecodeFailure::Scope)?;
                    let raw_range =
                        DiagnosticByteRange::new(raw_start, raw_end).map_err(|_| {
                            TextDecodeFailure::Scope(scope.format_failure(ObjFailure::internal(
                                "invalid-replacement-raw-range",
                            )))
                        })?;
                    let decoded_start = text.len();
                    append_preallocated_text(&mut text, "\u{fffd}")
                        .map_err(TextDecodeFailure::Scope)?;
                    let decoded_end = text.len();
                    if replacements.len() >= replacements.capacity() {
                        return Err(TextDecodeFailure::Scope(scope.format_failure(
                            ObjFailure::internal("replacement-plan-capacity-exhausted"),
                        )));
                    }
                    replacements
                        .try_push_reserved(TextReplacement {
                            raw_range,
                            decoded_start,
                            decoded_end,
                        })
                        .map_err(TextDecodeFailure::Scope)?;
                    offset = usize::try_from(raw_end).map_err(|_| {
                        TextDecodeFailure::Scope(scope.allocation_failure(resource))
                    })?;
                }
            }
        }
        permit.complete().map_err(TextDecodeFailure::Scope)?;
    }

    if text.len() != plan.decoded_bytes || replacements.len() != plan.replacement_count {
        return Err(TextDecodeFailure::Scope(
            scope.format_failure(ObjFailure::internal("text-decode-plan-mismatch")),
        ));
    }

    Ok(DecodedText {
        text: text.finish_exact().map_err(TextDecodeFailure::Scope)?,
        line_count: plan.line_count,
        replacements,
    })
}

pub(crate) fn scan_lines(
    scope: &ParseScope<'_>,
    text: &DecodedText<'_>,
    mut visit: impl FnMut(DecodedLine<'_>, u32) -> ParseResult<()>,
) -> ParseResult<()> {
    let decoded = text.text.as_str();
    let bytes = decoded.as_bytes();
    let mut line_start = 0usize;
    let mut line_number = 1u32;
    let mut replacement_start = 0usize;
    let mut scan_start = 0usize;
    let mut visited_lines = 0usize;

    while scan_start < bytes.len() {
        scope.poll()?;
        let scan_end = scan_start
            .checked_add(SCAN_CHUNK_BYTES)
            .map_or(bytes.len(), |end| end.min(bytes.len()));
        for (relative, byte) in bytes[scan_start..scan_end].iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let newline = scan_start
                .checked_add(relative)
                .ok_or_else(|| scope.allocation_failure("obj-line-offset"))?;
            let line_end = if newline > line_start
                && newline.checked_sub(1).and_then(|index| bytes.get(index)) == Some(&b'\r')
            {
                newline - 1
            } else {
                newline
            };
            let replacement_end = replacement_end_for_line(
                scope,
                text.replacements.as_slice(),
                replacement_start,
                line_end,
            )?;
            let line_text = decoded.get(line_start..line_end).ok_or_else(|| {
                scope.format_failure(ObjFailure::internal("text-line-range-out-of-bounds"))
            })?;
            visit(
                DecodedLine {
                    text: line_text,
                    decoded_start: line_start,
                    replacements: text
                        .replacements
                        .as_slice()
                        .get(replacement_start..replacement_end)
                        .ok_or_else(|| {
                            scope.format_failure(ObjFailure::internal(
                                "replacement-line-range-out-of-bounds",
                            ))
                        })?,
                },
                line_number,
            )?;
            visited_lines = visited_lines
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure("obj-line-count"))?;
            replacement_start = replacement_end;
            line_start = newline
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure("obj-line-offset"))?;
            line_number = line_number
                .checked_add(1)
                .ok_or_else(|| scope.resource_limit_exceeded("line-count", u64::from(u32::MAX)))?;
        }
        scan_start = scan_end;
    }

    if line_start < decoded.len() {
        let replacement_end = replacement_end_for_line(
            scope,
            text.replacements.as_slice(),
            replacement_start,
            decoded.len(),
        )?;
        let line_text = decoded.get(line_start..).ok_or_else(|| {
            scope.format_failure(ObjFailure::internal("text-line-range-out-of-bounds"))
        })?;
        visit(
            DecodedLine {
                text: line_text,
                decoded_start: line_start,
                replacements: text
                    .replacements
                    .as_slice()
                    .get(replacement_start..replacement_end)
                    .ok_or_else(|| {
                        scope.format_failure(ObjFailure::internal(
                            "replacement-line-range-out-of-bounds",
                        ))
                    })?,
            },
            line_number,
        )?;
        visited_lines = visited_lines
            .checked_add(1)
            .ok_or_else(|| scope.allocation_failure("obj-line-count"))?;
        replacement_start = replacement_end;
    }

    if visited_lines != text.line_count || replacement_start != text.replacements.len() {
        return Err(scope.format_failure(ObjFailure::internal("text-line-scan-mismatch")));
    }
    Ok(())
}

fn replacement_end_for_line(
    scope: &ParseScope<'_>,
    replacements: &[TextReplacement],
    start: usize,
    decoded_line_end: usize,
) -> ParseResult<usize> {
    let mut end = start;
    while let Some(replacement) = replacements.get(end) {
        if replacement.decoded_start >= decoded_line_end {
            break;
        }
        if replacement.decoded_end > decoded_line_end {
            return Err(
                scope.format_failure(ObjFailure::internal("replacement-crosses-line-boundary"))
            );
        }
        end = end
            .checked_add(1)
            .ok_or_else(|| scope.allocation_failure("obj-replacement-index"))?;
    }
    Ok(end)
}

pub(crate) fn authorize_line_recoveries<'source>(
    scope: &ParseScope<'_>,
    line: &DecodedLine<'_>,
    allowed_value: Option<Range<usize>>,
    comment_start: Option<usize>,
    mut source: impl FnMut() -> DiagnosticSource<'source>,
) -> ParseResult<()> {
    for replacement in line.replacements {
        let local_start = replacement
            .decoded_start
            .checked_sub(line.decoded_start)
            .ok_or_else(|| scope.format_failure(ObjFailure::internal("replacement-before-line")))?;
        let local_end = replacement
            .decoded_end
            .checked_sub(line.decoded_start)
            .ok_or_else(|| scope.format_failure(ObjFailure::internal("replacement-before-line")))?;
        if comment_start.is_some_and(|comment| local_start >= comment) {
            continue;
        }
        if allowed_value
            .as_ref()
            .is_some_and(|range| local_start >= range.start && local_end <= range.end)
        {
            let code = DiagnosticCode::new("obj.invalid_utf8_replaced").map_err(|_| {
                scope.format_failure(ObjFailure::internal("invalid-recovery-diagnostic-code"))
            })?;
            scope.record_format_recovery(
                code,
                source(),
                replacement.raw_range,
                format_args!(
                    "invalid UTF-8 bytes {}..{} were replaced in an OBJ textual value",
                    replacement.raw_range.start(),
                    replacement.raw_range.end()
                ),
            )?;
            continue;
        }

        let diagnostic_source = source();
        return Err(invalid_utf8_failure(
            scope,
            diagnostic_source.bytes(),
            replacement.raw_range.start(),
        ));
    }
    Ok(())
}

pub(crate) fn scan_token_ranges(
    scope: &ParseScope<'_>,
    line: &str,
    resource: &'static str,
    mut inspect: impl FnMut(usize, char) -> ParseResult<()>,
    mut visit: impl FnMut(usize, usize) -> ParseResult<()>,
) -> ParseResult<()> {
    let mut token_start = None;
    let mut chunk_start = 0usize;

    while chunk_start < line.len() {
        let chunk_end = str_chunk_end(line, chunk_start);
        let permit = scope.begin_work(
            ImportWorkClass::DecodeWork,
            usize_amount(scope, chunk_end - chunk_start, resource)?,
        )?;
        for (relative, character) in line[chunk_start..chunk_end].char_indices() {
            let index = chunk_start
                .checked_add(relative)
                .ok_or_else(|| scope.allocation_failure(resource))?;
            if character == '#' {
                if let Some(start) = token_start.take() {
                    visit(start, index)?;
                }
                permit.complete()?;
                return Ok(());
            }
            inspect(index, character)?;
            if character.is_whitespace() {
                if let Some(start) = token_start.take() {
                    visit(start, index)?;
                }
            } else if token_start.is_none() {
                token_start = Some(index);
            }
        }
        permit.complete()?;
        chunk_start = chunk_end;
    }

    if let Some(start) = token_start {
        visit(start, line.len())?;
    }
    Ok(())
}

pub(crate) fn read_obj(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let text = match decode_utf8(scope, scope.primary_bytes(), "obj-primary-text") {
        Ok(text) => text,
        Err(TextDecodeFailure::Scope(failure)) => return Err(failure),
        Err(TextDecodeFailure::InvalidUtf8 {
            source,
            base_offset,
        }) => {
            let failure = ObjFailure::invalid_utf8_at(source, base_offset)
                .ok_or_else(|| scope.allocation_failure("obj-utf8-byte-offset"))?;
            return Err(scope.format_failure(failure));
        }
    };
    let parsed = obj::parse(scope, &text)?;
    drop(text);
    let materials = mtl::load_libraries(scope, parsed.mtllibs.as_slice())?;

    mesh_builder::scene_from_parsed(scope, parsed, &materials)
}

#[derive(Debug, Clone, Copy)]
struct DecodePlan {
    decoded_bytes: usize,
    line_count: usize,
    replacement_count: usize,
}

fn plan_decode(
    scope: &ParseScope<'_>,
    bytes: &[u8],
    input_start: usize,
    replace_invalid: bool,
    resource: &'static str,
) -> Result<DecodePlan, TextDecodeFailure> {
    let mut decoded_bytes = 0usize;
    let mut line_count = 0usize;
    let mut replacement_count = 0usize;
    let mut line_bytes = 0usize;
    let mut line_raw_start = input_start;
    let mut offset = input_start;

    while offset < bytes.len() {
        let end = utf8_chunk_end(bytes, offset);
        let permit = scope
            .begin_work(
                ImportWorkClass::DecodeWork,
                usize_amount(scope, end - offset, resource).map_err(TextDecodeFailure::Scope)?,
            )
            .map_err(TextDecodeFailure::Scope)?;
        while offset < end {
            let unit = classify_utf8_unit(bytes, offset, end);
            if let Utf8Unit::Valid(length) = unit {
                validate_text_unit(scope, bytes, offset, length, resource)?;
            }
            let (raw_length, decoded_length) = match unit {
                Utf8Unit::Valid(length) => (length, length),
                Utf8Unit::Invalid(length) if replace_invalid => {
                    replacement_count = replacement_count.checked_add(1).ok_or_else(|| {
                        TextDecodeFailure::Scope(scope.allocation_failure(resource))
                    })?;
                    (length, '\u{fffd}'.len_utf8())
                }
                Utf8Unit::Invalid(_) => {
                    return Err(invalid_utf8_error(scope, bytes, offset, resource));
                }
            };

            if matches!(unit, Utf8Unit::Valid(1)) && bytes.get(offset) == Some(&b'\n') {
                let has_cr = offset > line_raw_start
                    && offset.checked_sub(1).and_then(|index| bytes.get(index)) == Some(&b'\r');
                let logical_line_bytes = line_bytes.saturating_sub(usize::from(has_cr));
                enforce_line_limit(scope, logical_line_bytes, scope.limits().max_line_bytes)
                    .map_err(TextDecodeFailure::Scope)?;
                line_count = line_count.checked_add(1).ok_or_else(|| {
                    TextDecodeFailure::Scope(scope.allocation_failure("obj-line-count"))
                })?;
                line_bytes = 0;
                line_raw_start = offset.checked_add(1).ok_or_else(|| {
                    TextDecodeFailure::Scope(scope.allocation_failure("obj-line-offset"))
                })?;
            } else {
                line_bytes = line_bytes
                    .checked_add(decoded_length)
                    .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;
            }
            decoded_bytes = decoded_bytes
                .checked_add(decoded_length)
                .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;
            offset = offset
                .checked_add(raw_length)
                .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;
        }
        permit.complete().map_err(TextDecodeFailure::Scope)?;
    }

    if line_raw_start < bytes.len() {
        enforce_line_limit(scope, line_bytes, scope.limits().max_line_bytes)
            .map_err(TextDecodeFailure::Scope)?;
        line_count = line_count
            .checked_add(1)
            .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure("obj-line-count")))?;
    }

    enforce_decode_limit(
        scope,
        decoded_bytes,
        scope.limits().max_decoded_utf8_bytes,
        "obj-decoded-utf8-bytes",
    )
    .map_err(TextDecodeFailure::Scope)?;
    let line_index_bytes = checked_index_bytes(scope, replacement_count)?;
    enforce_decode_limit(
        scope,
        line_index_bytes,
        scope.limits().max_line_index_bytes,
        "obj-line-index-bytes",
    )
    .map_err(TextDecodeFailure::Scope)?;

    Ok(DecodePlan {
        decoded_bytes,
        line_count,
        replacement_count,
    })
}

#[derive(Debug, Clone, Copy)]
enum Utf8Unit {
    Valid(usize),
    Invalid(usize),
}

fn classify_utf8_unit(bytes: &[u8], offset: usize, end: usize) -> Utf8Unit {
    let Some(chunk) = bytes.get(offset..end) else {
        return Utf8Unit::Invalid(1);
    };
    let Some(&lead) = chunk.first() else {
        return Utf8Unit::Invalid(1);
    };
    if lead.is_ascii() {
        return Utf8Unit::Valid(1);
    }

    let (width, second_min, second_max) = match lead {
        0xc2..=0xdf => (2, 0x80, 0xbf),
        0xe0 => (3, 0xa0, 0xbf),
        0xe1..=0xec | 0xee..=0xef => (3, 0x80, 0xbf),
        0xed => (3, 0x80, 0x9f),
        0xf0 => (4, 0x90, 0xbf),
        0xf1..=0xf3 => (4, 0x80, 0xbf),
        0xf4 => (4, 0x80, 0x8f),
        _ => return Utf8Unit::Invalid(1),
    };
    let Some(&second) = chunk.get(1) else {
        return Utf8Unit::Invalid(1);
    };
    if !(second_min..=second_max).contains(&second) {
        return Utf8Unit::Invalid(1);
    }
    if width == 2 {
        return Utf8Unit::Valid(2);
    }

    let Some(&third) = chunk.get(2) else {
        return Utf8Unit::Invalid(2);
    };
    if !is_utf8_continuation(third) {
        return Utf8Unit::Invalid(2);
    }
    if width == 3 {
        return Utf8Unit::Valid(3);
    }

    let Some(&fourth) = chunk.get(3) else {
        return Utf8Unit::Invalid(3);
    };
    if !is_utf8_continuation(fourth) {
        return Utf8Unit::Invalid(3);
    }
    Utf8Unit::Valid(4)
}

fn invalid_utf8_error(
    scope: &ParseScope<'_>,
    bytes: &[u8],
    offset: usize,
    resource: &'static str,
) -> TextDecodeFailure {
    match bytes.get(offset..).map(std::str::from_utf8) {
        Some(Err(source)) => match usize_amount(scope, offset, resource) {
            Ok(base_offset) => TextDecodeFailure::InvalidUtf8 {
                source,
                base_offset,
            },
            Err(failure) => TextDecodeFailure::Scope(failure),
        },
        Some(Ok(_)) | None => TextDecodeFailure::Scope(
            scope.format_failure(ObjFailure::internal("utf8-classifier-disagreement")),
        ),
    }
}

fn append_preallocated_text(output: &mut MeteredDecodedText<'_>, value: &str) -> ParseResult<()> {
    output.try_push_str_reserved(value)
}

fn validate_text_unit(
    scope: &ParseScope<'_>,
    bytes: &[u8],
    offset: usize,
    length: usize,
    resource: &'static str,
) -> Result<(), TextDecodeFailure> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;
    let unit = bytes
        .get(offset..end)
        .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure(resource)))?;
    let decoded = std::str::from_utf8(unit)
        .map_err(|_| invalid_utf8_error(scope, bytes, offset, resource))?;
    let Some(character) = decoded.chars().next() else {
        return Err(TextDecodeFailure::Scope(scope.format_failure(
            ObjFailure::internal("empty-classified-utf8-unit"),
        )));
    };
    let allowed_control = matches!(character, '\t' | '\n' | '\r');
    if character.is_control() && !allowed_control {
        return Err(invalid_document_byte_failure(
            scope,
            offset,
            "forbidden-control-character",
            resource,
        ));
    }
    if character == '\r' && bytes.get(end) != Some(&b'\n') {
        return Err(invalid_document_byte_failure(
            scope,
            offset,
            "unsupported-newline",
            resource,
        ));
    }
    Ok(())
}

fn invalid_document_byte_failure(
    scope: &ParseScope<'_>,
    offset: usize,
    reason: &'static str,
    resource: &'static str,
) -> TextDecodeFailure {
    match usize_amount(scope, offset, resource) {
        Ok(byte_offset) => TextDecodeFailure::Scope(
            scope.format_failure(ObjFailure::invalid_document_byte(reason, byte_offset)),
        ),
        Err(failure) => TextDecodeFailure::Scope(failure),
    }
}

fn invalid_utf8_failure(scope: &ParseScope<'_>, bytes: &[u8], raw_offset: u64) -> ParseFailure {
    let Ok(offset) = usize::try_from(raw_offset) else {
        return scope.allocation_failure("obj-utf8-byte-offset");
    };
    match bytes.get(offset..).map(std::str::from_utf8) {
        Some(Err(source)) => ObjFailure::invalid_utf8_at(source, raw_offset).map_or_else(
            || scope.allocation_failure("obj-utf8-byte-offset"),
            |failure| scope.format_failure(failure),
        ),
        Some(Ok(_)) | None => {
            scope.format_failure(ObjFailure::internal("utf8-replacement-source-disagreement"))
        }
    }
}

fn checked_index_bytes(
    scope: &ParseScope<'_>,
    replacement_count: usize,
) -> Result<usize, TextDecodeFailure> {
    replacement_count
        .checked_mul(std::mem::size_of::<TextReplacement>())
        .ok_or_else(|| TextDecodeFailure::Scope(scope.allocation_failure("obj-line-index-bytes")))
}

fn enforce_decode_limit(
    scope: &ParseScope<'_>,
    attempted: usize,
    limit: u64,
    resource: &'static str,
) -> ParseResult<()> {
    scope.enforce_resource_limit(resource, usize_amount(scope, attempted, resource)?, limit)
}

pub(crate) fn max_text_value_bytes(scope: &ParseScope<'_>) -> usize {
    scope.limits().max_string_bytes
}

pub(crate) fn enforce_text_value_limit(
    scope: &ParseScope<'_>,
    bytes: usize,
    _resource: &'static str,
) -> ParseResult<()> {
    scope.enforce_resource_limit(
        "string-bytes",
        usize_amount(scope, bytes, "string-bytes")?,
        usize_amount(scope, scope.limits().max_string_bytes, "string-bytes")?,
    )
}

fn utf8_chunk_end(bytes: &[u8], start: usize) -> usize {
    let candidate = start
        .checked_add(SCAN_CHUNK_BYTES)
        .map_or(bytes.len(), |end| end.min(bytes.len()));
    if candidate == bytes.len() {
        return candidate;
    }

    let mut end = candidate;
    let mut backed_up = 0usize;
    while end > start
        && backed_up < 3
        && bytes
            .get(end)
            .is_some_and(|byte| is_utf8_continuation(*byte))
    {
        end -= 1;
        backed_up += 1;
    }
    if end == start { candidate } else { end }
}

fn str_chunk_end(text: &str, start: usize) -> usize {
    let mut end = start
        .checked_add(SCAN_CHUNK_BYTES)
        .map_or(text.len(), |end| end.min(text.len()));
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start { text.len() } else { end }
}

const fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn enforce_line_limit(scope: &ParseScope<'_>, attempted: usize, limit: usize) -> ParseResult<()> {
    scope.enforce_resource_limit(
        "line-bytes",
        usize_amount(scope, attempted, "obj-line-bytes")?,
        usize_amount(scope, limit, "obj-line-bytes")?,
    )
}
