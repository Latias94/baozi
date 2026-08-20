use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextDecodingMode, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity, DiagnosticSource,
};
use crate::ledger::ImportProgressPhase;
use crate::metered_storage::{MeteredDecodedText, MeteredVec, metered_vec_replacement_plans};
use crate::scope::{
    ImportStorageClass, ImportWorkClass, METERED_COPY_POLL_INTERVAL, ParseFailure, ParseResult,
    ParseScope,
};
use std::fmt;
use std::ops::Range;

const TEXT_INPUT_WORK_CHUNK_BYTES: usize = METERED_COPY_POLL_INTERVAL;

pub type TextConformanceResult<T> = Result<T, TextConformanceViolation>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextInputOptions {
    decoding_mode: TextDecodingMode,
    resource: &'static str,
    decoded_resource: &'static str,
    line_resource: &'static str,
    line_index_resource: &'static str,
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
            decoded_resource: resource,
            line_resource: resource,
            line_index_resource: resource,
            raw_base: 0,
            raw_limit: None,
        }
    }

    /// Selects the source suffix beginning at an absolute raw byte offset.
    ///
    /// BOM interpretation remains anchored at source byte zero. A BOM byte sequence at a nonzero
    /// offset is decoded as ordinary text.
    pub const fn with_raw_base(mut self, raw_base: u64) -> Self {
        self.raw_base = raw_base;
        self
    }

    pub const fn with_raw_limit(mut self, resource: &'static str, limit: u64) -> Self {
        self.raw_limit = Some(TextRawLimit { resource, limit });
        self
    }

    pub const fn with_decoded_resource(mut self, resource: &'static str) -> Self {
        self.decoded_resource = resource;
        self
    }

    pub const fn with_line_resource(mut self, resource: &'static str) -> Self {
        self.line_resource = resource;
        self
    }

    pub const fn with_line_index_resource(mut self, resource: &'static str) -> Self {
        self.line_index_resource = resource;
        self
    }

    pub const fn decoding_mode(self) -> TextDecodingMode {
        self.decoding_mode
    }

    pub const fn resource(self) -> &'static str {
        self.resource
    }
}

pub struct ConformedTextInput<'scope> {
    text: MeteredDecodedText<'scope>,
    bom: Option<TextInputBom>,
    raw_start: u64,
    raw_end: u64,
    line_starts: MeteredVec<'scope, TextInputLineStart>,
    replacements: MeteredVec<'scope, TextInputReplacement>,
}

impl fmt::Debug for ConformedTextInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConformedTextInput")
            .field("text", &self.text.as_str())
            .field("bom", &self.bom)
            .field("raw_start", &self.raw_start)
            .field("raw_end", &self.raw_end)
            .field("line_starts", &self.line_starts.as_slice())
            .field("replacements", &self.replacements.as_slice())
            .finish()
    }
}

impl ConformedTextInput<'_> {
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    pub const fn bom(&self) -> Option<&TextInputBom> {
        self.bom.as_ref()
    }

    pub fn lines(&self) -> impl Iterator<Item = TextInputLine<'_>> + '_ {
        TextInputLines {
            text: self.text.as_str(),
            raw_start: self.raw_start,
            raw_end: self.raw_end,
            line_starts: self.line_starts.as_slice(),
            replacements: self.replacements.as_slice(),
            line_index: 0,
            replacement_index: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextInputLineStart {
    decoded_offset: usize,
}

#[derive(Debug)]
pub struct TextInputLine<'input> {
    number: u64,
    text: &'input str,
    raw_content_range: DiagnosticByteRange,
    raw_record_range: DiagnosticByteRange,
    decoded_content_range: Range<usize>,
    decoded_record_range: Range<usize>,
    newline: Option<TextNewline>,
    replacements: &'input [TextInputReplacement],
}

impl TextInputLine<'_> {
    pub const fn number(&self) -> u64 {
        self.number
    }

    pub const fn text(&self) -> &str {
        self.text
    }

    pub const fn raw_content_range(&self) -> DiagnosticByteRange {
        self.raw_content_range
    }

    pub const fn raw_record_range(&self) -> DiagnosticByteRange {
        self.raw_record_range
    }

    pub fn decoded_content_range(&self) -> Range<usize> {
        self.decoded_content_range.clone()
    }

    pub fn decoded_record_range(&self) -> Range<usize> {
        self.decoded_record_range.clone()
    }

    pub const fn newline(&self) -> Option<TextNewline> {
        self.newline
    }

    pub const fn replacements(&self) -> &[TextInputReplacement] {
        self.replacements
    }
}

struct TextInputLines<'input> {
    text: &'input str,
    raw_start: u64,
    raw_end: u64,
    line_starts: &'input [TextInputLineStart],
    replacements: &'input [TextInputReplacement],
    line_index: usize,
    replacement_index: usize,
}

impl<'input> Iterator for TextInputLines<'input> {
    type Item = TextInputLine<'input>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = *self.line_starts.get(self.line_index)?;
        let next = self.line_starts.get(self.line_index + 1).copied();
        let terminated_decoded_end = next.map_or(self.text.len(), |line| line.decoded_offset);

        let record = self
            .text
            .get(start.decoded_offset..terminated_decoded_end)?;
        let (text, newline, raw_newline_len) = if let Some(text) = record.strip_suffix("\r\n") {
            (text, Some(TextNewline::CrLf), 2_u64)
        } else if let Some(text) = record.strip_suffix('\n') {
            (text, Some(TextNewline::Lf), 1)
        } else if let Some(text) = record.strip_suffix('\r') {
            (text, Some(TextNewline::Cr), 1)
        } else {
            (record, None, 0)
        };
        let decoded_end = start.decoded_offset.checked_add(text.len())?;
        let mut replacement_end = self.replacement_index;
        while let Some(replacement) = self.replacements.get(replacement_end) {
            if replacement.decoded_start < start.decoded_offset {
                return None;
            }
            if replacement.decoded_start >= decoded_end {
                break;
            }
            if replacement.decoded_end > decoded_end {
                return None;
            }
            replacement_end = replacement_end.checked_add(1)?;
        }
        let replacements = self
            .replacements
            .get(self.replacement_index..replacement_end)?;
        if next.is_none() && replacement_end != self.replacements.len() {
            return None;
        }
        let raw_content_end = mapped_raw_end(
            self.raw_start,
            start.decoded_offset,
            decoded_end,
            replacements,
        )?;
        let raw_record_end = raw_content_end.checked_add(raw_newline_len)?;
        if raw_record_end > self.raw_end || (next.is_none() && raw_record_end != self.raw_end) {
            return None;
        }
        let raw_content_range = DiagnosticByteRange::new(self.raw_start, raw_content_end).ok()?;
        let raw_record_range = DiagnosticByteRange::new(self.raw_start, raw_record_end).ok()?;
        let number = u64::try_from(self.line_index).ok()?.checked_add(1)?;
        self.line_index = self.line_index.checked_add(1)?;
        self.replacement_index = replacement_end;
        self.raw_start = raw_record_end;
        Some(TextInputLine {
            number,
            text,
            raw_content_range,
            raw_record_range,
            decoded_content_range: start.decoded_offset..decoded_end,
            decoded_record_range: start.decoded_offset..terminated_decoded_end,
            newline,
            replacements,
        })
    }
}

fn mapped_raw_end(
    raw_start: u64,
    decoded_start: usize,
    decoded_end: usize,
    replacements: &[TextInputReplacement],
) -> Option<u64> {
    let mut raw_offset = raw_start;
    let mut decoded_offset = decoded_start;
    for replacement in replacements {
        let valid_len = replacement.decoded_start.checked_sub(decoded_offset)?;
        raw_offset = raw_offset.checked_add(u64::try_from(valid_len).ok()?)?;
        if raw_offset != replacement.raw_range.start() {
            return None;
        }
        raw_offset = replacement.raw_range.end();
        decoded_offset = replacement.decoded_end;
    }
    let trailing_len = decoded_end.checked_sub(decoded_offset)?;
    raw_offset.checked_add(u64::try_from(trailing_len).ok()?)
}

#[derive(Debug)]
pub struct TextInputBom {
    raw_range: DiagnosticByteRange,
    diagnostic_required: bool,
}

impl TextInputBom {
    const fn new(raw_range: DiagnosticByteRange, diagnostic_required: bool) -> Self {
        Self {
            raw_range,
            diagnostic_required,
        }
    }

    pub const fn raw_range(&self) -> DiagnosticByteRange {
        self.raw_range
    }

    pub const fn requires_diagnostic(&self) -> bool {
        self.diagnostic_required
    }
}

#[derive(Debug)]
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

    pub const fn raw_range(&self) -> DiagnosticByteRange {
        self.raw_range
    }

    pub const fn decoded_start(&self) -> usize {
        self.decoded_start
    }

    pub const fn decoded_end(&self) -> usize {
        self.decoded_end
    }
}

/// Tracks the format-owned disposition of text-input recovery obligations.
///
/// A successful text-input callback must emit any required BOM diagnostic before classifying
/// invalid UTF-8 replacements in source order. A replacement may be discarded only when its
/// decoded value did not influence format semantics; otherwise it must be authorized through
/// [`Self::record_format_recovery`]. Diagnostics and recovery evidence always use the source bound
/// to the conformance operation.
pub struct TextInputRecoveryTracker<'input, 'session> {
    scope: &'input ParseScope<'session>,
    source: DiagnosticSource<'input>,
    pending_bom_diagnostic: Option<&'input TextInputBom>,
    replacements: &'input [TextInputReplacement],
    next: usize,
}

impl fmt::Debug for TextInputRecoveryTracker<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextInputRecoveryTracker")
            .field(
                "bom_diagnostic_pending",
                &self.pending_bom_diagnostic.is_some(),
            )
            .field("replacement_count", &self.replacements.len())
            .field("classified_count", &self.next)
            .finish()
    }
}

impl TextInputRecoveryTracker<'_, '_> {
    pub fn record_bom_diagnostic(
        &mut self,
        bom: &TextInputBom,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        let Some(expected) = self.pending_bom_diagnostic else {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-bom-diagnostic-overclassified"));
        };
        if !std::ptr::eq(expected, bom) {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-bom-diagnostic-mismatch"));
        }
        self.scope.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            self.bound_source(),
            Some(bom.raw_range()),
            None,
            |_| {},
            message,
        )?;
        self.pending_bom_diagnostic = None;
        Ok(())
    }

    pub fn discard_replacement(&mut self, replacement: &TextInputReplacement) -> ParseResult<()> {
        self.classify_next(replacement)
    }

    pub fn discard_replacements(
        &mut self,
        replacements: &[TextInputReplacement],
    ) -> ParseResult<()> {
        if replacements.is_empty() {
            return Ok(());
        }
        self.require_bom_diagnostic()?;
        let Some(end) = self.next.checked_add(replacements.len()) else {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        let Some(expected) = self.replacements.get(self.next..end) else {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        if !std::ptr::eq(expected.as_ptr(), replacements.as_ptr()) {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-replacement-out-of-order"));
        }
        self.next = end;
        Ok(())
    }

    pub fn record_format_recovery(
        &mut self,
        replacement: &TextInputReplacement,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.expect_next(replacement)?;
        self.scope.record_format_recovery(
            code,
            self.bound_source(),
            replacement.raw_range(),
            message,
        )?;
        self.next += 1;
        Ok(())
    }

    fn classify_next(&mut self, replacement: &TextInputReplacement) -> ParseResult<()> {
        self.expect_next(replacement)?;
        self.next += 1;
        Ok(())
    }

    fn expect_next(&self, replacement: &TextInputReplacement) -> ParseResult<()> {
        self.require_bom_diagnostic()?;
        let Some(expected) = self.replacements.get(self.next) else {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        if !std::ptr::eq(expected, replacement) {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-replacement-out-of-order"));
        }
        Ok(())
    }

    fn require_bom_diagnostic(&self) -> ParseResult<()> {
        if self.pending_bom_diagnostic.is_none() {
            return Ok(());
        }
        Err(self
            .scope
            .descriptor_contract_failure("text-input-bom-diagnostic-unclassified"))
    }

    fn bound_source(&self) -> DiagnosticSource<'_> {
        DiagnosticSource::new(
            self.source.authority().clone(),
            self.source.path(),
            self.source.bytes(),
        )
    }

    fn finish(self) -> ParseResult<()> {
        self.require_bom_diagnostic()?;
        if self.next == self.replacements.len() {
            return Ok(());
        }
        Err(self
            .scope
            .descriptor_contract_failure("text-input-replacement-unclassified"))
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

pub(crate) fn with_conformed_text_input<'scope, 'session, 'source, T>(
    scope: &'scope ParseScope<'session>,
    profile: Option<&TextProfile>,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    action: impl for<'input> FnOnce(
        &'input ConformedTextInput<'scope>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<TextConformanceResult<T>> {
    let bytes = text_input_bytes(scope, &source, options)?;
    let input = match conform_text_input(scope, profile, bytes, options)? {
        Ok(input) => input,
        Err(violation) => return Ok(Err(violation)),
    };
    let mut recoveries = TextInputRecoveryTracker {
        scope,
        source: DiagnosticSource::new(source.authority().clone(), source.path(), source.bytes()),
        pending_bom_diagnostic: input.bom().filter(|bom| bom.requires_diagnostic()),
        replacements: input.replacements.as_slice(),
        next: 0,
    };
    let value = action(&input, &mut recoveries)?;
    recoveries.finish()?;
    Ok(Ok(value))
}

fn text_input_bytes<'source>(
    scope: &ParseScope<'_>,
    source: &'source DiagnosticSource<'_>,
    options: TextInputOptions,
) -> ParseResult<&'source [u8]> {
    let start = usize::try_from(options.raw_base)
        .map_err(|_| scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds"))?;
    source
        .bytes()
        .get(start..)
        .ok_or_else(|| scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds"))
}

fn conform_text_input<'scope>(
    scope: &'scope ParseScope<'_>,
    profile: Option<&TextProfile>,
    bytes: &[u8],
    options: TextInputOptions,
) -> ParseResult<TextConformanceResult<ConformedTextInput<'scope>>> {
    let facts = match text_profile_facts(profile, options.decoding_mode) {
        Ok(facts) => facts,
        Err(violation) => return Ok(Err(violation)),
    };

    let raw_len = usize_to_u64(scope, bytes.len(), options.resource)?;
    let raw_limit = options.raw_limit.unwrap_or(TextRawLimit {
        resource: options.resource,
        limit: scope.limits().max_primary_asset_bytes,
    });
    scope.enforce_resource_limit(raw_limit.resource, raw_len, raw_limit.limit)?;

    let plan = match plan_text_input(scope, &facts, bytes, options) {
        Ok(plan) => plan,
        Err(TextInputScanError::Scope(failure)) => return Err(failure),
        Err(TextInputScanError::Violation(violation)) => return Ok(Err(violation)),
    };
    build_text_input(scope, &facts, bytes, options, plan).map(Ok)
}

pub(crate) fn check_text_input_conformance(
    profile: &TextProfile,
    bytes: &[u8],
    decoding_mode: TextDecodingMode,
) -> TextConformanceResult<()> {
    let facts = text_profile_facts(Some(profile), decoding_mode)?;
    check_text_input_units(&facts, bytes, decoding_mode)
}

fn text_profile_facts(
    profile: Option<&TextProfile>,
    decoding_mode: TextDecodingMode,
) -> TextConformanceResult<TextProfileFacts> {
    let Some(profile) = profile else {
        return Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::MissingTextProfile,
        ));
    };
    if !profile
        .source_encodings()
        .any(|encoding| encoding == TextSourceEncoding::Utf8)
    {
        return Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::UnsupportedSourceEncoding,
        ));
    }
    if !profile.allows_decoding_mode(decoding_mode) {
        return Err(TextConformanceViolation::without_range(
            TextConformanceViolationKind::DecodingModeNotDeclared,
        ));
    }
    Ok(TextProfileFacts::new(profile))
}

fn check_text_input_units(
    facts: &TextProfileFacts,
    bytes: &[u8],
    decoding_mode: TextDecodingMode,
) -> TextConformanceResult<()> {
    let mut offset = 0usize;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        if facts.bom_policy == TextBomPolicy::Forbidden {
            return Err(TextConformanceViolation::at(
                TextConformanceViolationKind::ForbiddenBom,
                detection_raw_range(0, 3),
            ));
        }
        offset = 3;
    }

    while offset < bytes.len() {
        match classify_text_unit(facts, decoding_mode, bytes, offset) {
            TextUnit::Valid { raw_len, .. } | TextUnit::Replacement { raw_len } => {
                offset += raw_len;
            }
            TextUnit::Violation { kind, raw_len } => {
                return Err(TextConformanceViolation::at(
                    kind,
                    detection_raw_range(offset, offset + raw_len),
                ));
            }
        }
    }
    Ok(())
}

fn detection_raw_range(start: usize, end: usize) -> DiagnosticByteRange {
    let start = u64::try_from(start).expect("bounded detector byte offset must fit in u64");
    let end = u64::try_from(end).expect("bounded detector byte offset must fit in u64");
    DiagnosticByteRange::new(start, end).expect("ordered detector byte range must be valid")
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
    line_starts: InitialMeteredVecPlan,
    replacements: InitialMeteredVecPlan,
    line_index_limit_dirty: bool,
    bom: Option<TextInputBom>,
}

#[derive(Debug, Default)]
struct InitialMeteredVecPlan {
    len: usize,
    capacity: usize,
    envelope_bytes: u64,
}

impl InitialMeteredVecPlan {
    const fn will_grow(&self) -> bool {
        self.len == self.capacity
    }

    fn add<T>(&mut self, scope: &ParseScope<'_>, resource: &'static str) -> ParseResult<bool> {
        let len = checked_add_usize(scope, self.len, 1, resource)?;
        if len <= self.capacity {
            self.len = len;
            return Ok(false);
        }

        let (_, replacement_plan) = metered_vec_replacement_plans::<T>(0, 0, len)
            .map_err(|_| scope.resource_limit_exceeded(resource, u64::MAX))?;
        self.len = len;
        self.capacity = replacement_plan.envelope_capacity();
        self.envelope_bytes = replacement_plan
            .envelope_delta_bytes()
            .map_err(|_| scope.resource_limit_exceeded(resource, u64::MAX))?;
        Ok(true)
    }
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
        line_starts: InitialMeteredVecPlan::default(),
        replacements: InitialMeteredVecPlan::default(),
        line_index_limit_dirty: false,
        bom: None,
    };
    if options.raw_base == 0 && bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
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
                    cursor.process_to(3);
                    cursor.flush(scope, work, options.resource)?;
                    Err(TextInputScanError::Violation(violation))
                });
            }
            TextBomPolicy::Accepted => {
                plan.input_start = 3;
                plan.bom = Some(TextInputBom::new(raw_range, false));
            }
            TextBomPolicy::AcceptedWithDiagnostic => {
                plan.input_start = 3;
                plan.bom = Some(TextInputBom::new(raw_range, true));
            }
        }
    }

    if plan.input_start < bytes.len() {
        plan.line_index_limit_dirty = plan
            .line_starts
            .add::<TextInputLineStart>(scope, options.line_index_resource)?;
    }

    let work_amount = usize_to_u64(scope, bytes.len(), options.resource)?;
    scope.with_work(ImportWorkClass::DecodeWork, work_amount, |work| {
        let mut cursor = WorkCursor::new();
        let mut offset = plan.input_start;
        let mut line_len = 0usize;
        while offset < bytes.len() {
            let unit = classify_text_unit(facts, options.decoding_mode, bytes, offset);
            match unit {
                TextUnit::Valid { raw_len, newline } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.poll_before_crossing(scope, work, unit_end, options.resource)?;
                    cursor.process_to(unit_end);
                    plan.decoded_len = cursor.checked_add_after_processing(
                        scope,
                        work,
                        plan.decoded_len,
                        raw_len,
                        options.resource,
                    )?;
                    enforce_processed_usize_limit(
                        scope,
                        &mut cursor,
                        work,
                        plan.decoded_len,
                        options.resource,
                        options.decoded_resource,
                        scope.limits().max_decoded_utf8_bytes,
                    )?;
                    if newline.is_some() {
                        enforce_processed_usize_limit(
                            scope,
                            &mut cursor,
                            work,
                            line_len,
                            options.resource,
                            options.line_resource,
                            scope.limits().max_line_bytes as u64,
                        )?;
                        if unit_end < bytes.len() {
                            if plan.line_starts.will_grow() {
                                cursor.flush(scope, work, options.resource)?;
                            }
                            plan.line_index_limit_dirty |= plan
                                .line_starts
                                .add::<TextInputLineStart>(scope, options.line_index_resource)?;
                        }
                        enforce_line_index_limit_after_processing(
                            scope,
                            &mut cursor,
                            work,
                            &mut plan,
                            options,
                        )?;
                        line_len = 0;
                    } else {
                        line_len = cursor.checked_add_after_processing(
                            scope,
                            work,
                            line_len,
                            raw_len,
                            options.resource,
                        )?;
                    }
                    offset = unit_end;
                }
                TextUnit::Replacement { raw_len } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.poll_before_crossing(scope, work, unit_end, options.resource)?;
                    cursor.process_to(unit_end);
                    plan.decoded_len = cursor.checked_add_after_processing(
                        scope,
                        work,
                        plan.decoded_len,
                        '\u{fffd}'.len_utf8(),
                        options.resource,
                    )?;
                    enforce_processed_usize_limit(
                        scope,
                        &mut cursor,
                        work,
                        plan.decoded_len,
                        options.resource,
                        options.decoded_resource,
                        scope.limits().max_decoded_utf8_bytes,
                    )?;
                    if plan.replacements.will_grow() {
                        cursor.flush(scope, work, options.resource)?;
                    }
                    plan.line_index_limit_dirty |= plan
                        .replacements
                        .add::<TextInputReplacement>(scope, options.line_index_resource)?;
                    line_len = cursor.checked_add_after_processing(
                        scope,
                        work,
                        line_len,
                        '\u{fffd}'.len_utf8(),
                        options.resource,
                    )?;
                    enforce_line_index_limit_after_processing(
                        scope,
                        &mut cursor,
                        work,
                        &mut plan,
                        options,
                    )?;
                    offset = unit_end;
                }
                TextUnit::Violation { kind, raw_len } => {
                    let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                    cursor.poll_before_crossing(scope, work, unit_end, options.resource)?;
                    cursor.process_to(unit_end);
                    cursor.flush(scope, work, options.resource)?;
                    let violation = TextConformanceViolation::at(
                        kind,
                        raw_range(scope, options, offset, unit_end)?,
                    );
                    return Err(TextInputScanError::Violation(violation));
                }
            }
            cursor.flush_and_poll_if_chunk_complete(scope, work, options.resource)?;
        }
        cursor.process_to(bytes.len());
        cursor.flush(scope, work, options.resource)?;
        enforce_processed_usize_limit(
            scope,
            &mut cursor,
            work,
            line_len,
            options.resource,
            options.line_resource,
            scope.limits().max_line_bytes as u64,
        )?;
        enforce_line_index_limit_after_processing(scope, &mut cursor, work, &mut plan, options)?;
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
    let mut line_starts =
        scope.metered_vec(ImportStorageClass::LineIndex, options.line_index_resource);
    line_starts.try_reserve(plan.line_starts.len)?;
    let mut replacements =
        scope.metered_vec(ImportStorageClass::LineIndex, options.line_index_resource);
    replacements.try_reserve(plan.replacements.len)?;
    let mut text = scope.metered_decoded_text(plan.decoded_len, options.decoded_resource)?;
    let raw_start = raw_offset(scope, options, plan.input_start)?;
    let raw_end = raw_offset(scope, options, bytes.len())?;

    let work_amount = usize_to_u64(scope, bytes.len(), options.resource)?;
    scope.with_work(
        ImportWorkClass::DecodeWork,
        work_amount,
        |work| -> ParseResult<()> {
            let mut cursor = WorkCursor::new();
            if plan.line_starts.len != 0 {
                line_starts.try_push_reserved(TextInputLineStart { decoded_offset: 0 })?;
            }
            let mut offset = plan.input_start;
            let mut valid_run_start = offset;
            while offset < bytes.len() {
                match classify_text_unit(facts, options.decoding_mode, bytes, offset) {
                    TextUnit::Valid { raw_len, newline } => {
                        let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                        cursor.poll_before_crossing(scope, work, unit_end, options.resource)?;
                        if unit_end.saturating_sub(valid_run_start) > TEXT_INPUT_WORK_CHUNK_BYTES {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, offset)?;
                            valid_run_start = offset;
                        }
                        if newline.is_some() {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, unit_end)?;
                            if unit_end < bytes.len() {
                                line_starts.try_push_reserved(TextInputLineStart {
                                    decoded_offset: text.len(),
                                })?;
                            }
                            valid_run_start = unit_end;
                        } else if unit_end.saturating_sub(valid_run_start)
                            >= TEXT_INPUT_WORK_CHUNK_BYTES
                        {
                            append_valid_run(scope, &mut text, bytes, valid_run_start, unit_end)?;
                            valid_run_start = unit_end;
                        }
                        offset = unit_end;
                        cursor.process_to(offset);
                    }
                    TextUnit::Replacement { raw_len } => {
                        let unit_end = checked_add_usize(scope, offset, raw_len, options.resource)?;
                        cursor.poll_before_crossing(scope, work, unit_end, options.resource)?;
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
                        cursor.process_to(offset);
                    }
                    TextUnit::Violation { .. } => {
                        return Err(scope
                            .descriptor_contract_failure("text-input-plan-revalidation-failed"));
                    }
                }
                cursor.flush_and_poll_if_chunk_complete(scope, work, options.resource)?;
            }
            append_valid_run(scope, &mut text, bytes, valid_run_start, bytes.len())?;
            cursor.process_to(bytes.len());
            cursor.flush(scope, work, options.resource)?;
            Ok(())
        },
    )?;

    let text = text.finish_exact()?;
    if line_starts.len() != plan.line_starts.len || replacements.len() != plan.replacements.len {
        return Err(scope.descriptor_contract_failure("text-input-plan-count-mismatch"));
    }
    Ok(ConformedTextInput {
        text,
        bom: plan.bom,
        raw_start,
        raw_end,
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

#[derive(Debug, Clone, Copy)]
enum TextUnit {
    Valid {
        raw_len: usize,
        newline: Option<TextNewline>,
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
    if let Some((newline, raw_len)) = newline_at(bytes, offset) {
        return if facts.allows_newline(newline) {
            TextUnit::Valid {
                raw_len,
                newline: Some(newline),
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
            newline: None,
        };
    }

    match classify_utf8_unit(bytes, offset) {
        Utf8Unit::Valid(raw_len) => {
            if facts.lexical_mode == TextLexicalMode::AsciiOnly {
                return TextUnit::Violation {
                    kind: TextConformanceViolationKind::NonAscii,
                    raw_len: 1,
                };
            }
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
                newline: None,
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
    processed_to: usize,
    completed_to: usize,
    next_poll_at: usize,
}

impl WorkCursor {
    const fn new() -> Self {
        Self {
            processed_to: 0,
            completed_to: 0,
            next_poll_at: TEXT_INPUT_WORK_CHUNK_BYTES,
        }
    }

    fn process_to(&mut self, offset: usize) {
        debug_assert!(offset >= self.processed_to);
        self.processed_to = offset;
    }

    fn poll_before_crossing(
        &mut self,
        scope: &ParseScope<'_>,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
        offset: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        if offset <= self.next_poll_at {
            return Ok(());
        }
        self.flush(scope, work, resource)?;
        work.poll()?;
        self.next_poll_at = self
            .processed_to
            .saturating_add(TEXT_INPUT_WORK_CHUNK_BYTES);
        Ok(())
    }

    fn flush(
        &mut self,
        scope: &ParseScope<'_>,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
        resource: &'static str,
    ) -> ParseResult<()> {
        if self.processed_to <= self.completed_to {
            return Ok(());
        }
        let amount = self
            .processed_to
            .checked_sub(self.completed_to)
            .ok_or_else(|| scope.resource_limit_exceeded(resource, u64::MAX))?;
        work.complete([usize_to_u64(scope, amount, resource)?])?;
        self.completed_to = self.processed_to;
        Ok(())
    }

    fn checked_add_after_processing(
        &mut self,
        scope: &ParseScope<'_>,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
        left: usize,
        right: usize,
        resource: &'static str,
    ) -> ParseResult<usize> {
        if let Some(value) = left.checked_add(right) {
            return Ok(value);
        }
        self.flush(scope, work, resource)?;
        Err(scope.resource_limit_exceeded(resource, u64::MAX))
    }

    fn flush_and_poll_if_chunk_complete(
        &mut self,
        scope: &ParseScope<'_>,
        work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
        resource: &'static str,
    ) -> ParseResult<()> {
        if self.processed_to >= self.next_poll_at {
            self.flush(scope, work, resource)?;
            work.poll()?;
            self.next_poll_at = self
                .processed_to
                .saturating_add(TEXT_INPUT_WORK_CHUNK_BYTES);
        }
        Ok(())
    }
}

fn enforce_processed_usize_limit(
    scope: &ParseScope<'_>,
    cursor: &mut WorkCursor,
    work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
    value: usize,
    work_resource: &'static str,
    resource: &'static str,
    limit: u64,
) -> ParseResult<()> {
    if u64::try_from(value).is_ok_and(|attempted| attempted <= limit) {
        return Ok(());
    }
    cursor.flush(scope, work, work_resource)?;
    Err(scope.resource_limit_exceeded(resource, limit))
}

fn enforce_line_index_limit_after_processing(
    scope: &ParseScope<'_>,
    cursor: &mut WorkCursor,
    work: &mut crate::scope::ImportWorkScope<'_, '_, 1>,
    plan: &mut TextInputPlan,
    options: TextInputOptions,
) -> ParseResult<()> {
    if !plan.line_index_limit_dirty {
        return Ok(());
    }
    let Some(bytes) = plan
        .line_starts
        .envelope_bytes
        .checked_add(plan.replacements.envelope_bytes)
    else {
        cursor.flush(scope, work, options.resource)?;
        return Err(scope.resource_limit_exceeded(options.line_index_resource, u64::MAX));
    };
    let limit = scope.limits().max_line_index_bytes;
    if bytes > limit {
        cursor.flush(scope, work, options.resource)?;
        return Err(scope.resource_limit_exceeded(options.line_index_resource, limit));
    }
    plan.line_index_limit_dirty = false;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
        FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportDimension,
        ImportOperationControl, ImportRequest, OptionCompileLimits, ReadConfidence, ReadHint,
        RegistryBuildLimits, RegistryBuilder, SceneDraft, ScopeAbortCause, TextHardCaps,
    };
    use baozi_core::FormatId;
    use baozi_io::AssetPath;

    struct AllocationFailureImporter;

    impl FormatImporter for AllocationFailureImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.text-input-allocation")
                    .expect("static format id must be valid"),
                "Text Input Allocation Failure",
            )
            .extensions(["txt"])
            .encoding(FormatEncoding::Text)
            .text_profile(
                TextProfile::builder(
                    TextHardCaps::new(1024, 1024, 1024, 1024, 1024)
                        .expect("test text hard caps must be valid"),
                )
                .source_encodings([TextSourceEncoding::Utf8])
                .bom_policy(TextBomPolicy::Forbidden)
                .newlines([TextNewline::Lf])
                .lexical_mode(TextLexicalMode::Utf8Document)
                .decoding_modes([TextDecodingMode::RejectInvalid])
                .control_policy(TextControlPolicy::RejectAll)
                .build()
                .expect("test text profile must be valid"),
            )
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("test text descriptor must be valid")
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            crate::scope::inject_next_metered_allocation_failure();
            scope
                .with_conformed_text_input(
                    scope.diagnostic_source(),
                    TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                        .with_line_index_resource("test-text-line-index"),
                    |_input, _recoveries| Ok(()),
                )?
                .expect("the allocation fixture must otherwise conform");
            panic!("injected allocation failure must stop text construction")
        }
    }

    #[test]
    fn line_start_index_retains_only_one_machine_word_per_line() {
        assert_eq!(
            std::mem::size_of::<TextInputLineStart>(),
            std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn line_index_allocation_failure_releases_all_text_storage() {
        let mut registry = RegistryBuilder::new();
        registry
            .register(AllocationFailureImporter)
            .expect("test importer registration must succeed");
        let request = registry
            .freeze(RegistryBuildLimits::default())
            .expect("test registry must freeze")
            .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("test request must compile");
        let error = crate::import_bytes(
            &request,
            AssetPath::new("allocation.txt").expect("static source path must be valid"),
            b"a",
            ImportOperationControl::default(),
        )
        .expect_err("injected line-index allocation must fail");
        let failure = error
            .into_operation_failure()
            .unwrap_or_else(|source| panic!("test session must initialize: {source}"));

        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Allocation {
                        resource: "test-text-line-index",
                    }
                )
        ));
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert!(failure.diagnostics().is_empty());
        assert!(failure.loss_evidence().is_empty());
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DecodedTextBytes)
                .map_or(0, |evidence| evidence.peak_live_bytes()),
            0
        );
    }
}
