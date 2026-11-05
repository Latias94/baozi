use crate::options::{ObjImportOptions, ObjTextDecodingMode};
use crate::{ObjFailure, mesh_builder, mtl, obj};
use baozi_import::{
    ConformedTextInput, DiagnosticCode, DiagnosticSource, ImportWorkClass, ParseChildren,
    ParseContext, ParseFailure, ParsePieceError, ParseResult, ParseServices, ParseWorkLayout,
    TextConformanceViolation, TextConformanceViolationKind, TextDecodingMode, TextInputLine,
    TextInputOptions, TextInputRecoveryTracker,
};
use std::ops::Range;

pub(crate) const SCAN_CHUNK_BYTES: usize = 4 * 1024;

// Bind once for the parser region; each physical scan and token still admits its own work.
pub(crate) struct TokenWorkLayouts<'op> {
    pub(crate) tokens: ParseWorkLayout<'op, 2>,
    pub(crate) scan: ParseWorkLayout<'op, 1>,
}

impl<'op> TokenWorkLayouts<'op> {
    pub(crate) fn bind(children: &mut ParseChildren<'_, 'op, '_>) -> ParseResult<Self> {
        Ok(Self {
            tokens: children.bind_group([ImportWorkClass::Tokens, ImportWorkClass::TokenBytes])?,
            scan: children.bind_group([ImportWorkClass::DecodeWork])?,
        })
    }
}

// A document reuses these initialized buffers; scans read only the populated prefix.
pub(crate) struct TokenScanScratch {
    offsets: [u16; SCAN_CHUNK_BYTES],
    characters: [char; SCAN_CHUNK_BYTES],
}

impl TokenScanScratch {
    pub(crate) fn new() -> Self {
        Self {
            offsets: [0_u16; SCAN_CHUNK_BYTES],
            characters: ['\0'; SCAN_CHUNK_BYTES],
        }
    }
}

pub(crate) fn usize_amount(
    exec: &ParseServices<'_, '_, '_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| exec.allocation_failure(resource))
}

pub(crate) fn with_obj_text<'region, 'op, 'session, 'source, T>(
    exec: &mut ParseChildren<'region, 'op, 'session>,
    options: &ObjImportOptions,
    source: DiagnosticSource<'source>,
    resource: &'static str,
    action: impl for<'input> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'input ConformedTextInput<'session>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    with_obj_text_options(exec, source, obj_text_options(options, resource), action)
}

pub(crate) fn with_obj_dependency_text_limit<'region, 'op, 'session, 'source, T>(
    exec: &mut ParseChildren<'region, 'op, 'session>,
    options: &ObjImportOptions,
    source: DiagnosticSource<'source>,
    resource: &'static str,
    effective_raw_limit: u64,
    action: impl for<'input> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'input ConformedTextInput<'session>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    let options = obj_text_options(options, resource).with_raw_limit(resource, effective_raw_limit);
    with_obj_text_options(exec, source, options, action)
}

fn obj_text_options(options: &ObjImportOptions, resource: &'static str) -> TextInputOptions {
    let selected_mode = options.text_decoding_mode();
    let decoding_mode = match selected_mode {
        ObjTextDecodingMode::RejectInvalid => TextDecodingMode::RejectInvalid,
        ObjTextDecodingMode::ReplaceInvalidUtf8 => TextDecodingMode::ReplaceInvalidUtf8,
    };
    TextInputOptions::new(decoding_mode, resource)
        .with_decoded_resource("obj-decoded-utf8-bytes")
        .with_line_resource("line-bytes")
        .with_line_index_resource("obj-line-index-bytes")
}

fn with_obj_text_options<'region, 'op, 'session, 'source, T>(
    exec: &mut ParseChildren<'region, 'op, 'session>,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    action: impl for<'input> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'input ConformedTextInput<'session>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<T> {
    match exec.with_conformed_text_input(source, options, action)? {
        Ok(value) => Ok(value),
        Err(violation) => Err(text_conformance_failure(&mut exec.services()?, violation)),
    }
}

pub(crate) fn authorize_line_recoveries<'session>(
    exec: &mut ParseServices<'_, '_, 'session>,
    recoveries: &mut TextInputRecoveryTracker<'_, 'session>,
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
                    exec.format_failure(ObjFailure::internal("comment-offset-overflow"))
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
            .ok_or_else(|| exec.format_failure(ObjFailure::internal("replacement-before-line")))?;
        let local_end = replacement
            .decoded_end()
            .checked_sub(decoded_start)
            .ok_or_else(|| exec.format_failure(ObjFailure::internal("replacement-before-line")))?;
        let raw_range = replacement.raw_range();
        if allowed_value
            .as_ref()
            .is_some_and(|range| local_start >= range.start && local_end <= range.end)
        {
            let code = DiagnosticCode::new("obj.invalid_utf8_replaced").map_err(|_| {
                exec.format_failure(ObjFailure::internal("invalid-recovery-diagnostic-code"))
            })?;
            recoveries.record_format_recovery(
                exec,
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

        return Err(invalid_utf8_failure(exec, raw_range.start()));
    }
    recoveries.discard_replacements(exec, &line.replacements()[semantic_replacement_count..])?;
    Ok(())
}

fn text_conformance_failure(
    exec: &mut ParseServices<'_, '_, '_>,
    violation: TextConformanceViolation,
) -> ParseFailure {
    let raw_offset = violation.raw_range().map(|range| range.start());
    match violation.kind() {
        TextConformanceViolationKind::InvalidUtf8 => match raw_offset {
            Some(offset) => invalid_utf8_failure(exec, offset),
            None => exec.format_failure(ObjFailure::internal("invalid-utf8-range-missing")),
        },
        TextConformanceViolationKind::UnsupportedNewline => document_byte_failure(
            exec,
            raw_offset,
            "unsupported-newline",
            "unsupported-newline-range-missing",
        ),
        TextConformanceViolationKind::ForbiddenControl => document_byte_failure(
            exec,
            raw_offset,
            "forbidden-control-character",
            "forbidden-control-range-missing",
        ),
        TextConformanceViolationKind::ForbiddenBom => document_byte_failure(
            exec,
            raw_offset,
            "forbidden-bom",
            "forbidden-bom-range-missing",
        ),
        TextConformanceViolationKind::NonAscii => {
            document_byte_failure(exec, raw_offset, "non-ascii", "non-ascii-range-missing")
        }
        TextConformanceViolationKind::MissingTextProfile
        | TextConformanceViolationKind::UnsupportedSourceEncoding
        | TextConformanceViolationKind::DecodingModeNotDeclared => {
            exec.format_failure(ObjFailure::internal("text-conformance-contract-violation"))
        }
        _ => exec.format_failure(ObjFailure::internal("unknown-text-conformance-violation")),
    }
}

fn document_byte_failure(
    exec: &mut ParseServices<'_, '_, '_>,
    raw_offset: Option<u64>,
    reason: &'static str,
    missing_range_reason: &'static str,
) -> ParseFailure {
    match raw_offset {
        Some(offset) => exec.format_failure(ObjFailure::invalid_document_byte(reason, offset)),
        None => exec.format_failure(ObjFailure::internal(missing_range_reason)),
    }
}

pub(crate) fn scan_token_ranges<'region, 'op, 'session>(
    exec: &mut ParseChildren<'region, 'op, 'session>,
    scan_work: &ParseWorkLayout<'op, 1>,
    scratch: &mut TokenScanScratch,
    line: &str,
    resource: &'static str,
    mut inspect: impl FnMut(usize, char) -> Result<(), ParsePieceError<ObjFailure>>,
    mut visit: impl FnMut(&mut ParseChildren<'region, 'op, 'session>, usize, usize) -> ParseResult<()>,
) -> ParseResult<Option<usize>> {
    let mut token_start = None;
    let mut chunk_start = 0usize;

    while chunk_start < line.len() {
        let chunk_end = str_chunk_end(line, chunk_start);
        let mut scanned_len = 0usize;
        let amount = usize_amount(&exec.services()?, chunk_end - chunk_start, resource)?;
        let errors = exec.error_view();
        exec.child(scan_work, [amount], |work| {
            work.run([amount], || -> ParseResult<()> {
                for (relative, character) in line[chunk_start..chunk_end].char_indices() {
                    let offset =
                        u16::try_from(relative).map_err(|_| errors.allocation_failure(resource))?;
                    let stored_offset = scratch
                        .offsets
                        .get_mut(scanned_len)
                        .ok_or_else(|| errors.allocation_failure(resource))?;
                    let stored_character = scratch
                        .characters
                        .get_mut(scanned_len)
                        .ok_or_else(|| errors.allocation_failure(resource))?;
                    *stored_offset = offset;
                    *stored_character = character;
                    scanned_len = scanned_len
                        .checked_add(1)
                        .ok_or_else(|| errors.allocation_failure(resource))?;
                }
                Ok(())
            })
        })?;

        // Token visitors can meter their own work, so semantic callbacks run only after the
        // bounded scan capability has closed. The fixed buffer avoids an unmetered allocation or
        // a second UTF-8 decode pass while preserving callback order.
        for (&relative, &character) in scratch.offsets[..scanned_len]
            .iter()
            .zip(&scratch.characters[..scanned_len])
        {
            let index = chunk_start
                .checked_add(usize::from(relative))
                .ok_or_else(|| exec.error_view().allocation_failure(resource))?;
            if character == '#' {
                if let Some(start) = token_start.take() {
                    visit(exec, start, index)?;
                }
                return Ok(Some(index));
            }
            if let Err(cause) = inspect(index, character) {
                return Err(match cause {
                    ParsePieceError::Parse(cause) => cause,
                    ParsePieceError::Format(cause) => exec.services()?.format_failure(cause),
                });
            }
            if character.is_whitespace() {
                if let Some(start) = token_start.take() {
                    visit(exec, start, index)?;
                }
            } else if token_start.is_none() {
                token_start = Some(index);
            }
        }
        chunk_start = chunk_end;
    }

    if let Some(start) = token_start {
        visit(exec, start, line.len())?;
    }
    Ok(None)
}

pub(crate) fn read_obj(
    options: &ObjImportOptions,
    context: &mut ParseContext<'_, '_>,
) -> ParseResult<()> {
    let parsed = context.with_execution(|root| {
        let source = root.services()?.diagnostic_source();
        with_obj_text(
            &mut root.children()?,
            options,
            source,
            "obj-primary-text",
            |exec, text, recoveries| obj::parse(exec, text, recoveries),
        )
    })?;
    let materials = context
        .with_execution(|exec| mtl::load_libraries(exec, options, parsed.mtllibs.as_slice()))?;

    mesh_builder::scene_from_parsed(context, parsed, &materials)
}

fn invalid_utf8_failure(exec: &mut ParseServices<'_, '_, '_>, raw_offset: u64) -> ParseFailure {
    exec.format_failure(ObjFailure::invalid_utf8_at(raw_offset))
}

pub(crate) fn max_text_value_bytes(exec: &ParseServices<'_, '_, '_>) -> usize {
    exec.limits().max_string_bytes
}

pub(crate) fn enforce_text_value_limit(
    exec: &ParseServices<'_, '_, '_>,
    bytes: usize,
    _resource: &'static str,
) -> ParseResult<()> {
    exec.enforce_resource_limit(
        "string-bytes",
        usize_amount(exec, bytes, "string-bytes")?,
        usize_amount(exec, exec.limits().max_string_bytes, "string-bytes")?,
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
