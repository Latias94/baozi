use crate::dependency::DependencyOriginIssuer;
use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextDecodingMode, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticLineColumn, DiagnosticSeverity,
    DiagnosticSource, TextDiagnosticLocation,
};
use crate::ledger::ImportProgressPhase;
use crate::metered_storage::{MeteredDecodedText, MeteredVec, metered_vec_replacement_plan};
use crate::scope::{
    ImportStorageClass, ImportWorkClass, METERED_COPY_POLL_INTERVAL, ParseChildren, ParseErrorView,
    ParseFailure, ParseParser, ParseReadRow, ParseResult, ParseServices,
};
use baozi_operation::__retained_store_integration::activation::StoreScanReturn;
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
pub struct TextInputRecordCursor<'session, 'source> {
    issuer: &'session DependencyOriginIssuer,
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
    Complete,
    Boundary(TextInputRecordBoundary),
    PendingCr { newline: usize },
    LineLimit { processed: usize, limit: usize },
}

impl<'session, 'source> TextInputRecordCursor<'session, 'source> {
    pub(crate) fn new(
        scope: &mut ParseChildren<'_, '_, 'session>,
        profile: Option<&TextProfile>,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
    ) -> ParseResult<TextConformanceResult<Self>> {
        let range = text_input_range(scope.error_view(), &source, options)?;
        let facts = match text_profile_facts(profile, options.decoding_mode) {
            Ok(facts) => facts,
            Err(violation) => return Ok(Err(violation)),
        };
        let raw_len = usize_to_u64(scope.error_view(), range.len(), options.resource)?;
        let raw_limit = options.raw_limit.unwrap_or(TextRawLimit {
            resource: options.resource,
            limit: scope.error_view().limits().max_primary_asset_bytes,
        });
        scope
            .error_view()
            .enforce_resource_limit(raw_limit.resource, raw_len, raw_limit.limit)?;
        let raw_offset = usize_to_u64(scope.error_view(), range.start, options.resource)?;
        let raw_end = usize_to_u64(scope.error_view(), range.end, options.resource)?;
        Ok(Ok(Self {
            issuer: scope.error_view().session_issuer(),
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
    pub fn with_next_record<'region, 'op, T>(
        &mut self,
        scope: &mut ParseChildren<'region, 'op, 'session>,
        action: impl for<'input> FnOnce(
            &mut ParseChildren<'region, 'op, 'session>,
            &'input ConformedTextInput<'session>,
            &mut TextInputRecoveryTracker<'input, 'session>,
        ) -> ParseResult<T>,
    ) -> ParseResult<TextConformanceResult<Option<T>>> {
        scope.error_view().check_session(self.issuer)?;
        let Some(boundary) = self.next_record_boundary(scope)? else {
            return Ok(Ok(None));
        };
        let raw_end = match boundary {
            TextInputRecordBoundary::Complete(raw_end) => raw_end,
            TextInputRecordBoundary::Violation(violation) => return Ok(Err(violation)),
        };
        let next_line_number = self.line_number.checked_add(1).ok_or_else(|| {
            scope
                .error_view()
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
        let conformed = with_conformed_text_facts(scope, self.facts, source, options, action)?;
        match conformed {
            Ok(value) => {
                self.raw_offset = raw_end;
                self.line_number = next_line_number;
                Ok(Ok(Some(value)))
            }
            Err(violation) => Ok(Err(violation)),
        }
    }

    fn next_record_boundary(
        &self,
        scope: &mut ParseChildren<'_, '_, 'session>,
    ) -> ParseResult<Option<TextInputRecordBoundary>> {
        if self.raw_offset == self.raw_end {
            return Ok(None);
        }
        let start = usize::try_from(self.raw_offset).map_err(|_| {
            scope
                .error_view()
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let end = usize::try_from(self.raw_end).map_err(|_| {
            scope
                .error_view()
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let bytes = self.source.bytes().get(start..end).ok_or_else(|| {
            scope
                .error_view()
                .descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?;
        let scan_limit = self.record_scan_limit(scope.error_view());
        let bom_probe_len = self.bom_probe_len(bytes.len());
        let scan_end = scan_limit.map_or(bytes.len(), |limit| {
            limit
                .probe_len()
                .saturating_add(bom_probe_len)
                .min(bytes.len())
        });
        let mut semantic_scan_end = None;
        let mut line_limit_reached = false;
        let group = scope.bind_group([ImportWorkClass::DecodeWork])?;
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
                scope
                    .error_view()
                    .descriptor_contract_failure("text-input-record-scan-range-invalid")
            })?;
            let admitted_work =
                usize_to_u64(scope.error_view(), admitted_len, self.options.resource)?;
            let scan = scope.scan_child(
                &group,
                [admitted_work],
                |scope| -> ParseResult<StoreScanReturn<TextInputRecordScanExit>> {
                    let mut scan_start = chunk_start;
                    if semantic_scan_end.is_none() {
                        let has_bom = scope.run_returned(|| {
                            let has_bom = bom_probe_len == 3
                                && bytes
                                    .get(..3)
                                    .is_some_and(|prefix| prefix == [0xef, 0xbb, 0xbf]);
                            Ok((has_bom, [u64::from(has_bom) * 3]))
                        })?;
                        if has_bom {
                            if self.facts.bom_policy == TextBomPolicy::Forbidden {
                                let raw_end = self.absolute_record_end(scope.error_view(), 3)?;
                                let raw_range = DiagnosticByteRange::new(self.raw_offset, raw_end)
                                    .map_err(|_| {
                                        scope.error_view().descriptor_contract_failure(
                                            "text-input-record-bom-range-invalid",
                                        )
                                    })?;
                                return Ok(StoreScanReturn::Boundary(
                                    TextInputRecordScanExit::Boundary(
                                        TextInputRecordBoundary::Violation(
                                            TextConformanceViolation::at(
                                                TextConformanceViolationKind::ForbiddenBom,
                                                raw_range,
                                            ),
                                        ),
                                    ),
                                ));
                            }
                            scan_start = 3;
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
                    let chunk = bytes.get(scan_start..scanned_end).ok_or_else(|| {
                        scope
                            .error_view()
                            .descriptor_contract_failure("text-input-record-scan-range-invalid")
                    })?;
                    for (relative, byte) in chunk.iter().copied().enumerate() {
                        let is_newline = scope
                            .run([1], || Ok::<_, ParseFailure>(matches!(byte, b'\r' | b'\n')))?;
                        if !is_newline {
                            continue;
                        }
                        let newline = scan_start.checked_add(relative).ok_or_else(|| {
                            scope
                                .error_view()
                                .resource_limit_exceeded(self.options.resource, u64::MAX)
                        })?;
                        if byte == b'\r' && newline.checked_add(1) == Some(scanned_end) {
                            return Ok(StoreScanReturn::Boundary(
                                TextInputRecordScanExit::PendingCr { newline },
                            ));
                        }

                        let mut newline_len = 1;
                        if byte == b'\r'
                            && let Some(next) = bytes.get(newline.saturating_add(1))
                        {
                            let is_lf = scope.run([1], || Ok::<_, ParseFailure>(*next == b'\n'))?;
                            newline_len += usize::from(is_lf);
                        }
                        let record_len = newline.checked_add(newline_len).ok_or_else(|| {
                            scope
                                .error_view()
                                .resource_limit_exceeded(self.options.resource, u64::MAX)
                        })?;
                        let raw_end = self.absolute_record_end(scope.error_view(), record_len)?;
                        return Ok(StoreScanReturn::Boundary(
                            TextInputRecordScanExit::Boundary(TextInputRecordBoundary::Complete(
                                raw_end,
                            )),
                        ));
                    }

                    if (scanned_end < chunk_end || scanned_end == semantic_scan_end.unwrap_or(0))
                        && line_limit_reached
                    {
                        if let Some(TextInputRecordScanLimit::LineBytes { probe_len, limit }) =
                            scan_limit
                        {
                            return Ok(StoreScanReturn::Boundary(
                                TextInputRecordScanExit::LineLimit {
                                    processed: probe_len,
                                    limit,
                                },
                            ));
                        }
                        return Err(scope.error_view().descriptor_contract_failure(
                            "text-input-record-line-limit-state-invalid",
                        ));
                    }
                    Ok(StoreScanReturn::Exhausted(
                        TextInputRecordScanExit::Complete,
                    ))
                },
            );
            match scan? {
                TextInputRecordScanExit::Complete => {}
                TextInputRecordScanExit::Boundary(boundary) => return Ok(Some(boundary)),
                TextInputRecordScanExit::PendingCr { newline } => {
                    return self
                        .record_boundary_after_cr(scope, bytes, newline)
                        .map(Some);
                }
                TextInputRecordScanExit::LineLimit { processed, limit } => {
                    return Err(self.line_limit_failure(scope, processed, limit));
                }
            }
            chunk_start = chunk_end;
        }
        Ok(Some(TextInputRecordBoundary::Complete(self.raw_end)))
    }

    fn record_scan_limit(&self, errors: ParseErrorView<'_>) -> Option<TextInputRecordScanLimit> {
        if self.options.decoding_mode != TextDecodingMode::RejectInvalid
            || self.facts.lexical_mode != TextLexicalMode::AsciiOnly
        {
            return None;
        }
        let probe_len = errors.limits().max_line_bytes.checked_add(1)?;
        Some(TextInputRecordScanLimit::LineBytes {
            probe_len,
            limit: errors.limits().max_line_bytes,
        })
    }

    fn bom_probe_len(&self, bytes_len: usize) -> usize {
        usize::from(self.raw_offset == 0) * 3.min(bytes_len)
    }

    fn record_boundary_after_cr(
        &self,
        scope: &mut ParseChildren<'_, '_, 'session>,
        bytes: &[u8],
        newline: usize,
    ) -> ParseResult<TextInputRecordBoundary> {
        let Some(next) = newline.checked_add(1) else {
            return Err(scope
                .error_view()
                .resource_limit_exceeded(self.options.resource, u64::MAX));
        };
        let newline_len = if let Some(byte) = bytes.get(next) {
            let group = scope.bind_group([ImportWorkClass::DecodeWork])?;
            scope.child(&group, [1], |child| {
                child.run([1], || {
                    Ok::<_, ParseFailure>(if *byte == b'\n' { 2 } else { 1 })
                })
            })?
        } else {
            1
        };
        let record_len = newline.checked_add(newline_len).ok_or_else(|| {
            scope
                .error_view()
                .resource_limit_exceeded(self.options.resource, u64::MAX)
        })?;
        self.absolute_record_end(scope.error_view(), record_len)
            .map(TextInputRecordBoundary::Complete)
    }

    fn line_limit_failure(
        &self,
        scope: &mut ParseChildren<'_, '_, '_>,
        processed: usize,
        limit: usize,
    ) -> ParseFailure {
        let processed = match usize_to_u64(scope.error_view(), processed, self.options.resource) {
            Ok(processed) => processed,
            Err(failure) => return failure,
        };
        let limit = match usize_to_u64(scope.error_view(), limit, self.options.line_resource) {
            Ok(limit) => limit,
            Err(failure) => return failure,
        };
        match scope.error_view().enforce_resource_limit(
            self.options.line_resource,
            processed,
            limit,
        ) {
            Ok(()) => scope
                .error_view()
                .descriptor_contract_failure("text-input-line-limit-probe-accepted"),
            Err(failure) => failure,
        }
    }

    fn absolute_record_end(
        &self,
        errors: ParseErrorView<'_>,
        record_len: usize,
    ) -> ParseResult<u64> {
        let record_len = usize_to_u64(errors, record_len, self.options.resource)?;
        let end = self
            .raw_offset
            .checked_add(record_len)
            .ok_or_else(|| errors.resource_limit_exceeded(self.options.resource, u64::MAX))?;
        if end > self.raw_end {
            return Err(errors.descriptor_contract_failure("text-input-record-end-out-of-bounds"));
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
    issuer: &'session DependencyOriginIssuer,
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

impl<'session> TextInputRecoveryTracker<'_, 'session> {
    pub fn record_bom_diagnostic(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        bom: &TextInputBom,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        scope.error_view().check_session(self.issuer)?;
        let Some(expected) = self.pending_bom_diagnostic else {
            return Err(scope
                .error_view()
                .descriptor_contract_failure("text-input-bom-diagnostic-overclassified"));
        };
        if !std::ptr::eq(expected, bom) {
            return Err(scope
                .error_view()
                .descriptor_contract_failure("text-input-bom-diagnostic-mismatch"));
        }
        let line_column = DiagnosticLineColumn::new(self.line_number_base, 1).map_err(|_| {
            scope
                .error_view()
                .descriptor_contract_failure("text-input-bom-location-invalid")
        })?;
        scope.emit_diagnostic_with_location(
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

    pub fn discard_replacement(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        replacement: &TextInputReplacement,
    ) -> ParseResult<()> {
        self.classify_next(scope, replacement)
    }

    pub fn discard_replacements(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        replacements: &[TextInputReplacement],
    ) -> ParseResult<()> {
        scope.error_view().check_session(self.issuer)?;
        if replacements.is_empty() {
            return Ok(());
        }
        self.require_bom_diagnostic(scope.error_view())?;
        let Some(end) = self.next.checked_add(replacements.len()) else {
            return Err(scope
                .error_view()
                .descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        let Some(expected) = self.replacements.get(self.next..end) else {
            return Err(scope
                .error_view()
                .descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        if !std::ptr::eq(expected.as_ptr(), replacements.as_ptr()) {
            return Err(scope
                .error_view()
                .descriptor_contract_failure("text-input-replacement-out-of-order"));
        }
        self.next = end;
        Ok(())
    }

    pub fn record_format_recovery(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        replacement: &TextInputReplacement,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.expect_next(scope.error_view(), replacement)?;
        let line_column = self.line_column(replacement.raw_range()).map_err(|()| {
            scope
                .error_view()
                .descriptor_contract_failure("text-input-recovery-location-invalid")
        })?;
        scope.record_format_recovery_with_location(
            code,
            self.bound_source(),
            replacement.raw_range(),
            Some(line_column),
            message,
        )?;
        self.next += 1;
        Ok(())
    }

    fn classify_next(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        replacement: &TextInputReplacement,
    ) -> ParseResult<()> {
        self.expect_next(scope.error_view(), replacement)?;
        self.next += 1;
        Ok(())
    }

    fn expect_next(
        &self,
        errors: ParseErrorView<'_>,
        replacement: &TextInputReplacement,
    ) -> ParseResult<()> {
        self.require_bom_diagnostic(errors)?;
        let Some(expected) = self.replacements.get(self.next) else {
            return Err(errors.descriptor_contract_failure("text-input-replacement-overclassified"));
        };
        if !std::ptr::eq(expected, replacement) {
            return Err(errors.descriptor_contract_failure("text-input-replacement-out-of-order"));
        }
        Ok(())
    }

    fn require_bom_diagnostic(&self, errors: ParseErrorView<'_>) -> ParseResult<()> {
        errors.check_session(self.issuer)?;
        if self.pending_bom_diagnostic.is_none() {
            return Ok(());
        }
        Err(errors.descriptor_contract_failure("text-input-bom-diagnostic-unclassified"))
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

    fn finish(self, scope: &mut ParseServices<'_, '_, 'session>) -> ParseResult<()> {
        self.require_bom_diagnostic(scope.error_view())?;
        if self.next == self.replacements.len() {
            return Ok(());
        }
        Err(scope
            .error_view()
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

pub(crate) fn with_conformed_text_input<'region, 'op, 'session, 'source, T>(
    scope: &mut ParseChildren<'region, 'op, 'session>,
    profile: Option<&TextProfile>,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    action: impl for<'input> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'input ConformedTextInput<'session>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<TextConformanceResult<T>> {
    let _ = text_input_range(scope.error_view(), &source, options)?;
    let facts = match text_profile_facts(profile, options.decoding_mode) {
        Ok(facts) => facts,
        Err(violation) => return Ok(Err(violation)),
    };
    with_conformed_text_facts(scope, facts, source, options, action)
}

fn with_conformed_text_facts<'region, 'op, 'session, 'source, T>(
    scope: &mut ParseChildren<'region, 'op, 'session>,
    facts: TextProfileFacts,
    source: DiagnosticSource<'source>,
    options: TextInputOptions,
    action: impl for<'input> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'input ConformedTextInput<'session>,
        &mut TextInputRecoveryTracker<'input, 'session>,
    ) -> ParseResult<T>,
) -> ParseResult<TextConformanceResult<T>> {
    let bytes = text_input_bytes(scope.error_view(), &source, options)?;
    let primary_source = scope.diagnostic_source_is_primary(&source);
    let input = match conform_text_input(scope, &facts, bytes, options, primary_source)? {
        Ok(input) => input,
        Err(violation) => return Ok(Err(violation)),
    };
    let mut recoveries = TextInputRecoveryTracker {
        issuer: scope.error_view().session_issuer(),
        source: DiagnosticSource::new(source.authority().clone(), source.path(), source.bytes()),
        line_number_base: input.line_number_base,
        line_starts: input.line_starts.as_slice(),
        pending_bom_diagnostic: input.bom().filter(|bom| bom.requires_diagnostic()),
        replacements: input.replacements.as_slice(),
        next: 0,
    };
    let value = action(scope, &input, &mut recoveries)?;
    recoveries.finish(&mut scope.services()?)?;
    Ok(Ok(value))
}

fn text_input_bytes<'source>(
    errors: ParseErrorView<'_>,
    source: &'source DiagnosticSource<'_>,
    options: TextInputOptions,
) -> ParseResult<&'source [u8]> {
    let range = text_input_range(errors, source, options)?;
    source
        .bytes()
        .get(range)
        .ok_or_else(|| errors.descriptor_contract_failure("text-input-source-offset-out-of-bounds"))
}

fn text_input_range(
    errors: ParseErrorView<'_>,
    source: &DiagnosticSource<'_>,
    options: TextInputOptions,
) -> ParseResult<Range<usize>> {
    let start = usize::try_from(options.raw_base).map_err(|_| {
        errors.descriptor_contract_failure("text-input-source-offset-out-of-bounds")
    })?;
    let end = match options.raw_end {
        Some(raw_end) => usize::try_from(raw_end).map_err(|_| {
            errors.descriptor_contract_failure("text-input-source-offset-out-of-bounds")
        })?,
        None => source.bytes().len(),
    };
    if start > end || end > source.bytes().len() {
        return Err(errors.descriptor_contract_failure("text-input-source-offset-out-of-bounds"));
    }
    Ok(start..end)
}

fn conform_text_input<'session>(
    scope: &mut ParseChildren<'_, '_, 'session>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
    primary_source: bool,
) -> ParseResult<TextConformanceResult<ConformedTextInput<'session>>> {
    let raw_len = usize_to_u64(scope.error_view(), bytes.len(), options.resource)?;
    let raw_limit = options.raw_limit.unwrap_or(TextRawLimit {
        resource: options.resource,
        limit: scope.error_view().limits().max_primary_asset_bytes,
    });
    scope
        .error_view()
        .enforce_resource_limit(raw_limit.resource, raw_len, raw_limit.limit)?;

    let plan = match plan_text_input(scope, facts, bytes, options) {
        Ok(plan) => plan,
        Err(TextInputScanError::Scope(failure)) => return Err(failure),
        Err(TextInputScanError::Violation(violation)) => return Ok(Err(violation)),
    };
    build_text_input(scope, facts, bytes, options, plan, primary_source).map(Ok)
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
    fn add<T>(&mut self, errors: ParseErrorView<'_>, resource: &'static str) -> ParseResult<bool> {
        let len = checked_add_usize(errors, self.len, 1, resource)?;
        if len <= self.capacity {
            self.len = len;
            return Ok(false);
        }

        let replacement_plan = metered_vec_replacement_plan::<T>(0, 0, len)
            .map_err(|_| errors.resource_limit_exceeded(resource, u64::MAX))?;
        self.len = len;
        self.capacity = replacement_plan.envelope_capacity();
        self.envelope_bytes = replacement_plan
            .envelope_delta_bytes()
            .map_err(|_| errors.resource_limit_exceeded(resource, u64::MAX))?;
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

struct TextPlanCursor {
    offset: usize,
    line_len: usize,
    work: WorkCursor,
}

enum TextPlanRegionExit {
    Exhausted,
    RawPoll,
    DebtPoll,
    Violation(TextConformanceViolation),
}

fn scan_text_plan_region(
    reads: &mut ParseReadRow<'_, '_, '_>,
    errors: ParseErrorView<'_>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
    plan: &mut TextInputPlan,
    cursor: &mut TextPlanCursor,
) -> ParseResult<TextPlanRegionExit> {
    while cursor.offset < bytes.len() {
        if cursor
            .work
            .poll_before_unit_due(facts, bytes, cursor.offset)
        {
            return Ok(TextPlanRegionExit::RawPoll);
        }
        if !reads.ready_for_read()? {
            return Ok(TextPlanRegionExit::DebtPoll);
        }
        let unit = classify_text_unit(facts, options.decoding_mode, bytes, cursor.offset);
        reads.complete_read(unit.raw_len() as u64)?;
        let unit_end = checked_add_usize(errors, cursor.offset, unit.raw_len(), options.resource)?;
        match unit {
            TextUnit::Valid { raw_len, newline } => {
                plan.decoded_len =
                    checked_add_usize(errors, plan.decoded_len, raw_len, options.resource)?;
                enforce_usize_limit(
                    errors,
                    plan.decoded_len,
                    options.decoded_resource,
                    errors.limits().max_decoded_utf8_bytes,
                )?;
                if newline.is_some() {
                    enforce_usize_limit(
                        errors,
                        cursor.line_len,
                        options.line_resource,
                        errors.limits().max_line_bytes as u64,
                    )?;
                    if unit_end < bytes.len() {
                        plan.line_index_limit_dirty |= plan
                            .line_starts
                            .add::<TextInputLineStart>(errors, options.line_index_resource)?;
                    }
                    enforce_line_index_limit(errors, plan, options)?;
                    cursor.line_len = 0;
                } else {
                    cursor.line_len =
                        checked_add_usize(errors, cursor.line_len, raw_len, options.resource)?;
                }
            }
            TextUnit::Replacement { .. } => {
                plan.decoded_len = checked_add_usize(
                    errors,
                    plan.decoded_len,
                    '\u{fffd}'.len_utf8(),
                    options.resource,
                )?;
                enforce_usize_limit(
                    errors,
                    plan.decoded_len,
                    options.decoded_resource,
                    errors.limits().max_decoded_utf8_bytes,
                )?;
                plan.line_index_limit_dirty |= plan
                    .replacements
                    .add::<TextInputReplacement>(errors, options.line_index_resource)?;
                cursor.line_len = checked_add_usize(
                    errors,
                    cursor.line_len,
                    '\u{fffd}'.len_utf8(),
                    options.resource,
                )?;
                enforce_line_index_limit(errors, plan, options)?;
            }
            TextUnit::Violation { kind, .. } => {
                return Ok(TextPlanRegionExit::Violation(TextConformanceViolation::at(
                    kind,
                    raw_range_parse(errors, options, cursor.offset, unit_end)?,
                )));
            }
        }
        cursor.offset = unit_end;
        #[cfg(test)]
        tests::classified_unit(tests::ScanPass::Planning, cursor.offset);
        if cursor.offset >= cursor.work.next_poll_at {
            return Ok(TextPlanRegionExit::RawPoll);
        }
    }
    Ok(TextPlanRegionExit::Exhausted)
}

fn plan_text_input(
    scope: &mut ParseChildren<'_, '_, '_>,
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
    let work_amount = usize_to_u64(scope.error_view(), bytes.len(), options.resource)?;
    let group = scope.bind_group([ImportWorkClass::DecodeWork])?;
    let scan = scope.scan_child(
        &group,
        [work_amount],
        |scope| -> ParseResult<StoreScanReturn<TextConformanceResult<()>>> {
            let has_bom = scope.run_returned(|| {
                let has_bom = options.raw_base == 0 && bytes.starts_with(&[0xef, 0xbb, 0xbf]);
                Ok((has_bom, [u64::from(has_bom) * 3]))
            })?;
            if has_bom {
                let raw_range = raw_range_parse(scope.error_view(), options, 0, 3)?;
                match facts.bom_policy {
                    TextBomPolicy::Forbidden => {
                        return Ok(StoreScanReturn::Boundary(Err(
                            TextConformanceViolation::at(
                                TextConformanceViolationKind::ForbiddenBom,
                                raw_range,
                            ),
                        )));
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
                    .add::<TextInputLineStart>(scope.error_view(), options.line_index_resource)?;
            }
            let mut cursor = TextPlanCursor {
                offset: plan.input_start,
                line_len: 0,
                work: WorkCursor::new(),
            };
            let errors = scope.error_view();
            while cursor.offset < bytes.len() {
                match scope.with_read_row(&mut |reads| {
                    scan_text_plan_region(
                        reads,
                        errors,
                        facts,
                        bytes,
                        options,
                        &mut plan,
                        &mut cursor,
                    )
                })? {
                    TextPlanRegionExit::Exhausted => break,
                    TextPlanRegionExit::RawPoll => {
                        scope.poll()?;
                        cursor.work.next_poll_at =
                            cursor.offset.saturating_add(TEXT_INPUT_WORK_CHUNK_BYTES);
                    }
                    TextPlanRegionExit::DebtPoll => scope.poll()?,
                    TextPlanRegionExit::Violation(violation) => {
                        return Ok(StoreScanReturn::Boundary(Err(violation)));
                    }
                }
            }
            enforce_usize_limit(
                scope.error_view(),
                cursor.line_len,
                options.line_resource,
                scope.error_view().limits().max_line_bytes as u64,
            )?;
            enforce_line_index_limit(scope.error_view(), &mut plan, options)?;
            Ok(StoreScanReturn::Exhausted(Ok(())))
        },
    )?;
    match scan {
        Ok(()) => Ok(plan),
        Err(violation) => Err(TextInputScanError::Violation(violation)),
    }
}

fn build_text_input<'session>(
    scope: &mut ParseChildren<'_, '_, 'session>,
    facts: &TextProfileFacts,
    bytes: &[u8],
    options: TextInputOptions,
    plan: TextInputPlan,
    primary_source: bool,
) -> ParseResult<ConformedTextInput<'session>> {
    let mut line_starts = scope
        .services()?
        .metered_vec(ImportStorageClass::LineIndex, options.line_index_resource);
    line_starts.try_reserve(&mut scope.services()?, plan.line_starts.len)?;
    let mut replacements = scope
        .services()?
        .metered_vec(ImportStorageClass::LineIndex, options.line_index_resource);
    replacements.try_reserve(&mut scope.services()?, plan.replacements.len)?;
    let mut text = scope
        .services()?
        .metered_decoded_text(plan.decoded_len, options.decoded_resource)?;
    let raw_start = raw_offset(scope.error_view(), options, plan.input_start)?;
    let raw_end = raw_offset(scope.error_view(), options, bytes.len())?;

    let work_amount = usize_to_u64(scope.error_view(), bytes.len(), options.resource)?;
    let group = scope.bind_group([ImportWorkClass::DecodeWork])?;
    scope.child(&group, [work_amount], |scope| -> ParseResult<()> {
        let mut cursor = WorkCursor::new();
        if plan.input_start != 0 {
            let prefix = bytes.get(..3).ok_or_else(|| {
                scope
                    .error_view()
                    .descriptor_contract_failure("text-input-plan-revalidation-failed")
            })?;
            let has_bom = scope.run([3], || Ok::<_, ParseFailure>(prefix == [0xef, 0xbb, 0xbf]))?;
            if !has_bom || plan.input_start != 3 {
                return Err(scope
                    .error_view()
                    .descriptor_contract_failure("text-input-plan-revalidation-failed"));
            }
        }
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
            cursor.poll_before_unit(scope, facts, bytes, offset)?;
            let unit = scope.run_returned(|| {
                let unit = TextEmission::read(bytes, offset);
                Ok((unit, [unit.raw_len() as u64]))
            })?;
            let unit_end =
                checked_add_usize(scope.error_view(), offset, unit.raw_len(), options.resource)?;
            match unit {
                TextEmission::Valid { newline, .. } => {
                    if unit_end.saturating_sub(valid_run_start) > TEXT_INPUT_WORK_CHUNK_BYTES {
                        append_valid_run(
                            scope.error_view(),
                            &mut text,
                            bytes,
                            valid_run_start,
                            offset,
                        )?;
                        valid_run_start = offset;
                    }
                    if newline {
                        append_valid_run(
                            scope.error_view(),
                            &mut text,
                            bytes,
                            valid_run_start,
                            unit_end,
                        )?;
                        if unit_end < bytes.len() {
                            line_starts.try_push_reserved(TextInputLineStart {
                                raw_offset: raw_offset(scope.error_view(), options, unit_end)?,
                                decoded_offset: text.len(),
                                replacement_index: replacements.len(),
                            })?;
                        }
                        valid_run_start = unit_end;
                    } else if unit_end.saturating_sub(valid_run_start)
                        >= TEXT_INPUT_WORK_CHUNK_BYTES
                    {
                        append_valid_run(
                            scope.error_view(),
                            &mut text,
                            bytes,
                            valid_run_start,
                            unit_end,
                        )?;
                        valid_run_start = unit_end;
                    }
                }
                TextEmission::Replacement { .. } => {
                    append_valid_run(
                        scope.error_view(),
                        &mut text,
                        bytes,
                        valid_run_start,
                        offset,
                    )?;
                    let raw_range = raw_range_parse(scope.error_view(), options, offset, unit_end)?;
                    let decoded_start = text.len();
                    text.try_push_str_reserved("\u{fffd}")?;
                    let decoded_end = text.len();
                    replacements.try_push_reserved(TextInputReplacement::new(
                        raw_range,
                        decoded_start,
                        decoded_end,
                    ))?;
                    valid_run_start = unit_end;
                }
            }
            offset = unit_end;
            #[cfg(test)]
            tests::classified_unit(tests::ScanPass::Building, offset);
            cursor.poll_if_chunk_complete(scope, offset)?;
        }
        append_valid_run(
            scope.error_view(),
            &mut text,
            bytes,
            valid_run_start,
            bytes.len(),
        )?;
        Ok(())
    })?;

    let text = text.finish_exact(&mut scope.services()?)?;
    if line_starts.len() != plan.line_starts.len || replacements.len() != plan.replacements.len {
        return Err(scope
            .error_view()
            .descriptor_contract_failure("text-input-plan-count-mismatch"));
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
    errors: ParseErrorView<'_>,
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
        .ok_or_else(|| errors.descriptor_contract_failure("text-input-copy-range-invalid"))?;
    let decoded = std::str::from_utf8(chunk)
        .map_err(|_| errors.descriptor_contract_failure("text-input-copy-utf8-invalid"))?;
    text.try_push_str_reserved(decoded)
}

/// Navigation over the same immutable input already accepted by the private planning pass.
/// Construction still validates actual output lengths, ranges and capacity separately.
#[derive(Debug, Clone, Copy)]
enum TextEmission {
    Valid { raw_len: usize, newline: bool },
    Replacement { raw_len: usize },
}

impl TextEmission {
    fn read(bytes: &[u8], offset: usize) -> Self {
        if let Some((_, raw_len)) = newline_at(bytes, offset) {
            return Self::Valid {
                raw_len,
                newline: true,
            };
        }
        if bytes.get(offset).is_some_and(u8::is_ascii) {
            return Self::Valid {
                raw_len: 1,
                newline: false,
            };
        }
        match classify_utf8_unit(bytes, offset) {
            Utf8Unit::Valid(raw_len) => Self::Valid {
                raw_len,
                newline: false,
            },
            Utf8Unit::Invalid(raw_len) => Self::Replacement { raw_len },
        }
    }

    const fn raw_len(self) -> usize {
        match self {
            Self::Valid { raw_len, .. } | Self::Replacement { raw_len } => raw_len,
        }
    }
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

impl TextUnit {
    const fn raw_len(self) -> usize {
        match self {
            Self::Valid { raw_len, .. }
            | Self::Replacement { raw_len }
            | Self::Violation { raw_len, .. } => raw_len,
        }
    }
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

#[derive(Debug)]
struct WorkCursor {
    next_poll_at: usize,
}

impl WorkCursor {
    const fn new() -> Self {
        Self {
            next_poll_at: TEXT_INPUT_WORK_CHUNK_BYTES,
        }
    }

    fn poll_before_unit(
        &mut self,
        scope: &mut ParseParser<'_, '_, '_, '_, '_, 1, true>,
        facts: &TextProfileFacts,
        bytes: &[u8],
        offset: usize,
    ) -> ParseResult<()> {
        if self.poll_before_unit_due(facts, bytes, offset) {
            scope.poll()?;
            self.next_poll_at = offset.saturating_add(TEXT_INPUT_WORK_CHUNK_BYTES);
        }
        Ok(())
    }

    fn poll_before_unit_due(&self, facts: &TextProfileFacts, bytes: &[u8], offset: usize) -> bool {
        if offset.saturating_add(4) <= self.next_poll_at {
            return false;
        }
        // Only bounded width lookahead precedes the poll; semantic classification follows it.
        let raw_len = text_unit_raw_len(facts, bytes, offset);
        offset.saturating_add(raw_len) > self.next_poll_at
    }

    fn poll_if_chunk_complete(
        &mut self,
        scope: &mut ParseParser<'_, '_, '_, '_, '_, 1, true>,
        offset: usize,
    ) -> ParseResult<()> {
        if offset >= self.next_poll_at {
            scope.poll()?;
            self.next_poll_at = offset.saturating_add(TEXT_INPUT_WORK_CHUNK_BYTES);
        }
        Ok(())
    }
}

fn text_unit_raw_len(facts: &TextProfileFacts, bytes: &[u8], offset: usize) -> usize {
    if let Some((_, raw_len)) = newline_at(bytes, offset) {
        return raw_len;
    }
    if bytes.get(offset).is_none_or(u8::is_ascii) {
        return 1;
    }
    match classify_utf8_unit(bytes, offset) {
        Utf8Unit::Valid(_) if facts.lexical_mode == TextLexicalMode::AsciiOnly => 1,
        Utf8Unit::Valid(raw_len) | Utf8Unit::Invalid(raw_len) => raw_len,
    }
}

fn enforce_usize_limit(
    errors: ParseErrorView<'_>,
    value: usize,
    resource: &'static str,
    limit: u64,
) -> ParseResult<()> {
    if u64::try_from(value).is_ok_and(|attempted| attempted <= limit) {
        return Ok(());
    }
    Err(errors.resource_limit_exceeded(resource, limit))
}

fn enforce_line_index_limit(
    errors: ParseErrorView<'_>,
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
        return Err(errors.resource_limit_exceeded(options.line_index_resource, u64::MAX));
    };
    let limit = errors.limits().max_line_index_bytes;
    if bytes > limit {
        return Err(errors.resource_limit_exceeded(options.line_index_resource, limit));
    }
    plan.line_index_limit_dirty = false;
    Ok(())
}

fn raw_range_parse(
    errors: ParseErrorView<'_>,
    options: TextInputOptions,
    start: usize,
    end: usize,
) -> ParseResult<DiagnosticByteRange> {
    let start = raw_offset(errors, options, start)?;
    let end = raw_offset(errors, options, end)?;
    DiagnosticByteRange::new(start, end)
        .map_err(|_| errors.descriptor_contract_failure("text-input-raw-range-invalid"))
}

fn raw_offset(
    errors: ParseErrorView<'_>,
    options: TextInputOptions,
    offset: usize,
) -> ParseResult<u64> {
    let offset = usize_to_u64(errors, offset, options.resource)?;
    options
        .raw_base
        .checked_add(offset)
        .ok_or_else(|| errors.resource_limit_exceeded(options.resource, u64::MAX))
}

fn checked_add_usize(
    errors: ParseErrorView<'_>,
    left: usize,
    right: usize,
    resource: &'static str,
) -> ParseResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| errors.resource_limit_exceeded(resource, u64::MAX))
}

fn usize_to_u64(
    errors: ParseErrorView<'_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| errors.resource_limit_exceeded(resource, u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, Format,
        FormatCapability, FormatDescriptor, FormatEncoding, ImportCause, ImportDimension,
        ImportOperationControl, ImportRequest, OptionCompileLimits, ReadConfidence, ReadHint,
        RegistryBuildLimits, RegistryBuilder, ScopeAbortCause, TextHardCaps,
    };
    use baozi_core::FormatId;
    use baozi_io::AssetPath;
    use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
    use std::{cell::RefCell, marker::PhantomData, rc::Rc};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ScanPass {
        Planning,
        Building,
    }

    struct FrontierProbe {
        pass: ScanPass,
        trigger_offset: usize,
        completed: usize,
        raised_at: Option<usize>,
        token: Option<CancellationToken>,
    }

    thread_local! {
        static FRONTIER_PROBE: RefCell<Option<FrontierProbe>> = const { RefCell::new(None) };
    }

    struct FrontierProbeGuard(PhantomData<Rc<()>>);

    impl FrontierProbeGuard {
        fn install(pass: ScanPass, trigger_offset: usize, token: CancellationToken) -> Self {
            FRONTIER_PROBE.with(|slot| {
                let mut slot = slot.borrow_mut();
                assert!(slot.is_none());
                *slot = Some(FrontierProbe {
                    pass,
                    trigger_offset,
                    completed: 0,
                    raised_at: None,
                    token: Some(token),
                });
            });
            Self(PhantomData)
        }

        fn finish(self) -> FrontierProbe {
            FRONTIER_PROBE.with(|slot| {
                slot.borrow_mut()
                    .take()
                    .expect("active text frontier probe")
            })
        }
    }

    impl Drop for FrontierProbeGuard {
        fn drop(&mut self) {
            let probe = FRONTIER_PROBE.with(|slot| slot.borrow_mut().take());
            drop(probe);
        }
    }

    pub(super) fn classified_unit(pass: ScanPass, completed: usize) {
        let token = FRONTIER_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let probe = slot.as_mut()?;
            if probe.pass != pass {
                return None;
            }
            probe.completed = completed;
            if completed == probe.trigger_offset && probe.raised_at.is_none() {
                probe.raised_at = Some(completed);
                probe.token.take()
            } else {
                None
            }
        });
        if let Some(token) = token {
            token.cancel();
        }
    }

    struct FrontierImporter;

    impl Format for FrontierImporter {
        type Options = crate::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut crate::RegistryBuildScope<'_>,
        ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
            crate::NoFormatOptions::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.text-frontier").unwrap(),
                "Text Frontier Probe",
            )
            .extensions(["txt"])
            .encoding(FormatEncoding::Text)
            .text_profile(
                TextProfile::builder(
                    TextHardCaps::new(65_536, 65_536, 65_536, 65_536, 65_536).unwrap(),
                )
                .source_encodings([TextSourceEncoding::Utf8])
                .bom_policy(TextBomPolicy::Forbidden)
                .newlines([TextNewline::Lf])
                .lexical_mode(TextLexicalMode::Utf8Document)
                .decoding_modes([
                    TextDecodingMode::RejectInvalid,
                    TextDecodingMode::ReplaceInvalidUtf8,
                ])
                .control_policy(TextControlPolicy::RejectAll)
                .build()
                .unwrap(),
            )
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .unwrap()
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _options: &Self::Options,
            context: &mut crate::ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            context.with_execution(|exec| -> ParseResult<()> {
                let source = exec.services()?.diagnostic_source();
                exec.with_conformed_text_input(
                    source,
                    TextInputOptions::new(TextDecodingMode::ReplaceInvalidUtf8, "frontier-text"),
                    |_exec, _input, _recoveries| -> ParseResult<()> {
                        panic!("the text frontier must observe cancellation")
                    },
                )?
                .expect("the text profile must permit the cancellation fixture");
                unreachable!("the text scan must stop before returning a conformed input")
            })
        }
    }

    #[test]
    fn real_text_passes_observe_physical_byte_frontiers_without_splitting_utf8() {
        let mut registry = RegistryBuilder::new();
        registry.register(FrontierImporter).unwrap();
        let plan = registry
            .freeze(RegistryBuildLimits::default())
            .unwrap()
            .plan(&ImportRequest::new(), OptionCompileLimits::default())
            .unwrap();
        for pass in [ScanPass::Planning, ScanPass::Building] {
            for shape in 0..3 {
                let mut bytes = vec![b'a'; 3 * TEXT_INPUT_WORK_CHUNK_BYTES + 8];
                let (trigger, expected_prefix) = if shape == 1 {
                    bytes[4095..4099].copy_from_slice(b"\xf0\x9f\x92\xa9");
                    (4095, 4095)
                } else {
                    if shape == 2 {
                        // Replacement is legal in the planning pass; the selected pass must
                        // still stop before this later invalid unit is classified.
                        bytes[8192] = 0xff;
                    }
                    (1, TEXT_INPUT_WORK_CHUNK_BYTES)
                };
                let token = CancellationToken::new();
                let probe = FrontierProbeGuard::install(pass, trigger, token.clone());
                let failure = plan
                    .read_bytes(
                        AssetPath::new("frontier.txt").unwrap(),
                        &bytes,
                        ImportOperationControl::default().with_cancellation(token),
                    )
                    .expect_err("a classified unit raises cancellation in the selected text pass")
                    .into_operation_failure()
                    .expect("ordinary cancellation failure");
                let probe = probe.finish();
                assert_eq!(probe.raised_at, Some(trigger), "{pass:?}, shape {shape}");
                assert_eq!(probe.completed, expected_prefix, "{pass:?}, shape {shape}");
                assert!(probe.completed - trigger < TEXT_INPUT_WORK_CHUNK_BYTES);
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Scope(abort) if matches!(
                        abort.cause(),
                        ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal })
                    )
                ));
                let prior_pass_bytes = if pass == ScanPass::Building {
                    bytes.len() as u64
                } else {
                    0
                };
                let decode = failure
                    .ledger()
                    .dimension(ImportDimension::DecodeWork)
                    .unwrap();
                assert_eq!(decode.attempted(), prior_pass_bytes + bytes.len() as u64);
                assert_eq!(decode.consumed(), prior_pass_bytes + expected_prefix as u64);
                assert_eq!(failure.ledger().current_live_bytes(), 0);
                assert_eq!(failure.ledger().active_reservations(), 0);
                assert_eq!(failure.ledger().committed_canonical_members(), 0);
                assert!(failure.diagnostics().is_empty());
            }
        }
    }

    struct AllocationFailureImporter;

    impl Format for AllocationFailureImporter {
        type Options = crate::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut crate::RegistryBuildScope<'_>,
        ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
            crate::NoFormatOptions::compile_defaults(scope)
        }

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
            _scope: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _options: &Self::Options,
            scope: &mut crate::ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            scope.with_execution(|exec| -> ParseResult<()> {
                crate::scope::inject_next_metered_allocation_failure();
                let source = exec.services()?.diagnostic_source();
                exec.with_conformed_text_input(
                    source,
                    TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text")
                        .with_line_index_resource("test-text-line-index"),
                    |_exec, _input, _recoveries| Ok(()),
                )?
                .expect("the allocation fixture must otherwise conform");
                Ok(())
            })?;
            panic!("injected allocation failure must stop text construction")
        }
    }

    struct CopyFailureImporter;

    impl Format for CopyFailureImporter {
        type Options = crate::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut crate::RegistryBuildScope<'_>,
        ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
            crate::NoFormatOptions::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            AllocationFailureImporter.descriptor()
        }

        fn detect(
            &self,
            probe: &crate::Probe<'_>,
            hint: &ReadHint,
            scope: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            AllocationFailureImporter.detect(probe, hint, scope)
        }

        fn parse(
            &self,
            _options: &Self::Options,
            scope: &mut crate::ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            scope.with_execution(|exec| -> ParseResult<()> {
                let descriptor = self.descriptor();
                let facts = TextProfileFacts::new(descriptor.text_profile().unwrap());
                let options = TextInputOptions::new(TextDecodingMode::RejectInvalid, "test-text");
                let source = exec.services()?.diagnostic_source();
                let bytes = source.bytes();
                let mut children = exec.children()?;
                let mut plan = match plan_text_input(&mut children, &facts, bytes, options) {
                    Ok(plan) => plan,
                    Err(TextInputScanError::Scope(failure)) => return Err(failure),
                    Err(TextInputScanError::Violation(_)) => panic!("copy fixture must conform"),
                };
                // Fault the copy capacity after a valid plan so classification precedes rejection.
                plan.decoded_len = 0;
                let _ = build_text_input(&mut children, &facts, bytes, options, plan, true)?;
                Ok(())
            })?;
            panic!("the shortened decoded capacity must reject the first copy")
        }
    }

    #[test]
    fn raw_unit_frontier_lookahead_keeps_utf8_and_malformed_prefix_widths() {
        let descriptor = AllocationFailureImporter.descriptor();
        let mut facts = TextProfileFacts::new(descriptor.text_profile().unwrap());
        for (bytes, expected) in [
            (b"a".as_slice(), 1),
            (b"\r\n".as_slice(), 2),
            (b"\xc3\xa9".as_slice(), 2),
            (b"\xe2\x82\xac".as_slice(), 3),
            (b"\xf0\x9f\x92\xa9".as_slice(), 4),
            (b"\xff".as_slice(), 1),
            (b"\xe2\x82x".as_slice(), 2),
            (b"\xf0\x9f\x92x".as_slice(), 3),
            (b"\xf0\x9f\x92".as_slice(), 3),
        ] {
            assert_eq!(text_unit_raw_len(&facts, bytes, 0), expected);
            for mode in [
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ] {
                assert_eq!(
                    classify_text_unit(&facts, mode, bytes, 0).raw_len(),
                    expected
                );
            }
        }
        facts.lexical_mode = TextLexicalMode::AsciiOnly;
        assert_eq!(text_unit_raw_len(&facts, b"\xc3\xa9", 0), 1);
        assert_eq!(
            classify_text_unit(&facts, TextDecodingMode::RejectInvalid, b"\xc3\xa9", 0).raw_len(),
            1
        );
    }

    #[test]
    fn copy_failure_keeps_classified_raw_units_without_a_final_completion_replay() {
        for (bytes, attempted, consumed) in [
            (b"a".as_slice(), 2, 2),
            (b"a\n".as_slice(), 4, 4),
            (b"a\nb".as_slice(), 6, 5),
            (b"\xc3\xa9".as_slice(), 4, 4),
        ] {
            let mut registry = RegistryBuilder::new();
            registry.register(CopyFailureImporter).unwrap();
            let request = registry
                .freeze(RegistryBuildLimits::default())
                .unwrap()
                .plan(&ImportRequest::new(), OptionCompileLimits::default())
                .unwrap();
            let error = request
                .read_bytes(
                    AssetPath::new("copy.txt").unwrap(),
                    bytes,
                    ImportOperationControl::default(),
                )
                .expect_err("the first reserved decoded copy must fail");
            let failure = error
                .into_operation_failure()
                .unwrap_or_else(|source| panic!("test session must initialize: {source}"));
            assert!(matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(
                        abort.cause(),
                        ScopeAbortCause::DescriptorContract {
                            code: "decoded-text-length-exhausted",
                        }
                    )
            ));
            let decode = failure
                .ledger()
                .dimension(ImportDimension::DecodeWork)
                .unwrap();
            assert_eq!(
                (decode.attempted(), decode.consumed()),
                (attempted, consumed)
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
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
            .plan(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("test request must compile");
        let error = request
            .read_bytes(
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
