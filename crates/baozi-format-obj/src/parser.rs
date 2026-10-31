use crate::options::{ObjTextDecodingMode, obj_import_option_key};
use crate::{ObjFailure, mesh_builder, mtl, obj};
use baozi_import::{
    ConformedTextInput, DiagnosticCode, DiagnosticSource, ImportWorkClass, ParseFailure,
    ParseResult, ParseScope, SceneDraft, TextConformanceViolation, TextConformanceViolationKind,
    TextDecodingMode, TextInputLine, TextInputOptions, TextInputRecoveryTracker,
};
use std::ops::Range;

pub(crate) const SCAN_CHUNK_BYTES: usize = 4 * 1024;

pub(crate) fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

pub(crate) fn with_obj_text<'scope, 'session, 'source, T>(
    scope: &'scope ParseScope<'session>,
    source: DiagnosticSource<'source>,
    resource: &'static str,
    action: impl for<'input> FnOnce(
        &'input ConformedTextInput<'scope>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    with_obj_text_options(scope, source, obj_text_options(scope, resource)?, action)
}

pub(crate) fn with_obj_dependency_text_limit<'scope, 'session, 'source, T>(
    scope: &'scope ParseScope<'session>,
    source: DiagnosticSource<'source>,
    resource: &'static str,
    effective_raw_limit: u64,
    action: impl for<'input> FnOnce(
        &'input ConformedTextInput<'scope>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    let options = obj_text_options(scope, resource)?.with_raw_limit(resource, effective_raw_limit);
    with_obj_text_options(scope, source, options, action)
}

fn obj_text_options(
    scope: &ParseScope<'_>,
    resource: &'static str,
) -> ParseResult<TextInputOptions> {
    let selected_mode = scope
        .options(&obj_import_option_key())?
        .text_decoding_mode();
    let decoding_mode = match selected_mode {
        ObjTextDecodingMode::RejectInvalid => TextDecodingMode::RejectInvalid,
        ObjTextDecodingMode::ReplaceInvalidUtf8 => TextDecodingMode::ReplaceInvalidUtf8,
    };
    Ok(TextInputOptions::new(decoding_mode, resource)
        .with_decoded_resource("obj-decoded-utf8-bytes")
        .with_line_resource("line-bytes")
        .with_line_index_resource("obj-line-index-bytes"))
}

fn with_obj_text_options<'scope, 'session, 'source, T>(
    scope: &'scope ParseScope<'session>,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    action: impl for<'input> FnOnce(
        &'input ConformedTextInput<'scope>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    scope
        .with_conformed_text_input(source, options, action)?
        .map_err(|violation| text_conformance_failure(scope, violation))
}

pub(crate) fn authorize_line_recoveries(
    scope: &ParseScope<'_>,
    recoveries: &mut TextInputRecoveryTracker<'_, '_>,
    line: &TextInputLine<'_, '_>,
    allowed_value: Option<Range<usize>>,
    comment_start: Option<usize>,
) -> ParseResult<()> {
    if line.replacements().is_empty() {
        return Ok(());
    }

    let decoded_start = line.decoded_content_range().start;
    let semantic_replacement_count = match comment_start {
        Some(comment_start) => {
            let decoded_comment_start =
                decoded_start.checked_add(comment_start).ok_or_else(|| {
                    scope.format_failure(ObjFailure::internal("comment-offset-overflow"))
                })?;
            line.replacements()
                .partition_point(|replacement| replacement.decoded_start() < decoded_comment_start)
        }
        None => line.replacements().len(),
    };
    for replacement in &line.replacements()[..semantic_replacement_count] {
        let local_start = replacement
            .decoded_start()
            .checked_sub(decoded_start)
            .ok_or_else(|| scope.format_failure(ObjFailure::internal("replacement-before-line")))?;
        let local_end = replacement
            .decoded_end()
            .checked_sub(decoded_start)
            .ok_or_else(|| scope.format_failure(ObjFailure::internal("replacement-before-line")))?;
        let raw_range = replacement.raw_range();
        if allowed_value
            .as_ref()
            .is_some_and(|range| local_start >= range.start && local_end <= range.end)
        {
            let code = DiagnosticCode::new("obj.invalid_utf8_replaced").map_err(|_| {
                scope.format_failure(ObjFailure::internal("invalid-recovery-diagnostic-code"))
            })?;
            recoveries.record_format_recovery(
                replacement,
                code,
                format_args!(
                    "invalid UTF-8 bytes {}..{} were replaced in an OBJ textual value",
                    raw_range.start(),
                    raw_range.end()
                ),
            )?;
            continue;
        }

        return Err(invalid_utf8_failure(scope, raw_range.start()));
    }
    recoveries.discard_replacements(&line.replacements()[semantic_replacement_count..])?;
    Ok(())
}

fn text_conformance_failure(
    scope: &ParseScope<'_>,
    violation: TextConformanceViolation,
) -> ParseFailure {
    let raw_offset = violation.raw_range().map(|range| range.start());
    match violation.kind() {
        TextConformanceViolationKind::InvalidUtf8 => raw_offset.map_or_else(
            || scope.format_failure(ObjFailure::internal("invalid-utf8-range-missing")),
            |offset| invalid_utf8_failure(scope, offset),
        ),
        TextConformanceViolationKind::UnsupportedNewline => document_byte_failure(
            scope,
            raw_offset,
            "unsupported-newline",
            "unsupported-newline-range-missing",
        ),
        TextConformanceViolationKind::ForbiddenControl => document_byte_failure(
            scope,
            raw_offset,
            "forbidden-control-character",
            "forbidden-control-range-missing",
        ),
        TextConformanceViolationKind::ForbiddenBom => document_byte_failure(
            scope,
            raw_offset,
            "forbidden-bom",
            "forbidden-bom-range-missing",
        ),
        TextConformanceViolationKind::NonAscii => {
            document_byte_failure(scope, raw_offset, "non-ascii", "non-ascii-range-missing")
        }
        TextConformanceViolationKind::MissingTextProfile
        | TextConformanceViolationKind::UnsupportedSourceEncoding
        | TextConformanceViolationKind::DecodingModeNotDeclared => {
            scope.format_failure(ObjFailure::internal("text-conformance-contract-violation"))
        }
        _ => scope.format_failure(ObjFailure::internal("unknown-text-conformance-violation")),
    }
}

fn document_byte_failure(
    scope: &ParseScope<'_>,
    raw_offset: Option<u64>,
    reason: &'static str,
    missing_range_reason: &'static str,
) -> ParseFailure {
    raw_offset.map_or_else(
        || scope.format_failure(ObjFailure::internal(missing_range_reason)),
        |offset| scope.format_failure(ObjFailure::invalid_document_byte(reason, offset)),
    )
}

pub(crate) fn scan_token_ranges(
    scope: &ParseScope<'_>,
    line: &str,
    resource: &'static str,
    mut inspect: impl FnMut(usize, char) -> ParseResult<()>,
    mut visit: impl FnMut(usize, usize) -> ParseResult<()>,
) -> ParseResult<Option<usize>> {
    let mut token_start = None;
    let mut chunk_start = 0usize;
    let mut scanned_offsets = [0_u16; SCAN_CHUNK_BYTES];
    let mut scanned_characters = ['\0'; SCAN_CHUNK_BYTES];

    while chunk_start < line.len() {
        let chunk_end = str_chunk_end(line, chunk_start);
        let mut scanned_len = 0usize;
        scope.with_work(
            ImportWorkClass::DecodeWork,
            usize_amount(scope, chunk_end - chunk_start, resource)?,
            |work| -> ParseResult<()> {
                for (relative, character) in line[chunk_start..chunk_end].char_indices() {
                    let offset =
                        u16::try_from(relative).map_err(|_| scope.allocation_failure(resource))?;
                    let stored_offset = scanned_offsets
                        .get_mut(scanned_len)
                        .ok_or_else(|| scope.allocation_failure(resource))?;
                    let stored_character = scanned_characters
                        .get_mut(scanned_len)
                        .ok_or_else(|| scope.allocation_failure(resource))?;
                    *stored_offset = offset;
                    *stored_character = character;
                    scanned_len = scanned_len
                        .checked_add(1)
                        .ok_or_else(|| scope.allocation_failure(resource))?;
                }
                work.complete_all()
            },
        )?;

        // Token visitors can meter their own work, so semantic callbacks run only after the
        // bounded scan capability has closed. The fixed buffer avoids an unmetered allocation or
        // a second UTF-8 decode pass while preserving callback order.
        for (&relative, &character) in scanned_offsets[..scanned_len]
            .iter()
            .zip(&scanned_characters[..scanned_len])
        {
            let index = chunk_start
                .checked_add(usize::from(relative))
                .ok_or_else(|| scope.allocation_failure(resource))?;
            if character == '#' {
                if let Some(start) = token_start.take() {
                    visit(start, index)?;
                }
                return Ok(Some(index));
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
        chunk_start = chunk_end;
    }

    if let Some(start) = token_start {
        visit(start, line.len())?;
    }
    Ok(None)
}

pub(crate) fn read_obj(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let parsed = with_obj_text(
        scope,
        scope.diagnostic_source(),
        "obj-primary-text",
        |text, recoveries| obj::parse(scope, text, recoveries),
    )?;
    let materials = mtl::load_libraries(scope, parsed.mtllibs.as_slice())?;

    mesh_builder::scene_from_parsed(scope, parsed, &materials)
}

fn invalid_utf8_failure(scope: &ParseScope<'_>, raw_offset: u64) -> ParseFailure {
    scope.format_failure(ObjFailure::invalid_utf8_at(raw_offset))
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

fn str_chunk_end(text: &str, start: usize) -> usize {
    let mut end = start
        .checked_add(SCAN_CHUNK_BYTES)
        .map_or(text.len(), |end| end.min(text.len()));
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start { text.len() } else { end }
}
