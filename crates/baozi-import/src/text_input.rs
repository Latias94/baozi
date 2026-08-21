use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextDecodingMode, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticLineColumn, DiagnosticSeverity,
    DiagnosticSource, TextDiagnosticLocation,
};
use crate::ledger::ImportProgressPhase;
use crate::metered_storage::{MeteredDecodedText, MeteredVec, metered_vec_replacement_plans};
use crate::scope::{
    ImportStorageClass, ImportWorkClass, METERED_COPY_POLL_INTERVAL, ParseFailure, ParseResult,
    ParseScope,
};
use std::fmt;
use std::marker::PhantomData;
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
    raw_end: Option<u64>,
    raw_limit: Option<TextRawLimit>,
    line_number_base: u64,
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
            raw_end: None,
            raw_limit: None,
            line_number_base: 1,
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

    /// Selects an exclusive absolute raw byte end for this conformance operation.
    ///
    /// The end must not precede [`Self::with_raw_base`] and both offsets must fall within the
    /// source capability. Omitting the end keeps the existing source-suffix behavior.
    pub const fn with_raw_end(mut self, raw_end: u64) -> Self {
        self.raw_end = Some(raw_end);
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

    const fn with_line_number_base(mut self, line_number_base: u64) -> Self {
        self.line_number_base = line_number_base;
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
    line_number_base: u64,
    primary_source: bool,
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
            .field("line_number_base", &self.line_number_base)
            .field("primary_source", &self.primary_source)
            .field("line_starts", &self.line_starts.as_slice())
            .field("replacements", &self.replacements.as_slice())
            .finish()
    }
}

impl<'scope> ConformedTextInput<'scope> {
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    pub const fn bom(&self) -> Option<&TextInputBom> {
        self.bom.as_ref()
    }

    pub fn lines(&self) -> impl Iterator<Item = TextInputLine<'_, 'scope>> + '_ {
        TextInputLines {
            text: self.text.as_str(),
            raw_end: self.raw_end,
            line_number_base: self.line_number_base,
            primary_source: self.primary_source,
            line_starts: self.line_starts.as_slice(),
            replacements: self.replacements.as_slice(),
            line_index: 0,
            scope: PhantomData,
        }
    }
}

/// Source-bound incremental cursor over physical text records.
///
/// Each successful step conforms exactly one LF, CRLF, or CR terminated record, or the final
/// unterminated record. Stopping the cursor leaves every following byte untouched, which allows
/// mixed text/binary formats to transfer ownership at an exact record boundary.
/// Reject-invalid ASCII profiles stop an overlong record after the first byte beyond the shared
/// line limit. Aggregate decoded limits remain the format's responsibility when it intentionally
/// consumes only a prefix of the selected source range.
pub struct TextInputRecordCursor<'scope, 'session, 'source> {
    scope: &'scope ParseScope<'session>,
    profile: &'scope TextProfile,
    facts: TextProfileFacts,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    raw_offset: u64,
    raw_end: u64,
    line_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextInputRecordBoundary {
    Complete(u64),
    Violation(TextConformanceViolation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextInputRecordScanLimit {
    LineBytes { probe_len: usize, limit: usize },
}

impl TextInputRecordScanLimit {
    const fn probe_len(self) -> usize {
        match self {
            Self::LineBytes { probe_len, .. } => probe_len,
        }
    }
}

#[derive(Debug)]
enum TextInputRecordScanExit {
    Scope(ParseFailure),
    Boundary(TextInputRecordBoundary),
    PendingCr { newline: usize },
    LineLimit { processed: usize, limit: usize },
}

impl From<ParseFailure> for TextInputRecordScanExit {
    fn from(failure: ParseFailure) -> Self {
        Self::Scope(failure)
    }
}

impl<'scope, 'session, 'source> TextInputRecordCursor<'scope, 'session, 'source> {
    pub(crate) fn new(
        scope: &'scope ParseScope<'session>,
        profile: Option<&'scope TextProfile>,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
    ) -> ParseResult<TextConformanceResult<Self>> {
        let range = text_input_range(scope, &source, options)?;
        let facts = match text_profile_facts(profile, options.decoding_mode) {
            Ok(facts) => facts,
            Err(violation) => return Ok(Err(violation)),
        };
        let profile = profile.expect("validated text profile must be present");
        let raw_len = usize_to_u64(scope, range.len(), options.resource)?;
        let raw_limit = options.raw_limit.unwrap_or(TextRawLimit {
            resource: options.resource,
            limit: scope.limits().max_primary_asset_bytes,
        });
        scope.enforce_resource_limit(raw_limit.resource, raw_len, raw_limit.limit)?;
        let raw_offset = usize_to_u64(scope, range.start, options.resource)?;
        let raw_end = usize_to_u64(scope, range.end, options.resource)?;
        Ok(Ok(Self {
            scope,
            profile,
            facts,
            source,
            options,
            raw_offset,
            raw_end,
            line_number: 1,
        }))
    }

    /// Returns the absolute raw offset where the next record begins.
    pub const fn raw_offset(&self) -> u64 {
        self.raw_offset
    }

    /// Conforms and processes the next physical record without allowing decoded borrows to escape.
    pub fn with_next_record<T>(
        &mut self,
        action: impl for<'input> FnOnce(
            &'input ConformedTextInput<'scope>,
            &mut TextInputRecoveryTracker<'input, 'session>,
        ) -> ParseResult<T>,
    ) -> ParseResult<TextConformanceResult<Option<T>>> {
        let Some(boundary) = self.next_record_boundary()? else {
            return Ok(Ok(None));
        };
        let raw_end = match boundary {
            TextInputRecordBoundary::Complete(raw_end) => raw_end,
            TextInputRecordBoundary::Violation(violation) => return Ok(Err(violation)),
        };
        let next_line_number = self.line_number.checked_add(1).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded("text-input-line-number", u64::MAX)
        })?;
        let options = self
            .options
            .with_raw_base(self.raw_offset)
            .with_raw_end(raw_end)
            .with_line_number_base(self.line_number);
        let source = DiagnosticSource::new(
            self.source.authority().clone(),
            self.source.path(),
            self.source.bytes(),
        );
        let conformed =
            with_conformed_text_input(self.scope, Some(self.profile), source, options, action)?;
        match conformed {
            Ok(value) => {
                self.raw_offset = raw_end;
                self.line_number = next_line_number;
                Ok(Ok(Some(value)))
            }
            Err(violation) => Ok(Err(violation)),
        }
    }

    fn next_record_boundary(&self) -> ParseResult<Option<TextInputRecordBoundary>> {
        if self.raw_offset == self.raw_end {
            return Ok(None);
        }
        let start = usize::try_from(self.raw_offset).map_err(|_| {
            self.scope
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let end = usize::try_from(self.raw_end).map_err(|_| {
            self.scope
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let bytes = self.source.bytes().get(start..end).ok_or_else(|| {
            self.scope
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let scan_limit = self.record_scan_limit();
        let bom_probe_len = self.bom_probe_len(bytes.len());
        let scan_end = scan_limit.map_or(bytes.len(), |limit| {
            limit
                .probe_len()
                .saturating_add(bom_probe_len)
                .min(bytes.len())
        });
        let mut semantic_scan_end = None;
        let mut line_limit_reached = false;
        let mut chunk_start = 0usize;
        while chunk_start < scan_end {
            let chunk_end = if semantic_scan_end.is_none() && bom_probe_len != 0 {
                bom_probe_len
            } else {
                chunk_start
                    .checked_add(TEXT_INPUT_WORK_CHUNK_BYTES)
                    .map_or(scan_end, |candidate| candidate.min(scan_end))
            };
            let admitted_len = chunk_end.checked_sub(chunk_start).ok_or_else(|| {
                self.scope
                    .descriptor_contract_failure("text-input-record-scan-range-invalid")
            })?;
            let admitted_work = usize_to_u64(self.scope, admitted_len, self.options.resource)?;
            let scan = self.scope.with_work(
                ImportWorkClass::DecodeWork,
                admitted_work,
                |work| -> Result<(), TextInputRecordScanExit> {
                    if semantic_scan_end.is_none() {
                        let has_bom = bom_probe_len == 3
                            && bytes
                                .get(..3)
                                .is_some_and(|prefix| prefix == [0xef, 0xbb, 0xbf]);
                        if has_bom && self.facts.bom_policy == TextBomPolicy::Forbidden {
                            work.complete([3])?;
                            let raw_end = self.absolute_record_end(3)?;
                            let raw_range = DiagnosticByteRange::new(self.raw_offset, raw_end)
                                .map_err(|_| {
                                    self.scope.descriptor_contract_failure(
                                        "text-input-record-bom-range-invalid",
                                    )
                                })?;
                            return Err(TextInputRecordScanExit::Boundary(
                                TextInputRecordBoundary::Violation(TextConformanceViolation::at(
                                    TextConformanceViolationKind::ForbiddenBom,
                                    raw_range,
                                )),
                            ));
                        }
                        let input_start = usize::from(has_bom) * 3;
                        semantic_scan_end = Some(scan_limit.map_or(bytes.len(), |limit| {
                            limit
                                .probe_len()
                                .saturating_add(input_start)
                                .min(bytes.len())
                        }));
                        line_limit_reached = scan_limit.is_some_and(|limit| {
                            bytes.len().saturating_sub(input_start) >= limit.probe_len()
                        });
                    }

                    let scanned_end = chunk_end.min(semantic_scan_end.unwrap_or(bytes.len()));
                    let chunk = bytes.get(chunk_start..scanned_end).ok_or_else(|| {
                        self.scope
                            .descriptor_contract_failure("text-input-record-scan-range-invalid")
                    })?;
                    if let Some(relative) =
                        chunk.iter().position(|byte| matches!(*byte, b'\r' | b'\n'))
                    {
                        let newline = chunk_start.checked_add(relative).ok_or_else(|| {
                            self.scope
                                .resource_limit_exceeded(self.options.resource, u64::MAX)
                        })?;
                        let byte = bytes.get(newline).copied().ok_or_else(|| {
                            self.scope.descriptor_contract_failure(
                                "text-input-record-newline-out-of-bounds",
                            )
                        })?;
                        if byte == b'\r' && newline.checked_add(1) == Some(scanned_end) {
                            let observed =
                                scanned_end.checked_sub(chunk_start).ok_or_else(|| {
                                    self.scope.descriptor_contract_failure(
                                        "text-input-record-scan-range-invalid",
                                    )
                                })?;
                            work.complete([usize_to_u64(
                                self.scope,
                                observed,
                                self.options.resource,
                            )?])?;
                            return Err(TextInputRecordScanExit::PendingCr { newline });
                        }

                        let has_lookahead =
                            byte == b'\r' && bytes.get(newline.saturating_add(1)).is_some();
                        let newline_len = usize::from(
                            has_lookahead && bytes.get(newline.saturating_add(1)) == Some(&b'\n'),
                        ) + 1;
                        let record_len = newline.checked_add(newline_len).ok_or_else(|| {
                            self.scope
                                .resource_limit_exceeded(self.options.resource, u64::MAX)
                        })?;
                        let observed = relative
                            .checked_add(1 + usize::from(has_lookahead))
                            .ok_or_else(|| {
                                self.scope
                                    .resource_limit_exceeded(self.options.resource, u64::MAX)
                            })?;
                        work.complete([usize_to_u64(
                            self.scope,
                            observed,
                            self.options.resource,
                        )?])?;
                        let raw_end = self.absolute_record_end(record_len)?;
                        return Err(TextInputRecordScanExit::Boundary(
                            TextInputRecordBoundary::Complete(raw_end),
                        ));
                    }

                    let observed = scanned_end.checked_sub(chunk_start).ok_or_else(|| {
                        self.scope
                            .descriptor_contract_failure("text-input-record-scan-range-invalid")
                    })?;
                    work.complete([usize_to_u64(self.scope, observed, self.options.resource)?])?;
                    if (scanned_end < chunk_end || scanned_end == semantic_scan_end.unwrap_or(0))
                        && line_limit_reached
                    {
                        if let Some(TextInputRecordScanLimit::LineBytes { probe_len, limit }) =
                            scan_limit
                        {
                            return Err(TextInputRecordScanExit::LineLimit {
                                processed: probe_len,
                                limit,
                            });
                        }
                        return Err(self
                            .scope
                            .descriptor_contract_failure(
                                "text-input-record-line-limit-state-invalid",
                            )
                            .into());
                    }
                    Ok(())
                },
            );
            match scan {
                Ok(()) => {}
                Err(TextInputRecordScanExit::Scope(failure)) => return Err(failure),
                Err(TextInputRecordScanExit::Boundary(boundary)) => return Ok(Some(boundary)),
                Err(TextInputRecordScanExit::PendingCr { newline }) => {
                    return self.record_boundary_after_cr(bytes, newline).map(Some);
                }
                Err(TextInputRecordScanExit::LineLimit { processed, limit }) => {
                    return Err(self.line_limit_failure(processed, limit));
                }
            }
            chunk_start = chunk_end;
        }
        Ok(Some(TextInputRecordBoundary::Complete(self.raw_end)))
    }

    fn record_scan_limit(&self) -> Option<TextInputRecordScanLimit> {
        if self.options.decoding_mode != TextDecodingMode::RejectInvalid
            || self.facts.lexical_mode != TextLexicalMode::AsciiOnly
        {
            return None;
        }
        let probe_len = self.scope.limits().max_line_bytes.checked_add(1)?;
        Some(TextInputRecordScanLimit::LineBytes {
            probe_len,
            limit: self.scope.limits().max_line_bytes,
        })
    }

    fn bom_probe_len(&self, bytes_len: usize) -> usize {
        usize::from(self.raw_offset == 0) * 3.min(bytes_len)
    }

    fn record_boundary_after_cr(
        &self,
        bytes: &[u8],
        newline: usize,
    ) -> ParseResult<TextInputRecordBoundary> {
        let Some(next) = newline.checked_add(1) else {
            return Err(self
                .scope
                .resource_limit_exceeded(self.options.resource, u64::MAX));
        };
        let newline_len = if let Some(byte) = bytes.get(next) {
            self.scope
                .with_work(ImportWorkClass::DecodeWork, 1, |work| {
                    let newline_len = if *byte == b'\n' { 2 } else { 1 };
                    work.complete_all()?;
                    Ok(newline_len)
                })?
        } else {
            1
        };
        let record_len = newline.checked_add(newline_len).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded(self.options.resource, u64::MAX)
        })?;
        self.absolute_record_end(record_len)
            .map(TextInputRecordBoundary::Complete)
    }

    fn line_limit_failure(&self, processed: usize, limit: usize) -> ParseFailure {
        let processed = match usize_to_u64(self.scope, processed, self.options.resource) {
            Ok(processed) => processed,
            Err(failure) => return failure,
        };
        let limit = match usize_to_u64(self.scope, limit, self.options.line_resource) {
            Ok(limit) => limit,
            Err(failure) => return failure,
        };
        match self
            .scope
            .enforce_resource_limit(self.options.line_resource, processed, limit)
        {
            Ok(()) => self
                .scope
                .descriptor_contract_failure("text-input-line-limit-probe-accepted"),
            Err(failure) => failure,
        }
    }

    fn absolute_record_end(&self, record_len: usize) -> ParseResult<u64> {
        let record_len = usize_to_u64(self.scope, record_len, self.options.resource)?;
        let end = self.raw_offset.checked_add(record_len).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded(self.options.resource, u64::MAX)
        })?;
        if end > self.raw_end {
            return Err(self
                .scope
                .descriptor_contract_failure("text-input-record-end-out-of-bounds"));
        }
        Ok(end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextInputLineStart {
    raw_offset: u64,
    decoded_offset: usize,
    replacement_index: usize,
}

#[derive(Debug)]
pub struct TextInputLine<'input, 'scope> {
    number: u64,
    text: &'input str,
    raw_content_range: DiagnosticByteRange,
    raw_record_range: DiagnosticByteRange,
    decoded_content_range: Range<usize>,
    decoded_record_range: Range<usize>,
    newline: Option<TextNewline>,
    replacements: &'input [TextInputReplacement],
    primary_source: bool,
    scope: PhantomData<&'scope ()>,
}

impl<'scope> TextInputLine<'_, 'scope> {
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

    pub fn start_diagnostic_location(&self) -> TextDiagnosticLocation<'scope> {
        let start = self.raw_content_range.start();
        let byte_range = DiagnosticByteRange::new(start, start)
            .expect("conformed line start must form a valid diagnostic range");
        let line_column = DiagnosticLineColumn::new(self.number, 1)
            .expect("conformed line number must form a valid diagnostic location");
        TextDiagnosticLocation::new(byte_range, line_column, self.primary_source)
    }

    pub fn diagnostic_location(
        &self,
        byte_range: DiagnosticByteRange,
    ) -> Option<TextDiagnosticLocation<'scope>> {
        if byte_range.start() < self.raw_content_range.start()
            || byte_range.end() > self.raw_content_range.end()
        {
            return None;
        }
        let column = byte_range
            .start()
            .checked_sub(self.raw_content_range.start())?
            .checked_add(1)?;
        let line_column = DiagnosticLineColumn::new(self.number, column).ok()?;
        Some(TextDiagnosticLocation::new(
            byte_range,
            line_column,
            self.primary_source,
        ))
    }
}

struct TextInputLines<'input, 'scope> {
    text: &'input str,
    raw_end: u64,
    line_number_base: u64,
    primary_source: bool,
    line_starts: &'input [TextInputLineStart],
    replacements: &'input [TextInputReplacement],
    line_index: usize,
    scope: PhantomData<&'scope ()>,
}

impl<'input, 'scope> Iterator for TextInputLines<'input, 'scope> {
    type Item = TextInputLine<'input, 'scope>;

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
        let replacement_end = next.map_or(self.replacements.len(), |line| line.replacement_index);
        let replacements = self
            .replacements
            .get(start.replacement_index..replacement_end)?;
        if replacements.iter().any(|replacement| {
            replacement.decoded_start < start.decoded_offset
                || replacement.decoded_end > decoded_end
        }) {
            return None;
        }
        let raw_record_end = next.map_or(self.raw_end, |line| line.raw_offset);
        let raw_content_end = raw_record_end.checked_sub(raw_newline_len)?;
        if start.raw_offset > raw_content_end
            || raw_record_end > self.raw_end
            || (next.is_none() && raw_record_end != self.raw_end)
        {
            return None;
        }
        let raw_content_range = DiagnosticByteRange::new(start.raw_offset, raw_content_end).ok()?;
        let raw_record_range = DiagnosticByteRange::new(start.raw_offset, raw_record_end).ok()?;
        let number = self
            .line_number_base
            .checked_add(u64::try_from(self.line_index).ok()?)?;
        self.line_index = self.line_index.checked_add(1)?;
        Some(TextInputLine {
            number,
            text,
            raw_content_range,
            raw_record_range,
            decoded_content_range: start.decoded_offset..decoded_end,
            decoded_record_range: start.decoded_offset..terminated_decoded_end,
            newline,
            replacements,
            primary_source: self.primary_source,
            scope: PhantomData,
        })
    }
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
    line_number_base: u64,
    line_starts: &'input [TextInputLineStart],
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
        let line_column = DiagnosticLineColumn::new(self.line_number_base, 1).map_err(|_| {
            self.scope
                .descriptor_contract_failure("text-input-bom-location-invalid")
        })?;
        self.scope.emit_diagnostic_with_location(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            self.bound_source(),
            Some(bom.raw_range()),
            Some(line_column),
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
        let line_column = self.line_column(replacement.raw_range()).map_err(|()| {
            self.scope
                .descriptor_contract_failure("text-input-recovery-location-invalid")
        })?;
        self.scope.record_format_recovery_with_location(
            code,
            self.bound_source(),
            replacement.raw_range(),
            Some(line_column),
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

    fn line_column(&self, range: DiagnosticByteRange) -> Result<DiagnosticLineColumn, ()> {
        let line_index = self
            .line_starts
            .partition_point(|line| line.raw_offset <= range.start())
            .checked_sub(1)
            .ok_or(())?;
        let line = self.line_starts.get(line_index).ok_or(())?;
        let line_number = self
            .line_number_base
            .checked_add(u64::try_from(line_index).map_err(|_| ())?)
            .ok_or(())?;
        let column = range
            .start()
            .checked_sub(line.raw_offset)
            .and_then(|offset| offset.checked_add(1))
            .ok_or(())?;
        DiagnosticLineColumn::new(line_number, column).map_err(|_| ())
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
    let primary_source = scope.diagnostic_source_is_primary(&source);
    let input = match conform_text_input(scope, profile, bytes, options, primary_source)? {
        Ok(input) => input,
        Err(violation) => return Ok(Err(violation)),
    };
    let mut recoveries = TextInputRecoveryTracker {
        scope,
        source: DiagnosticSource::new(source.authority().clone(), source.path(), source.bytes()),
        line_number_base: input.line_number_base,
        line_starts: input.line_starts.as_slice(),
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
    let range = text_input_range(scope, source, options)?;
    source
        .bytes()
        .get(range)
        .ok_or_else(|| scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds"))
}

fn text_input_range(
    scope: &ParseScope<'_>,
    source: &DiagnosticSource<'_>,
    options: TextInputOptions,
) -> ParseResult<Range<usize>> {
    let start = usize::try_from(options.raw_base)
        .map_err(|_| scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds"))?;
    let end = match options.raw_end {
        Some(raw_end) => usize::try_from(raw_end).map_err(|_| {
            scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?,
        None => source.bytes().len(),
    };
    if start > end || end > source.bytes().len() {
        return Err(scope.descriptor_contract_failure("text-input-source-offset-out-of-bounds"));
    }
    Ok(start..end)
}

fn conform_text_input<'scope>(
    scope: &'scope ParseScope<'_>,
    profile: Option<&TextProfile>,
    bytes: &[u8],
    options: TextInputOptions,
    primary_source: bool,
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
    build_text_input(scope, &facts, bytes, options, plan, primary_source).map(Ok)
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
    primary_source: bool,
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
                line_starts.try_push_reserved(TextInputLineStart {
                    raw_offset: raw_start,
                    decoded_offset: 0,
                    replacement_index: 0,
                })?;
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
                                    raw_offset: raw_offset(scope, options, unit_end)?,
                                    decoded_offset: text.len(),
                                    replacement_index: replacements.len(),
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
        line_number_base: options.line_number_base,
        primary_source,
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
    fn line_start_index_retains_three_constant_size_offsets_per_line() {
        assert_eq!(
            std::mem::size_of::<TextInputLineStart>(),
            std::mem::size_of::<u64>() + 2 * std::mem::size_of::<usize>()
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
