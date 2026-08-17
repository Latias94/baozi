use crate::error::{StlFailure, StlFailureCode};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_core::Vec3;
use baozi_import::{
    DetectionResult, DetectionScope, DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity,
    ImportProgressPhase, ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredDecodedText,
    MeteredString, MeteredVec, ParseFailure, ParseResult, ParseScope,
};

const CHECKPOINT_CHUNK_BYTES: usize = 4 * 1024;

pub(crate) fn parse<'scope>(
    scope: &'scope ParseScope<'_>,
    bytes: &[u8],
) -> ParseResult<ParsedStl<'scope>> {
    let text = validate_text_profile_scoped(scope, bytes)?;
    AsciiParser::new(scope).parse(text.as_str())
}

pub(crate) fn validate_text_profile_for_detection<'bytes>(
    scope: &DetectionScope<'_>,
    bytes: &'bytes [u8],
) -> DetectionResult<Option<&'bytes str>> {
    let profile = account_detection_work(scope, bytes.len(), || {
        validate_text_profile_range(bytes, 0, bytes.len())
    })?;
    if profile.is_err() {
        return Ok(None);
    }

    let text = account_detection_work(scope, bytes.len(), || std::str::from_utf8(bytes))?;
    Ok(text.ok())
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

fn validate_text_profile_scoped<'scope>(
    scope: &'scope ParseScope<'_>,
    bytes: &[u8],
) -> ParseResult<MeteredDecodedText<'scope>> {
    let mut decoded = None;
    let mut start = 0usize;
    while start < bytes.len() {
        let end = start
            .checked_add(CHECKPOINT_CHUNK_BYTES)
            .map_or(bytes.len(), |end| end.min(bytes.len()));
        let chunk_bytes = usize_amount(scope, end - start, "stl-decode-work")?;
        scope.with_work(
            ImportWorkClass::DecodeWork,
            chunk_bytes,
            |work| -> ParseResult<()> {
                if decoded.is_none() {
                    decoded = Some(scope.metered_decoded_text(bytes.len(), "stl-decoded-text")?);
                }
                validate_text_profile_range(bytes, start, end)
                    .map_err(|failure| scope.format_failure(failure))?;
                let chunk = bytes.get(start..end).ok_or_else(|| {
                    scope.format_failure(StlFailure::content(
                        StlFailureCode::InternalInvariant,
                        "ascii-profile-range",
                        "invalid-decode-range",
                    ))
                })?;
                let chunk = match std::str::from_utf8(chunk) {
                    Ok(chunk) => chunk,
                    Err(source) => {
                        let byte_offset = start
                            .checked_add(source.valid_up_to())
                            .and_then(|offset| u64::try_from(offset).ok())
                            .ok_or_else(|| scope.allocation_failure("stl-utf8-byte-offset"))?;
                        return Err(scope.format_failure(StlFailure::invalid_utf8_at(byte_offset)));
                    }
                };
                let Some(decoded) = decoded.as_mut() else {
                    return Err(scope.format_failure(StlFailure::content(
                        StlFailureCode::InternalInvariant,
                        "ascii-profile-buffer",
                        "missing-decoded-buffer",
                    )));
                };
                decoded.try_push_str(chunk)?;
                work.complete_all()
            },
        )?;
        start = end;
    }

    match decoded {
        Some(decoded) => decoded.finish_exact(),
        None => scope
            .metered_decoded_text(0, "stl-decoded-text")?
            .finish_exact(),
    }
}

fn validate_text_profile_range(bytes: &[u8], start: usize, end: usize) -> Result<(), StlFailure> {
    if start == 0 && bytes.starts_with(b"\xef\xbb\xbf") {
        return Err(StlFailure::non_ascii(0, "forbidden-bom"));
    }

    let chunk = bytes.get(start..end).ok_or_else(|| {
        StlFailure::content(
            StlFailureCode::InternalInvariant,
            "ascii-profile-range",
            "invalid-scan-range",
        )
    })?;
    for (relative, byte) in chunk.iter().copied().enumerate() {
        let offset = start.checked_add(relative).ok_or_else(|| {
            StlFailure::content(
                StlFailureCode::InternalInvariant,
                "ascii-profile-offset",
                "byte-offset-overflow",
            )
        })?;
        if !byte.is_ascii() {
            return Err(non_ascii_failure(bytes, offset));
        }

        let reason = match byte {
            b'\t' | b'\n' => None,
            b'\r' if offset.checked_add(1).and_then(|next| bytes.get(next)) == Some(&b'\n') => None,
            b'\r' => Some("bare-cr"),
            b'\0' => Some("nul-byte"),
            value if value.is_ascii_control() => Some("forbidden-control"),
            _ => None,
        };
        if let Some(reason) = reason {
            return Err(StlFailure::invalid_control(offset as u64, reason));
        }
    }

    Ok(())
}

fn non_ascii_failure(bytes: &[u8], offset: usize) -> StlFailure {
    const MAX_UTF8_SEQUENCE_BYTES: usize = 4;

    let end = offset
        .checked_add(MAX_UTF8_SEQUENCE_BYTES)
        .map_or(bytes.len(), |end| end.min(bytes.len()));
    if bytes
        .get(offset..end)
        .and_then(|window| std::str::from_utf8(window).err())
        .filter(|source| source.valid_up_to() == 0)
        .is_some()
    {
        return StlFailure::invalid_utf8_at(offset as u64);
    }
    StlFailure::non_ascii(offset as u64, "non-ascii-byte")
}

struct AsciiParser<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    parsed: ParsedStl<'scope>,
    state: State,
    current: Option<SolidBuilder<'scope>>,
}

impl<'scope, 'session> AsciiParser<'scope, 'session> {
    fn new(scope: &'scope ParseScope<'session>) -> Self {
        Self {
            scope,
            parsed: ParsedStl {
                material_color: None,
                meshes: scope.metered_vec(ImportStorageClass::FormatNormalization, "stl-meshes"),
            },
            state: State::Start,
            current: None,
        }
    }

    fn parse(mut self, text: &str) -> ParseResult<ParsedStl<'scope>> {
        let mut byte_offset = 0usize;
        let mut line_number = 0u32;
        let mut token_buffer = TokenBuffer::new(self.scope);
        while byte_offset < text.len() {
            let line_record =
                self.scope
                    .with_work(ImportWorkClass::Records, 1, |work| -> ParseResult<_> {
                        let line_record = next_line(self.scope, text, byte_offset, work)?;
                        work.complete_all()?;
                        Ok(line_record)
                    })?;
            line_number = line_number
                .checked_add(1)
                .ok_or_else(|| self.scope.allocation_failure("stl-line-number"))?;
            let line = line_record.text;
            self.parse_line(
                line,
                line_number,
                byte_offset,
                line_record.raw_line_bytes,
                &mut token_buffer,
            )?;
            byte_offset = line_record.next_offset;
        }
        self.finish_file()?;
        Ok(self.parsed)
    }

    fn parse_line(
        &mut self,
        line: &str,
        line_number: u32,
        line_byte_offset: usize,
        raw_line_bytes: usize,
        token_buffer: &mut TokenBuffer<'scope>,
    ) -> ParseResult<()> {
        account_line_bytes(self.scope, line, raw_line_bytes)?;
        let tokens = tokenize(self.scope, line, token_buffer)?;
        if tokens.is_empty() {
            return Ok(());
        }
        let first = required_token(self.scope, &tokens, 0)?;

        let state = std::mem::replace(&mut self.state, State::Start);
        match state {
            State::Start => self.parse_start_line(line, &tokens, line_number, line_byte_offset),
            State::InSolid => self.parse_solid_line(&tokens, line_number),
            State::NeedOuterLoop { normal } => {
                if tokens_match(&tokens, &["outer", "loop"]) {
                    self.state = State::InLoop {
                        normal,
                        vertices: [Vec3::ZERO; 3],
                        vertex_count: 0,
                    };
                    Ok(())
                } else {
                    Err(self.failure(
                        "outer-loop",
                        "expected-after-facet-normal",
                        line_number,
                        first.column,
                    ))
                }
            }
            State::InLoop {
                normal,
                mut vertices,
                mut vertex_count,
            } => {
                if keyword_is(first, "vertex") {
                    if vertex_count == vertices.len() {
                        return Err(self.failure(
                            "facet",
                            "more-than-three-vertices",
                            line_number,
                            first.column,
                        ));
                    }
                    let vertex = parse_vec3(self.scope, &tokens, "vertex", line_number)?;
                    vertices[vertex_count] = vertex;
                    vertex_count += 1;
                    self.state = State::InLoop {
                        normal,
                        vertices,
                        vertex_count,
                    };
                    Ok(())
                } else if tokens_match(&tokens, &["endloop"]) {
                    if vertex_count != vertices.len() {
                        return Err(self.failure(
                            "facet",
                            "fewer-than-three-vertices",
                            line_number,
                            first.column,
                        ));
                    }
                    self.state = State::NeedEndFacet { normal, vertices };
                    Ok(())
                } else {
                    Err(self.failure(
                        "facet-loop",
                        "expected-vertex-or-endloop",
                        line_number,
                        first.column,
                    ))
                }
            }
            State::NeedEndFacet { normal, vertices } => {
                if tokens_match(&tokens, &["endfacet"]) {
                    self.push_facet(normal, &vertices)?;
                    self.state = State::InSolid;
                    Ok(())
                } else {
                    Err(self.failure("facet", "expected-endfacet", line_number, first.column))
                }
            }
        }
    }

    fn parse_start_line(
        &mut self,
        line: &str,
        tokens: &TokenLine<'_, '_>,
        line_number: u32,
        line_byte_offset: usize,
    ) -> ParseResult<()> {
        let first = required_token(self.scope, tokens, 0)?;
        if !keyword_is(first, "solid") {
            return Err(self.failure("document", "expected-solid", line_number, first.column));
        }
        self.start_solid(line, tokens, line_byte_offset)
    }

    fn parse_solid_line(
        &mut self,
        tokens: &TokenLine<'_, '_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let first = required_token(self.scope, tokens, 0)?;
        if keyword_is(first, "facet") {
            let normal = parse_facet_normal(self.scope, tokens, line_number)?;
            self.state = State::NeedOuterLoop { normal };
            Ok(())
        } else if keyword_is(first, "endsolid") {
            if tokens.len() > 2 {
                let third = required_token(self.scope, tokens, 2)?;
                return Err(self.failure(
                    "endsolid",
                    "too-many-name-tokens",
                    line_number,
                    third.column,
                ));
            }
            self.finish_solid(line_number, first.column)
        } else if keyword_is(first, "solid") {
            Err(self.failure("solid", "nested-solid", line_number, first.column))
        } else {
            Err(self.failure(
                "solid",
                "expected-facet-or-endsolid",
                line_number,
                first.column,
            ))
        }
    }

    fn start_solid(
        &mut self,
        line: &str,
        tokens: &TokenLine<'_, '_>,
        line_byte_offset: usize,
    ) -> ParseResult<()> {
        let scope = self.scope;
        let name_value = trailing_text_after_first_token(line, tokens);
        let name_bytes = name_value
            .map(|name| usize_amount(scope, name.len(), "stl-solid-name"))
            .transpose()?
            .unwrap_or(0);
        scope.with_work_group(
            [
                (ImportWorkClass::Solids, 1),
                (ImportWorkClass::StringBytes, name_bytes),
            ],
            |work| -> ParseResult<()> {
                let name = match name_value {
                    Some(value) => Some(work.run([0, name_bytes], || -> ParseResult<_> {
                        let mut owned = scope.metered_string(
                            ImportStorageClass::SceneConstruction,
                            "stl-solid-name",
                        );
                        owned.try_push_str(value)?;
                        Ok(owned)
                    })?),
                    None => None,
                };
                let first = required_token(scope, tokens, 0)?;
                let token_offset = usize::try_from(first.column)
                    .ok()
                    .and_then(|column| column.checked_sub(1))
                    .and_then(|column| line_byte_offset.checked_add(column))
                    .ok_or_else(|| scope.allocation_failure("stl-solid-byte-offset"))?;
                let token_end = token_offset
                    .checked_add(first.text.len())
                    .ok_or_else(|| scope.allocation_failure("stl-solid-byte-range"))?;
                let start = diagnostic_range(scope, token_offset, token_end)?;
                self.current = Some(SolidBuilder::new(scope, name, start));
                self.state = State::InSolid;
                work.complete([1, 0])
            },
        )
    }

    fn finish_solid(&mut self, line_number: u32, column: u32) -> ParseResult<()> {
        let Some(current) = self.current.take() else {
            return Err(self.failure("endsolid", "without-active-solid", line_number, column));
        };

        if current.face_count == 0 {
            self.scope.emit_diagnostic_with(
                DiagnosticSeverity::Warning,
                DiagnosticCode::new("stl.empty_solid")
                    .expect("static STL diagnostic code must be valid"),
                ImportProgressPhase::Decode,
                self.scope.diagnostic_source(),
                Some(current.start),
                None,
                |details| details.unsigned("facet_count", 0),
                format_args!("ASCII STL solid contains no facets"),
            )?;
        } else {
            self.scope
                .with_work(ImportWorkClass::Meshes, 1, |work| -> ParseResult<()> {
                    work.run_all(|| {
                        self.parsed.meshes.try_push(ParsedMesh {
                            name: current.name,
                            positions: current.positions,
                            normals: current.normals,
                            colors: None,
                        })
                    })
                })?;
        }

        self.state = State::Start;
        Ok(())
    }

    fn push_facet(&mut self, normal: Vec3, vertices: &[Vec3; 3]) -> ParseResult<()> {
        let scope = self.scope;
        let positions = vertices.map(vec3_components);
        let normals = [vec3_components(normal); 3];
        scope.with_work_group(
            [
                (ImportWorkClass::Faces, 1),
                (ImportWorkClass::Vertices, 3),
                (ImportWorkClass::AttributeCells, 6),
            ],
            |work| -> ParseResult<()> {
                let Some(current) = self.current.as_mut() else {
                    return Err(scope.format_failure(StlFailure::content(
                        StlFailureCode::InternalInvariant,
                        "facet",
                        "outside-solid",
                    )));
                };
                work.run([0, 3, 3], || {
                    current.positions.try_extend_from_slice(&positions)
                })?;
                work.run([0, 0, 3], || {
                    current.normals.try_extend_from_slice(&normals)
                })?;
                work.run([1, 0, 0], || {
                    current.face_count = current
                        .face_count
                        .checked_add(1)
                        .ok_or_else(|| scope.allocation_failure("stl-face-counter"))?;
                    Ok(())
                })
            },
        )
    }

    fn finish_file(&self) -> ParseResult<()> {
        let reason = match &self.state {
            State::Start => return Ok(()),
            State::InSolid => "ended-before-endsolid",
            State::NeedOuterLoop { .. } => "ended-before-outer-loop",
            State::InLoop { .. } => "ended-inside-facet-loop",
            State::NeedEndFacet { .. } => "ended-before-endfacet",
        };
        Err(self.scope.format_failure(StlFailure::content(
            StlFailureCode::InvalidStructure,
            "ascii-document",
            reason,
        )))
    }

    fn failure(
        &self,
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> ParseFailure {
        self.scope.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            subject,
            reason,
            line,
            column,
        ))
    }
}

struct LineRecord<'text> {
    text: &'text str,
    raw_line_bytes: usize,
    next_offset: usize,
}

fn account_line_bytes(
    scope: &ParseScope<'_>,
    line: &str,
    raw_line_bytes: usize,
) -> ParseResult<()> {
    let amount = usize_amount(scope, raw_line_bytes, "stl-line-bytes")?;
    scope.with_work(
        ImportWorkClass::LineBytes,
        amount,
        |work| -> ParseResult<()> {
            let mut next_poll = CHECKPOINT_CHUNK_BYTES;
            for (index, _) in line.char_indices() {
                if index >= next_poll {
                    work.poll()?;
                    next_poll = next_poll.saturating_add(CHECKPOINT_CHUNK_BYTES);
                }
            }
            work.complete_all()
        },
    )
}

fn next_line<'text>(
    scope: &ParseScope<'_>,
    text: &'text str,
    start: usize,
    work: &mut ImportWorkScope<'_, '_, 1>,
) -> ParseResult<LineRecord<'text>> {
    let bytes = text.as_bytes();
    let mut cursor = start;
    let mut next_poll = start.saturating_add(4096);
    loop {
        if cursor == bytes.len() || bytes[cursor] == b'\n' {
            let raw_line_bytes = cursor - start;
            let content_end = if cursor > start && bytes[cursor - 1] == b'\r' {
                cursor - 1
            } else {
                cursor
            };
            let line = text.get(start..content_end).ok_or_else(|| {
                scope.format_failure(StlFailure::content(
                    StlFailureCode::InternalInvariant,
                    "ascii-line",
                    "invalid-utf8-boundary",
                ))
            })?;
            let next_offset = if cursor < bytes.len() {
                cursor
                    .checked_add(1)
                    .ok_or_else(|| scope.allocation_failure("stl-line-byte-offset"))?
            } else {
                cursor
            };
            return Ok(LineRecord {
                text: line,
                raw_line_bytes,
                next_offset,
            });
        }

        cursor += 1;
        let line_bytes = cursor - start;
        if line_bytes > scope.limits().max_line_bytes {
            scope.enforce_resource_limit(
                "line-bytes",
                usize_amount(scope, line_bytes, "stl-line-bytes")?,
                usize_amount(scope, scope.limits().max_line_bytes, "stl-line-bytes")?,
            )?;
            return Err(scope.format_failure(StlFailure::content(
                StlFailureCode::InternalInvariant,
                "ascii-line",
                "line-limit-did-not-reject",
            )));
        }
        if cursor >= next_poll {
            work.poll()?;
            next_poll = next_poll.saturating_add(4096);
        }
    }
}

struct SolidBuilder<'scope> {
    name: Option<MeteredString<'scope>>,
    positions: MeteredVec<'scope, [f32; 3]>,
    normals: MeteredVec<'scope, [f32; 3]>,
    face_count: usize,
    start: DiagnosticByteRange,
}

const fn vec3_components(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

impl<'scope> SolidBuilder<'scope> {
    fn new(
        scope: &'scope ParseScope<'_>,
        name: Option<MeteredString<'scope>>,
        start: DiagnosticByteRange,
    ) -> Self {
        Self {
            name,
            positions: scope
                .metered_vec(ImportStorageClass::SceneConstruction, "stl-ascii-positions"),
            normals: scope.metered_vec(ImportStorageClass::SceneConstruction, "stl-ascii-normals"),
            face_count: 0,
            start,
        }
    }
}

enum State {
    Start,
    InSolid,
    NeedOuterLoop {
        normal: Vec3,
    },
    InLoop {
        normal: Vec3,
        vertices: [Vec3; 3],
        vertex_count: usize,
    },
    NeedEndFacet {
        normal: Vec3,
        vertices: [Vec3; 3],
    },
}

#[derive(Debug, Clone, Copy)]
struct Token<'line> {
    text: &'line str,
    column: u32,
}

#[derive(Debug, Clone, Copy)]
struct TokenSpan {
    start: usize,
    end: usize,
    column: u32,
}

struct TokenBuffer<'scope> {
    values: MeteredVec<'scope, TokenSpan>,
}

impl<'scope> TokenBuffer<'scope> {
    fn new(scope: &'scope ParseScope<'_>) -> Self {
        Self {
            values: scope.metered_vec(ImportStorageClass::FormatNormalization, "stl-tokens"),
        }
    }
}

struct TokenLine<'line, 'buffer> {
    line: &'line str,
    spans: &'buffer [TokenSpan],
}

impl<'line> TokenLine<'line, '_> {
    const fn len(&self) -> usize {
        self.spans.len()
    }

    const fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn get(&self, index: usize) -> Option<Token<'line>> {
        let span = self.spans.get(index)?;
        let text = self.line.get(span.start..span.end)?;
        Some(Token {
            text,
            column: span.column,
        })
    }

    fn first(&self) -> Option<Token<'line>> {
        self.get(0)
    }
}

fn tokenize<'line, 'buffer, 'scope>(
    scope: &'scope ParseScope<'_>,
    line: &'line str,
    tokens: &'buffer mut TokenBuffer<'scope>,
) -> ParseResult<TokenLine<'line, 'buffer>> {
    tokens.values.truncate(0);
    let mut start = None;
    let mut next_poll = 4096usize;

    for (index, ch) in line.char_indices() {
        if index >= next_poll {
            scope.poll()?;
            next_poll = next_poll.saturating_add(4096);
        }
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                push_token(scope, tokens, line, token_start, index)?;
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(token_start) = start {
        push_token(scope, tokens, line, token_start, line.len())?;
    }
    Ok(TokenLine {
        line,
        spans: tokens.values.as_slice(),
    })
}

fn push_token<'scope>(
    scope: &'scope ParseScope<'_>,
    tokens: &mut TokenBuffer<'scope>,
    line: &str,
    start: usize,
    end: usize,
) -> ParseResult<()> {
    let text = &line[start..end];
    let token_bytes = usize_amount(scope, text.len(), "stl-token-bytes")?;
    scope.with_work_group(
        [
            (ImportWorkClass::Tokens, 1),
            (ImportWorkClass::TokenBytes, token_bytes),
        ],
        |work| -> ParseResult<()> {
            let column = u32::try_from(start)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| scope.allocation_failure("stl-token-column"))?;
            work.run_all(|| tokens.values.try_push(TokenSpan { start, end, column }))
        },
    )
}

fn parse_facet_normal(
    scope: &ParseScope<'_>,
    tokens: &TokenLine<'_, '_>,
    line_number: u32,
) -> ParseResult<Vec3> {
    if tokens.len() != 5 || !tokens_prefix_matches(tokens, &["facet", "normal"]) {
        return Err(scope.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            "facet-normal",
            "expected-three-components",
            line_number,
            tokens.first().map_or(1, |token| token.column),
        )));
    }
    parse_vec3_components(scope, tokens, 2, line_number)
}

fn parse_vec3(
    scope: &ParseScope<'_>,
    tokens: &TokenLine<'_, '_>,
    keyword: &'static str,
    line_number: u32,
) -> ParseResult<Vec3> {
    if tokens.len() != 4
        || !tokens
            .get(0)
            .is_some_and(|token| keyword_is(token, keyword))
    {
        return Err(scope.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            keyword,
            "expected-three-components",
            line_number,
            tokens.first().map_or(1, |token| token.column),
        )));
    }
    parse_vec3_components(scope, tokens, 1, line_number)
}

fn parse_vec3_components(
    scope: &ParseScope<'_>,
    tokens: &TokenLine<'_, '_>,
    start: usize,
    line_number: u32,
) -> ParseResult<Vec3> {
    Ok(Vec3::new(
        parse_f32(scope, required_token(scope, tokens, start)?, line_number)?,
        parse_f32(
            scope,
            required_token(scope, tokens, start + 1)?,
            line_number,
        )?,
        parse_f32(
            scope,
            required_token(scope, tokens, start + 2)?,
            line_number,
        )?,
    ))
}

fn required_token<'line>(
    scope: &ParseScope<'_>,
    tokens: &TokenLine<'line, '_>,
    index: usize,
) -> ParseResult<Token<'line>> {
    tokens.get(index).ok_or_else(|| {
        scope.format_failure(StlFailure::content(
            StlFailureCode::InternalInvariant,
            "ascii-token-range",
            "invalid-token-span",
        ))
    })
}

fn parse_f32(scope: &ParseScope<'_>, token: Token<'_>, line_number: u32) -> ParseResult<f32> {
    let value = token
        .text
        .parse::<f32>()
        .map_err(|_| scope.format_failure(StlFailure::invalid_float(line_number, token.column)))?;
    if !value.is_finite() {
        return Err(scope.format_failure(StlFailure::located(
            StlFailureCode::NonFiniteNumber,
            "float",
            "non-finite",
            line_number,
            token.column,
        )));
    }
    Ok(value)
}

fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

fn diagnostic_range(
    scope: &ParseScope<'_>,
    start: usize,
    end: usize,
) -> ParseResult<DiagnosticByteRange> {
    let start = usize_amount(scope, start, "stl-diagnostic-byte-range")?;
    let end = usize_amount(scope, end, "stl-diagnostic-byte-range")?;
    DiagnosticByteRange::new(start, end).map_err(|_| {
        scope.format_failure(StlFailure::content(
            StlFailureCode::InternalInvariant,
            "diagnostic-byte-range",
            "invalid-byte-range",
        ))
    })
}

fn trailing_text_after_first_token<'line>(
    line: &'line str,
    tokens: &TokenLine<'line, '_>,
) -> Option<&'line str> {
    let first = tokens.first()?;
    let start = usize::try_from(first.column)
        .ok()?
        .checked_sub(1)?
        .checked_add(first.text.len())?;
    let text = line.get(start..)?.trim();
    (!text.is_empty()).then_some(text)
}

fn tokens_match(tokens: &TokenLine<'_, '_>, expected: &[&str]) -> bool {
    tokens.len() == expected.len() && tokens_prefix_matches(tokens, expected)
}

fn tokens_prefix_matches(tokens: &TokenLine<'_, '_>, expected: &[&str]) -> bool {
    expected.iter().enumerate().all(|(index, expected)| {
        tokens
            .get(index)
            .is_some_and(|token| keyword_is(token, expected))
    })
}

fn keyword_is(token: Token<'_>, expected: &str) -> bool {
    token.text.eq_ignore_ascii_case(expected)
}
