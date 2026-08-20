use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextDecodingMode, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::diagnostic::{DiagnosticByteRange, DiagnosticCode};
use crate::metered_storage::{MeteredDecodedText, MeteredVec};
use crate::scope::{
    ImportStorageClass, ImportWorkClass, METERED_COPY_POLL_INTERVAL, ParseFailure, ParseResult,
    ParseScope,
};
use baozi_operation::CapacityPlan;
use std::fmt;

const TEXT_INPUT_WORK_CHUNK_BYTES: usize = METERED_COPY_POLL_INTERVAL;
const INVALID_UTF8_REPLACEMENT_CODE: &str = "import.text.invalid_utf8_replaced";

pub type TextConformanceResult<'scope> =
    Result<ConformedTextInput<'scope>, TextConformanceViolation>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextInputOptions {
    decoding_mode: TextDecodingMode,
    resource: &'static str,
    raw_base: u64,
    raw_limit: Option<TextRawLimit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextRawLimit {
    resource: &'static str,
    limit: u64,
}

impl TextInputOptions {
    pub const fn new(decoding_mode: TextDecodingMode, resource: &'static str) -> Self {
        Self {
            decoding_mode,
            resource,
            raw_base: 0,
            raw_limit: None,
        }
    }

    pub const fn with_raw_base(mut self, raw_base: u64) -> Self {
        self.raw_base = raw_base;
        self
    }

    pub const fn with_raw_limit(mut self, resource: &'static str, limit: u64) -> Self {
        self.raw_limit = Some(TextRawLimit { resource, limit });
        self
    }

    pub const fn decoding_mode(self) -> TextDecodingMode {
        self.decoding_mode
    }

    pub const fn resource(self) -> &'static str {
        self.resource
    }

    pub const fn raw_base(self) -> u64 {
        self.raw_base
    }
}

pub struct ConformedTextInput<'scope> {
    text: MeteredDecodedText<'scope>,
    bom: Option<TextInputBom>,
    line_starts: MeteredVec<'scope, u64>,
    replacements: MeteredVec<'scope, TextInputReplacement>,
}

impl fmt::Debug for ConformedTextInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConformedTextInput")
            .field("text", &self.text.as_str())
            .field("bom", &self.bom)
            .field("line_starts", &self.line_starts.as_slice())
            .field("replacements", &self.replacements.as_slice())
            .finish()
    }
}

impl ConformedTextInput<'_> {
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    pub const fn decoded_text(&self) -> &MeteredDecodedText<'_> {
        &self.text
    }

    pub const fn bom(&self) -> Option<TextInputBom> {
        self.bom
    }

    pub fn line_start_offsets(&self) -> &[u64] {
        self.line_starts.as_slice()
    }

    pub fn replacements(&self) -> &[TextInputReplacement] {
        self.replacements.as_slice()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextInputBom {
    raw_range: DiagnosticByteRange,
}

impl TextInputBom {
    const fn new(raw_range: DiagnosticByteRange) -> Self {
        Self { raw_range }
    }

    pub const fn raw_range(self) -> DiagnosticByteRange {
        self.raw_range
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextInputReplacement {
    raw_range: DiagnosticByteRange,
    decoded_start: usize,
    decoded_end: usize,
}

impl TextInputReplacement {
    const fn new(raw_range: DiagnosticByteRange, decoded_start: usize, decoded_end: usize) -> Self {
        Self {
            raw_range,
            decoded_start,
            decoded_end,
        }
    }

    pub const fn raw_range(self) -> DiagnosticByteRange {
        self.raw_range
    }

    pub const fn decoded_start(self) -> usize {
        self.decoded_start
    }

    pub const fn decoded_end(self) -> usize {
        self.decoded_end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextConformanceViolationKind {
    MissingTextProfile,
    UnsupportedSourceEncoding,
    DecodingModeNotDeclared,
    ForbiddenBom,
    InvalidUtf8,
    UnsupportedNewline,
    ForbiddenControl,
    NonAscii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextConformanceViolation {
    kind: TextConformanceViolationKind,
    raw_range: Option<DiagnosticByteRange>,
}

impl TextConformanceViolation {
    const fn without_range(kind: TextConformanceViolationKind) -> Self {
        Self {
            kind,
            raw_range: None,
        }
    }

    const fn at(kind: TextConformanceViolationKind, raw_range: DiagnosticByteRange) -> Self {
        Self {
            kind,
            raw_range: Some(raw_range),
        }
    }

    pub const fn kind(self) -> TextConformanceViolationKind {
        self.kind
    }

    pub const fn raw_range(self) -> Option<DiagnosticByteRange> {
        self.raw_range
    }
}

pub(crate) fn conform_text_input<'scope>(
    scope: &'scope ParseScope<'_>,
    profile: Option<&TextProfile>,
    bytes: &[u8],
    options: TextInputOptions,
) -> ParseResult<TextConformanceResult<'scope>> {
    let Some(profile) = profile else {
        return Ok(Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::MissingTextProfile,
        )));
    };
    if !profile
        .source_encodings()
        .any(|encoding| encoding == TextSourceEncoding::Utf8)
    {
        return Ok(Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::UnsupportedSourceEncoding,
        )));
    }
    if !profile.allows_decoding_mode(options.decoding_mode) {
        return Ok(Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::DecodingModeNotDeclared,
        )));
    }

    let raw_len = usize_to_u64(scope, bytes.len(), options.resource)?;
    let raw_limit = options.raw_limit.unwrap_or(TextRawLimit {
        resource: options.resource,
        limit: scope.limits().max_primary_asset_bytes,
    });
    scope.enforce_resource_limit(raw_limit.resource, raw_len, raw_limit.limit)?;

    let facts = TextProfileFacts::new(profile);
    let plan = match plan_text_input(scope, &facts, bytes, options) {
        Ok(plan) => plan,
        Err(TextInputScanError::Scope(failure)) => return Err(failure),
        Err(TextInputScanError::Violation(violation)) => return Ok(Err(violation)),
    };
    if options.decoding_mode == TextDecodingMode::ReplaceInvalidUtf8 && plan.replacement_count != 0
    {
        record_invalid_utf8_replacements(scope, &facts, bytes, options)?;
    }
    build_text_input(scope, &facts, bytes, options, plan).map(Ok)
}

#[derive(Debug, Clone, Copy)]
struct TextProfileFacts {
    bom_policy: TextBomPolicy,
    lexical_mode: TextLexicalMode,
    control_policy: TextControlPolicy,
    allow_lf: bool,
    allow_crlf: bool,
    allow_cr: bool,
}

impl TextProfileFacts {
    fn new(profile: &TextProfile) -> Self {
        let mut facts = Self {
            bom_policy: profile.bom_policy(),
            lexical_mode: profile.lexical_mode(),
            control_policy: profile.control_policy(),
            allow_lf: false,
            allow_crlf: false,
            allow_cr: false,
        };
        for newline in profile.newlines() {
            match newline {
                TextNewline::Lf => facts.allow_lf = true,
                TextNewline::CrLf => facts.allow_crlf = true,
                TextNewline::Cr => facts.allow_cr = true,
            }
        }
        facts
    }

    const fn allows_newline(self, newline: TextNewline) -> bool {
        match newline {
            TextNewline::Lf => self.allow_lf,
            TextNewline::CrLf => self.allow_crlf,
            TextNewline::Cr => self.allow_cr,
        }
    }
}

#[derive(Debug)]
struct TextInputPlan {
    input_start: usize,
    decoded_len: usize,
    line_start_count: usize,
    replacement_count: usize,
    bom: Option<TextInputBom>,
}

#[derive(Debug)]
enum TextInputScanError {
    Scope(ParseFailure),
    Violation(TextConformanceViolation),
}

impl From<ParseFailure> for TextInputScanError {
    fn from(failure: ParseFailure) -> Self {
        Self::Scope(failure)
    }
}

fn plan_text_input(
    scope: &ParseScope<'_>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
) -> Result<TextInputPlan, TextInputScanError> {
    let mut plan = TextInputPlan {
        input_start: 0,
        decoded_len: 0,
        line_start_count: 1,
        replacement_count: 0,
        bom: None,
    };
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let raw_range = raw_range(scope, options, 0, 3)?;
        match facts.bom_policy {
            TextBomPolicy::Forbidden => {
                let violation = TextConformanceViolation::at(
                    TextConformanceViolationKind::ForbiddenBom,
                    raw_range,
                );
                let work_amount = usize_to_u64(scope, bytes.len(), options.resource)?;
                return scope.with_work(ImportWorkClass::DecodeWork, work_amount, |work| {
                    let mut cursor = WorkCursor::new();
                    cursor.complete_to(scope, work, 3, options.resource)?;
                    Err(TextInputScanError::Violation(violation))
                });
            }
            TextBomPolicy::Accepted | TextBomPolicy::AcceptedWithDiagnostic => {
                plan.input_start = 3;
                plan.bom = Some(TextInputBom::new(raw_range));
            }
        }
    }

    let work_amount = usize_to_u64(scope, bytes.len(), options.resource)?;
    scope.with_work(ImportWorkClass::DecodeWork, work_amount, |work| {
        let mut cursor = WorkCursor::new();
        let mut offset = plan.input_start;
        let mut line_len = 0usize;
        while offset < bytes.len() {
            work.poll()?;
            let unit = classify_text_unit(facts, options.decoding_mode, bytes, offset);
            match unit {
                TextUnit::Valid {
                    raw_len,
                    is_newline,
                } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.complete_to(scope, work, unit_end, options.resource)?;
                    plan.decoded_len =
                        checked_add_usize(scope, plan.decoded_len, raw_len, options.resource)?;
                    enforce_decoded_limit(scope, plan.decoded_len, options.resource)?;
                    if is_newline {
                        enforce_line_limit(scope, line_len, options.resource)?;
                        plan.line_start_count =
                            checked_add_usize(scope, plan.line_start_count, 1, options.resource)?;
                        enforce_line_index_limit(scope, &plan, options.resource)?;
                        line_len = 0;
                    } else {
                        line_len = checked_add_usize(scope, line_len, raw_len, options.resource)?;
                    }
                    offset = unit_end;
                }
                TextUnit::Replacement { raw_len } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.complete_to(scope, work, unit_end, options.resource)?;
                    plan.decoded_len = checked_add_usize(
                        scope,
                        plan.decoded_len,
                        '\u{fffd}'.len_utf8(),
                        options.resource,
                    )?;
                    enforce_decoded_limit(scope, plan.decoded_len, options.resource)?;
                    plan.replacement_count =
                        checked_add_usize(scope, plan.replacement_count, 1, options.resource)?;
                    line_len = checked_add_usize(
                        scope,
                        line_len,
                        '\u{fffd}'.len_utf8(),
                        options.resource,
                    )?;
                    enforce_line_index_limit(scope, &plan, options.resource)?;
                    offset = unit_end;
                }
                TextUnit::Violation { kind, raw_len } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.complete_to(scope, work, unit_end, options.resource)?;
                    let violation = TextConformanceViolation::at(
                        kind,
                        raw_range(scope, options, offset, unit_end)?,
                    );
                    return Err(TextInputScanError::Violation(violation));
                }
            }
            cursor.poll_if_chunk_complete(work)?;
        }
        cursor.complete_to(scope, work, bytes.len(), options.resource)?;
        enforce_line_limit(scope, line_len, options.resource)?;
        enforce_line_index_limit(scope, &plan, options.resource)?;
        Ok(plan)
    })
}

fn build_text_input<'scope>(
    scope: &'scope ParseScope<'_>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
    plan: TextInputPlan,
) -> ParseResult<ConformedTextInput<'scope>> {
    let mut line_starts = scope.metered_vec(ImportStorageClass::LineIndex, "text-line-index");
    line_starts.try_reserve(plan.line_start_count)?;
    let mut replacements =
        scope.metered_vec(ImportStorageClass::LineIndex, "text-replacement-index");
    replacements.try_reserve(plan.replacement_count)?;
    let mut text = scope.metered_decoded_text(plan.decoded_len, options.resource)?;

    let work_amount = usize_to_u64(scope, bytes.len(), options.resource)?;
    scope.with_work(
        ImportWorkClass::DecodeWork,
        work_amount,
        |work| -> ParseResult<()> {
            let mut cursor = WorkCursor::new();
            line_starts.try_push_reserved(raw_offset(scope, options, plan.input_start)?)?;
            let mut offset = plan.input_start;
            let mut valid_run_start = offset;
            while offset < bytes.len() {
                work.poll()?;
                match classify_text_unit(facts, options.decoding_mode, bytes, offset) {
                    TextUnit::Valid {
                        raw_len,
                        is_newline,
                    } => {
                        let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                        if unit_end.saturating_sub(valid_run_start) > TEXT_INPUT_WORK_CHUNK_BYTES {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, offset)?;
                            valid_run_start = offset;
                        }
                        if is_newline {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, unit_end)?;
                            line_starts.try_push_reserved(raw_offset(scope, options, unit_end)?)?;
                            valid_run_start = unit_end;
                        } else if unit_end.saturating_sub(valid_run_start)
                            >= TEXT_INPUT_WORK_CHUNK_BYTES
                        {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, unit_end)?;
                            valid_run_start = unit_end;
                        }
                        offset = unit_end;
                        cursor.complete_to(scope, work, offset, options.resource)?;
                    }
                    TextUnit::Replacement { raw_len } => {
                        let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                        append_valid_run(scope, &mut text, bytes, valid_run_start, offset)?;
                        let raw_range = raw_range_parse(scope, options, offset, unit_end)?;
                        let decoded_start = text.len();
                        text.try_push_str_reserved("\u{fffd}")?;
                        let decoded_end = text.len();
                        replacements.try_push_reserved(TextInputReplacement::new(
                            raw_range,
                            decoded_start,
                            decoded_end,
                        ))?;
                        offset = unit_end;
                        valid_run_start = unit_end;
                        cursor.complete_to(scope, work, offset, options.resource)?;
                    }
                    TextUnit::Violation { .. } => {
                        return Err(scope
                            .descriptor_contract_failure("text-input-plan-revalidation-failed"));
                    }
                }
                cursor.poll_if_chunk_complete(work)?;
            }
            append_valid_run(scope, &mut text, bytes, valid_run_start, bytes.len())?;
            cursor.complete_to(scope, work, bytes.len(), options.resource)?;
            Ok(())
        },
    )?;

    let text = text.finish_exact()?;
    if line_starts.len() != plan.line_start_count || replacements.len() != plan.replacement_count {
        return Err(scope.descriptor_contract_failure("text-input-plan-count-mismatch"));
    }
    Ok(ConformedTextInput {
        text,
        bom: plan.bom,
        line_starts,
        replacements,
    })
}

fn append_valid_run(
    scope: &ParseScope<'_>,
    text: &mut MeteredDecodedText<'_>,
    bytes: &[u8],
    start: usize,
    end: usize,
) -> ParseResult<()> {
    if start == end {
        return Ok(());
    }
    let chunk = bytes
        .get(start..end)
        .ok_or_else(|| scope.descriptor_contract_failure("text-input-copy-range-invalid"))?;
    let decoded = std::str::from_utf8(chunk)
        .map_err(|_| scope.descriptor_contract_failure("text-input-copy-utf8-invalid"))?;
    text.try_push_str_reserved(decoded)
}

fn record_invalid_utf8_replacements(
    scope: &ParseScope<'_>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
) -> ParseResult<()> {
    let mut offset = 0;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) && facts.bom_policy != TextBomPolicy::Forbidden {
        offset = 3;
    }
    while offset < bytes.len() {
        match classify_text_unit(facts, options.decoding_mode, bytes, offset) {
            TextUnit::Valid { raw_len, .. } => {
                offset = checked_add_usize(scope, offset, raw_len, options.resource)?;
            }
            TextUnit::Replacement { raw_len } => {
                let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                record_invalid_utf8_replacement(scope, options, offset, unit_end)?;
                offset = unit_end;
            }
            TextUnit::Violation { .. } => {
                return Err(
                    scope.descriptor_contract_failure("text-input-recovery-revalidation-failed")
                );
            }
        }
    }
    Ok(())
}

fn record_invalid_utf8_replacement(
    scope: &ParseScope<'_>,
    options: TextInputOptions,
    start: usize,
    end: usize,
) -> ParseResult<()> {
    let raw_range = raw_range_parse(scope, options, start, end)?;
    let code = DiagnosticCode::new(INVALID_UTF8_REPLACEMENT_CODE)
        .map_err(|_| scope.descriptor_contract_failure("text-input-invalid-replacement-code"))?;
    scope.record_format_recovery(
        code,
        scope.diagnostic_source(),
        raw_range,
        format_args!("replaced invalid UTF-8"),
    )
}

#[derive(Debug, Clone, Copy)]
enum TextUnit {
    Valid {
        raw_len: usize,
        is_newline: bool,
    },
    Replacement {
        raw_len: usize,
    },
    Violation {
        kind: TextConformanceViolationKind,
        raw_len: usize,
    },
}

fn classify_text_unit(
    facts: &TextProfileFacts,
    decoding_mode: TextDecodingMode,
    bytes: &[u8],
    offset: usize,
) -> TextUnit {
    let Some(&byte) = bytes.get(offset) else {
        return TextUnit::Violation {
            kind: TextConformanceViolationKind::InvalidUtf8,
            raw_len: 1,
        };
    };
    if facts.lexical_mode == TextLexicalMode::AsciiOnly && !byte.is_ascii() {
        return TextUnit::Violation {
            kind: TextConformanceViolationKind::NonAscii,
            raw_len: 1,
        };
    }
    if let Some((newline, raw_len)) = newline_at(bytes, offset) {
        return if facts.allows_newline(newline) {
            TextUnit::Valid {
                raw_len,
                is_newline: true,
            }
        } else {
            TextUnit::Violation {
                kind: TextConformanceViolationKind::UnsupportedNewline,
                raw_len,
            }
        };
    }
    if byte.is_ascii() {
        if byte.is_ascii_control() && !horizontal_tab_allowed(facts, byte) {
            return TextUnit::Violation {
                kind: TextConformanceViolationKind::ForbiddenControl,
                raw_len: 1,
            };
        }
        return TextUnit::Valid {
            raw_len: 1,
            is_newline: false,
        };
    }

    match classify_utf8_unit(bytes, offset) {
        Utf8Unit::Valid(raw_len) => {
            let Some(unit) = bytes.get(offset..offset.saturating_add(raw_len)) else {
                return TextUnit::Violation {
                    kind: TextConformanceViolationKind::InvalidUtf8,
                    raw_len,
                };
            };
            let Ok(text) = std::str::from_utf8(unit) else {
                return TextUnit::Violation {
                    kind: TextConformanceViolationKind::InvalidUtf8,
                    raw_len,
                };
            };
            let Some(character) = text.chars().next() else {
                return TextUnit::Violation {
                    kind: TextConformanceViolationKind::InvalidUtf8,
                    raw_len,
                };
            };
            if character.is_control() {
                return TextUnit::Violation {
                    kind: TextConformanceViolationKind::ForbiddenControl,
                    raw_len,
                };
            }
            TextUnit::Valid {
                raw_len,
                is_newline: false,
            }
        }
        Utf8Unit::Invalid(raw_len) => {
            if decoding_mode == TextDecodingMode::ReplaceInvalidUtf8 {
                TextUnit::Replacement { raw_len }
            } else {
                TextUnit::Violation {
                    kind: TextConformanceViolationKind::InvalidUtf8,
                    raw_len,
                }
            }
        }
    }
}

fn newline_at(bytes: &[u8], offset: usize) -> Option<(TextNewline, usize)> {
    match bytes.get(offset).copied()? {
        b'\r' if bytes.get(offset.saturating_add(1)) == Some(&b'\n') => {
            Some((TextNewline::CrLf, 2))
        }
        b'\r' => Some((TextNewline::Cr, 1)),
        b'\n' => Some((TextNewline::Lf, 1)),
        _ => None,
    }
}

const fn horizontal_tab_allowed(facts: &TextProfileFacts, byte: u8) -> bool {
    matches!(facts.control_policy, TextControlPolicy::AllowHorizontalTab) && byte == b'\t'
}

#[derive(Debug, Clone, Copy)]
enum Utf8Unit {
    Valid(usize),
    Invalid(usize),
}

fn classify_utf8_unit(bytes: &[u8], offset: usize) -> Utf8Unit {
    let Some(chunk) = bytes.get(offset..) else {
        return Utf8Unit::Invalid(1);
    };
    let Some(&lead) = chunk.first() else {
        return Utf8Unit::Invalid(1);
    };
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

const fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

#[derive(Debug, Default)]
struct WorkCursor {
    completed_to: usize,
}

impl WorkCursor {
    const fn new() -> Self {
        Self { completed_to: 0 }
    }

    fn complete_to(
        &mut self,
        scope: &ParseScope<'_>,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
        offset: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        if offset <= self.completed_to {
            return Ok(());
        }
        let amount = offset
            .checked_sub(self.completed_to)
            .ok_or_else(|| scope.resource_limit_exceeded(resource, u64::MAX))?;
        work.complete([usize_to_u64(scope, amount, resource)?])?;
        self.completed_to = offset;
        Ok(())
    }

    fn poll_if_chunk_complete(
        &self,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
    ) -> ParseResult<()> {
        if self
            .completed_to
            .is_multiple_of(TEXT_INPUT_WORK_CHUNK_BYTES)
        {
            work.poll()?;
        }
        Ok(())
    }
}

fn enforce_decoded_limit(
    scope: &ParseScope<'_>,
    decoded_len: usize,
    resource: &'static str,
) -> ParseResult<()> {
    scope.enforce_resource_limit(
        resource,
        usize_to_u64(scope, decoded_len, resource)?,
        scope.limits().max_decoded_utf8_bytes,
    )
}

fn enforce_line_limit(
    scope: &ParseScope<'_>,
    line_len: usize,
    resource: &'static str,
) -> ParseResult<()> {
    scope.enforce_resource_limit(
        resource,
        usize_to_u64(scope, line_len, resource)?,
        scope.limits().max_line_bytes as u64,
    )
}

fn enforce_line_index_limit(
    scope: &ParseScope<'_>,
    plan: &TextInputPlan,
    resource: &'static str,
) -> ParseResult<()> {
    let line_start_bytes =
        initial_metered_vec_bytes::<u64>(scope, plan.line_start_count, resource)?;
    let replacement_bytes =
        initial_metered_vec_bytes::<TextInputReplacement>(scope, plan.replacement_count, resource)?;
    let bytes = line_start_bytes
        .checked_add(replacement_bytes)
        .ok_or_else(|| scope.resource_limit_exceeded(resource, u64::MAX))?;
    scope.enforce_resource_limit(resource, bytes, scope.limits().max_line_index_bytes)
}

fn initial_metered_vec_bytes<T>(
    scope: &ParseScope<'_>,
    item_count: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    if item_count == 0 {
        return Ok(0);
    }
    let desired = item_count.max(4);
    let plan = CapacityPlan::for_type::<T>(0, 0, desired)
        .map_err(|_| scope.resource_limit_exceeded(resource, u64::MAX))?;
    plan.envelope_delta_bytes()
        .map_err(|_| scope.resource_limit_exceeded(resource, u64::MAX))
}

fn raw_range(
    scope: &ParseScope<'_>,
    options: TextInputOptions,
    start: usize,
    end: usize,
) -> Result<DiagnosticByteRange, TextInputScanError> {
    raw_range_parse(scope, options, start, end).map_err(TextInputScanError::Scope)
}

fn raw_range_parse(
    scope: &ParseScope<'_>,
    options: TextInputOptions,
    start: usize,
    end: usize,
) -> ParseResult<DiagnosticByteRange> {
    let start = raw_offset(scope, options, start)?;
    let end = raw_offset(scope, options, end)?;
    DiagnosticByteRange::new(start, end)
        .map_err(|_| scope.descriptor_contract_failure("text-input-raw-range-invalid"))
}

fn raw_offset(
    scope: &ParseScope<'_>,
    options: TextInputOptions,
    offset: usize,
) -> ParseResult<u64> {
    let offset = usize_to_u64(scope, offset, options.resource)?;
    options
        .raw_base
        .checked_add(offset)
        .ok_or_else(|| scope.resource_limit_exceeded(options.resource, u64::MAX))
}

fn checked_add_usize(
    scope: &ParseScope<'_>,
    left: usize,
    right: usize,
    resource: &'static str,
) -> ParseResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| scope.resource_limit_exceeded(resource, u64::MAX))
}

fn usize_to_u64(scope: &ParseScope<'_>, value: usize, resource: &'static str) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| scope.resource_limit_exceeded(resource, u64::MAX))
}
