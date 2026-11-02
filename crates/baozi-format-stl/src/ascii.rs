use crate::error::{StlFailure, StlFailureCode};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_import::authoring::Vec3;
use baozi_import::{
    ConformedTextInput, DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity,
    ImportProgressPhase, ImportStorageClass, ImportWorkClass, ImportWorkGroup, MeteredString,
    MeteredVec, ParseExecution, ParseFailure, ParseResult, TextConformanceViolation,
    TextConformanceViolationKind, TextDecodingMode, TextDiagnosticLocation, TextInputLine,
    TextInputOptions,
};

const CHECKPOINT_CHUNK_BYTES: usize = 4 * 1024;

pub(crate) fn parse<'session>(
    exec: &mut ParseExecution<'_, 'session>,
) -> ParseResult<ParsedStl<'session>> {
    let options = TextInputOptions::new(TextDecodingMode::RejectInvalid, "stl-text-input")
        .with_decoded_resource("stl-decoded-text")
        .with_line_resource("line-bytes")
        .with_line_index_resource("stl-line-index-bytes");
    exec.with_conformed_text_input(
        exec.diagnostic_source(),
        options,
        |exec, input, _recoveries| AsciiParser::new(exec)?.parse(exec, input),
    )?
    .map_err(|violation| text_conformance_failure(exec, violation))
}

fn text_conformance_failure(
    exec: &mut ParseExecution<'_, '_>,
    violation: TextConformanceViolation,
) -> ParseFailure {
    let raw_offset = violation.raw_range().map(DiagnosticByteRange::start);
    match violation.kind() {
        TextConformanceViolationKind::ForbiddenBom => {
            byte_conformance_failure(exec, raw_offset, "forbidden-bom-range-missing", |offset| {
                StlFailure::non_ascii(offset, "forbidden-bom")
            })
        }
        TextConformanceViolationKind::InvalidUtf8 => byte_conformance_failure(
            exec,
            raw_offset,
            "invalid-utf8-range-missing",
            StlFailure::invalid_utf8_at,
        ),
        TextConformanceViolationKind::UnsupportedNewline => byte_conformance_failure(
            exec,
            raw_offset,
            "unsupported-newline-range-missing",
            |offset| StlFailure::invalid_control(offset, "bare-cr"),
        ),
        TextConformanceViolationKind::ForbiddenControl => {
            let reason = raw_offset
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| exec.primary_bytes().get(offset))
                .filter(|&&byte| byte == b'\0')
                .map_or("forbidden-control", |_| "nul-byte");
            byte_conformance_failure(
                exec,
                raw_offset,
                "forbidden-control-range-missing",
                |offset| StlFailure::invalid_control(offset, reason),
            )
        }
        TextConformanceViolationKind::NonAscii => {
            byte_conformance_failure(exec, raw_offset, "non-ascii-range-missing", |offset| {
                StlFailure::non_ascii(offset, "non-ascii-byte")
            })
        }
        TextConformanceViolationKind::MissingTextProfile => {
            internal_text_conformance_failure(exec, "missing-text-profile")
        }
        TextConformanceViolationKind::UnsupportedSourceEncoding => {
            internal_text_conformance_failure(exec, "unsupported-source-encoding")
        }
        TextConformanceViolationKind::DecodingModeNotDeclared => {
            internal_text_conformance_failure(exec, "decoding-mode-not-declared")
        }
        _ => internal_text_conformance_failure(exec, "unknown-violation"),
    }
}

fn byte_conformance_failure(
    exec: &mut ParseExecution<'_, '_>,
    raw_offset: Option<u64>,
    missing_range_reason: &'static str,
    failure: impl FnOnce(u64) -> StlFailure,
) -> ParseFailure {
    match raw_offset {
        Some(offset) => exec.format_failure(failure(offset)),
        None => internal_text_conformance_failure(exec, missing_range_reason),
    }
}

fn internal_text_conformance_failure(
    exec: &mut ParseExecution<'_, '_>,
    reason: &'static str,
) -> ParseFailure {
    exec.format_failure(StlFailure::content(
        StlFailureCode::InternalInvariant,
        "text-conformance",
        reason,
    ))
}

struct AsciiParser<'region, 'session> {
    record_work: ImportWorkGroup<'region, 1>,
    line_work: ImportWorkGroup<'region, 1>,
    token_work: ImportWorkGroup<'region, 2>,
    solid_work: ImportWorkGroup<'region, 2>,
    mesh_work: ImportWorkGroup<'region, 1>,
    facet_work: ImportWorkGroup<'region, 3>,
    parsed: ParsedStl<'session>,
    state: State,
    current: Option<SolidBuilder<'session>>,
}

impl<'region, 'session> AsciiParser<'region, 'session> {
    fn new(exec: &mut ParseExecution<'region, 'session>) -> ParseResult<Self> {
        Ok(Self {
            record_work: exec.bind_work_group([ImportWorkClass::Records])?,
            line_work: exec.bind_work_group([ImportWorkClass::LineBytes])?,
            token_work: exec
                .bind_work_group([ImportWorkClass::Tokens, ImportWorkClass::TokenBytes])?,
            solid_work: exec
                .bind_work_group([ImportWorkClass::Solids, ImportWorkClass::StringBytes])?,
            mesh_work: exec.bind_work_group([ImportWorkClass::Meshes])?,
            facet_work: exec.bind_work_group([
                ImportWorkClass::Faces,
                ImportWorkClass::Vertices,
                ImportWorkClass::AttributeCells,
            ])?,
            parsed: ParsedStl {
                material_color: None,
                meshes: exec.metered_vec(ImportStorageClass::FormatNormalization, "stl-meshes"),
            },
            state: State::Start,
            current: None,
        })
    }

    fn parse(
        mut self,
        exec: &mut ParseExecution<'region, 'session>,
        input: &ConformedTextInput<'session>,
    ) -> ParseResult<ParsedStl<'session>> {
        let mut token_buffer = TokenBuffer::new(exec);
        for line in input.lines() {
            exec.with_work(&self.record_work, [1], |_exec, work| work.complete_all())?;
            let line_number = u32::try_from(line.number())
                .map_err(|_| exec.allocation_failure("stl-line-number"))?;
            self.parse_line(exec, &line, line_number, &mut token_buffer)?;
        }
        self.finish_file(exec)?;
        Ok(self.parsed)
    }

    fn parse_line(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        line: &TextInputLine<'_, 'session>,
        line_number: u32,
        token_buffer: &mut TokenBuffer<'session>,
    ) -> ParseResult<()> {
        let text = line.text();
        account_line_bytes(exec, &self.line_work, text)?;
        let tokens = tokenize(exec, &self.token_work, text, token_buffer)?;
        if tokens.is_empty() {
            return Ok(());
        }
        let first = required_token(exec, &tokens, 0)?;

        let state = std::mem::replace(&mut self.state, State::Start);
        match state {
            State::Start => self.parse_start_line(exec, line, text, &tokens, line_number),
            State::InSolid => self.parse_solid_line(exec, text, &tokens, line_number),
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
                        exec,
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
                            exec,
                            "facet",
                            "more-than-three-vertices",
                            line_number,
                            first.column,
                        ));
                    }
                    let vertex = parse_vec3(exec, &tokens, "vertex", line_number)?;
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
                            exec,
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
                        exec,
                        "facet-loop",
                        "expected-vertex-or-endloop",
                        line_number,
                        first.column,
                    ))
                }
            }
            State::NeedEndFacet { normal, vertices } => {
                if tokens_match(&tokens, &["endfacet"]) {
                    self.push_facet(exec, normal, &vertices)?;
                    self.state = State::InSolid;
                    Ok(())
                } else {
                    Err(self.failure(
                        exec,
                        "facet",
                        "expected-endfacet",
                        line_number,
                        first.column,
                    ))
                }
            }
        }
    }

    fn parse_start_line(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        decoded_line: &TextInputLine<'_, 'session>,
        line: &str,
        tokens: &TokenLine<'_, '_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let first = required_token(exec, tokens, 0)?;
        if !keyword_is(first, "solid") {
            return Err(self.failure(
                exec,
                "document",
                "expected-solid",
                line_number,
                first.column,
            ));
        }
        self.start_solid(exec, decoded_line, line, tokens)
    }

    fn parse_solid_line(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        line: &str,
        tokens: &TokenLine<'_, '_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let first = required_token(exec, tokens, 0)?;
        if keyword_is(first, "facet") {
            let normal = parse_facet_normal(exec, tokens, line_number)?;
            self.state = State::NeedOuterLoop { normal };
            Ok(())
        } else if keyword_is(first, "endsolid") {
            let name = trailing_text_after_first_token(line, tokens);
            self.finish_solid(exec, name, line_number, first.column)
        } else if keyword_is(first, "solid") {
            Err(self.failure(exec, "solid", "nested-solid", line_number, first.column))
        } else {
            Err(self.failure(
                exec,
                "solid",
                "expected-facet-or-endsolid",
                line_number,
                first.column,
            ))
        }
    }

    fn start_solid(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        decoded_line: &TextInputLine<'_, 'session>,
        line: &str,
        tokens: &TokenLine<'_, '_>,
    ) -> ParseResult<()> {
        let name_value = trailing_text_after_first_token(line, tokens);
        let name_bytes = name_value
            .map(|name| usize_amount(exec, name.len(), "stl-solid-name"))
            .transpose()?
            .unwrap_or(0);
        exec.with_work(
            &self.solid_work,
            [1, name_bytes],
            |exec, work| -> ParseResult<()> {
                let name = match name_value {
                    Some(value) => Some(work.run([0, name_bytes], || -> ParseResult<_> {
                        let mut owned = exec.metered_string(
                            ImportStorageClass::SceneConstruction,
                            "stl-solid-name",
                        );
                        owned.try_push_str(exec, value)?;
                        Ok(owned)
                    })?),
                    None => None,
                };
                let first = required_token(exec, tokens, 0)?;
                let column_offset = first
                    .column
                    .checked_sub(1)
                    .map(u64::from)
                    .ok_or_else(|| exec.allocation_failure("stl-solid-byte-offset"))?;
                let token_offset = decoded_line
                    .raw_content_range()
                    .start()
                    .checked_add(column_offset)
                    .ok_or_else(|| exec.allocation_failure("stl-solid-byte-offset"))?;
                let token_end = token_offset
                    .checked_add(usize_amount(
                        exec,
                        first.text.len(),
                        "stl-solid-byte-range",
                    )?)
                    .ok_or_else(|| exec.allocation_failure("stl-solid-byte-range"))?;
                let start = diagnostic_range(exec, token_offset, token_end)?;
                let start_location = decoded_line.diagnostic_location(start).ok_or_else(|| {
                    exec.format_failure(StlFailure::content(
                        StlFailureCode::InternalInvariant,
                        "diagnostic-line-column",
                        "invalid-solid-start",
                    ))
                })?;
                self.current = Some(SolidBuilder::new(exec, name, start_location));
                self.state = State::InSolid;
                work.complete([1, 0])
            },
        )
    }

    fn finish_solid(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        name: Option<&str>,
        line_number: u32,
        column: u32,
    ) -> ParseResult<()> {
        let Some(current) = self.current.take() else {
            return Err(self.failure(
                exec,
                "endsolid",
                "without-active-solid",
                line_number,
                column,
            ));
        };
        let start_name = current.name.as_ref().map_or("", MeteredString::as_str);
        if !solid_names_match(exec, start_name, name.unwrap_or(""))? {
            return Err(self.failure(exec, "endsolid", "name-mismatch", line_number, column));
        }

        if current.face_count == 0 {
            exec.emit_text_diagnostic_with(
                DiagnosticSeverity::Warning,
                DiagnosticCode::new("stl.empty_solid")
                    .expect("static STL diagnostic code must be valid"),
                ImportProgressPhase::Decode,
                current.start_location,
                None,
                |details| details.unsigned("facet_count", 0),
                format_args!("ASCII STL solid contains no facets"),
            )?;
        } else {
            exec.with_work(&self.mesh_work, [1], |exec, work| -> ParseResult<()> {
                work.run_all(|| {
                    self.parsed.meshes.try_push(
                        exec,
                        ParsedMesh {
                            name: current.name,
                            positions: current.positions,
                            normals: current.normals,
                            colors: None,
                        },
                    )
                })
            })?;
        }

        self.state = State::Start;
        Ok(())
    }

    fn push_facet(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        normal: Vec3,
        vertices: &[Vec3; 3],
    ) -> ParseResult<()> {
        let positions = vertices.map(vec3_components);
        let normals = [vec3_components(normal); 3];
        exec.with_work(
            &self.facet_work,
            [1, 3, 6],
            |exec, work| -> ParseResult<()> {
                let Some(current) = self.current.as_mut() else {
                    return Err(exec.format_failure(StlFailure::content(
                        StlFailureCode::InternalInvariant,
                        "facet",
                        "outside-solid",
                    )));
                };
                work.run([0, 3, 3], || {
                    current.positions.try_extend_from_slice(exec, &positions)
                })?;
                #[cfg(test)]
                crate::control_tests::facet_completed(
                    crate::control_tests::CompletionSite::AsciiFacet,
                    current.face_count,
                    current.positions.as_slice(),
                    current.normals.as_slice(),
                );
                work.run([0, 0, 3], || {
                    current.normals.try_extend_from_slice(exec, &normals)
                })?;
                #[cfg(test)]
                crate::control_tests::facet_completed(
                    crate::control_tests::CompletionSite::AsciiFacet,
                    current.face_count,
                    current.positions.as_slice(),
                    current.normals.as_slice(),
                );
                work.run([1, 0, 0], || -> ParseResult<()> {
                    current.face_count = current
                        .face_count
                        .checked_add(1)
                        .ok_or_else(|| exec.allocation_failure("stl-face-counter"))?;
                    Ok(())
                })?;
                #[cfg(test)]
                crate::control_tests::facet_completed(
                    crate::control_tests::CompletionSite::AsciiFacet,
                    current.face_count,
                    current.positions.as_slice(),
                    current.normals.as_slice(),
                );
                Ok(())
            },
        )
    }

    fn finish_file(&self, exec: &mut ParseExecution<'_, '_>) -> ParseResult<()> {
        let reason = match &self.state {
            State::Start => return Ok(()),
            State::InSolid => "ended-before-endsolid",
            State::NeedOuterLoop { .. } => "ended-before-outer-loop",
            State::InLoop { .. } => "ended-inside-facet-loop",
            State::NeedEndFacet { .. } => "ended-before-endfacet",
        };
        Err(exec.format_failure(StlFailure::content(
            StlFailureCode::InvalidStructure,
            "ascii-document",
            reason,
        )))
    }

    fn failure(
        &self,
        exec: &mut ParseExecution<'_, '_>,
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> ParseFailure {
        exec.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            subject,
            reason,
            line,
            column,
        ))
    }
}

fn account_line_bytes<'region>(
    exec: &mut ParseExecution<'region, '_>,
    group: &ImportWorkGroup<'region, 1>,
    line: &str,
) -> ParseResult<()> {
    let amount = usize_amount(exec, line.len(), "stl-line-bytes")?;
    exec.with_work(group, [amount], |exec, work| -> ParseResult<()> {
        for _ in (CHECKPOINT_CHUNK_BYTES..line.len()).step_by(CHECKPOINT_CHUNK_BYTES) {
            exec.poll()?;
        }
        work.complete_all()
    })
}

struct SolidBuilder<'session> {
    name: Option<MeteredString<'session>>,
    positions: MeteredVec<'session, [f32; 3]>,
    normals: MeteredVec<'session, [f32; 3]>,
    face_count: usize,
    start_location: TextDiagnosticLocation<'session>,
}

const fn vec3_components(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

impl<'session> SolidBuilder<'session> {
    fn new(
        exec: &mut ParseExecution<'_, 'session>,
        name: Option<MeteredString<'session>>,
        start_location: TextDiagnosticLocation<'session>,
    ) -> Self {
        Self {
            name,
            positions: exec
                .metered_vec(ImportStorageClass::SceneConstruction, "stl-ascii-positions"),
            normals: exec.metered_vec(ImportStorageClass::SceneConstruction, "stl-ascii-normals"),
            face_count: 0,
            start_location,
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

struct TokenBuffer<'session> {
    values: MeteredVec<'session, TokenSpan>,
}

impl<'session> TokenBuffer<'session> {
    fn new(exec: &mut ParseExecution<'_, 'session>) -> Self {
        Self {
            values: exec.metered_vec(ImportStorageClass::FormatNormalization, "stl-tokens"),
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

fn tokenize<'line, 'buffer, 'region, 'session>(
    exec: &mut ParseExecution<'region, 'session>,
    group: &ImportWorkGroup<'region, 2>,
    line: &'line str,
    tokens: &'buffer mut TokenBuffer<'session>,
) -> ParseResult<TokenLine<'line, 'buffer>> {
    tokens.values.truncate(0);
    let mut start = None;
    let mut next_poll = 4096usize;

    for (index, byte) in line.bytes().enumerate() {
        if index >= next_poll {
            exec.poll()?;
            next_poll = next_poll.saturating_add(4096);
        }
        if byte.is_ascii_whitespace() {
            if let Some(token_start) = start.take() {
                push_token(exec, group, tokens, line, token_start, index)?;
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(token_start) = start {
        push_token(exec, group, tokens, line, token_start, line.len())?;
    }
    Ok(TokenLine {
        line,
        spans: tokens.values.as_slice(),
    })
}

fn push_token<'region, 'session>(
    exec: &mut ParseExecution<'region, 'session>,
    group: &ImportWorkGroup<'region, 2>,
    tokens: &mut TokenBuffer<'session>,
    line: &str,
    start: usize,
    end: usize,
) -> ParseResult<()> {
    let text = &line[start..end];
    let token_bytes = usize_amount(exec, text.len(), "stl-token-bytes")?;
    exec.with_work(group, [1, token_bytes], |exec, work| -> ParseResult<()> {
        let column = u32::try_from(start)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| exec.allocation_failure("stl-token-column"))?;
        work.run_all(|| {
            tokens
                .values
                .try_push(exec, TokenSpan { start, end, column })
        })
    })
}

fn parse_facet_normal(
    exec: &mut ParseExecution<'_, '_>,
    tokens: &TokenLine<'_, '_>,
    line_number: u32,
) -> ParseResult<Vec3> {
    if tokens.len() != 5 || !tokens_prefix_matches(tokens, &["facet", "normal"]) {
        return Err(exec.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            "facet-normal",
            "expected-three-components",
            line_number,
            tokens.first().map_or(1, |token| token.column),
        )));
    }
    parse_vec3_components(exec, tokens, 2, line_number)
}

fn parse_vec3(
    exec: &mut ParseExecution<'_, '_>,
    tokens: &TokenLine<'_, '_>,
    keyword: &'static str,
    line_number: u32,
) -> ParseResult<Vec3> {
    if tokens.len() != 4
        || !tokens
            .get(0)
            .is_some_and(|token| keyword_is(token, keyword))
    {
        return Err(exec.format_failure(StlFailure::located(
            StlFailureCode::InvalidStructure,
            keyword,
            "expected-three-components",
            line_number,
            tokens.first().map_or(1, |token| token.column),
        )));
    }
    parse_vec3_components(exec, tokens, 1, line_number)
}

fn parse_vec3_components(
    exec: &mut ParseExecution<'_, '_>,
    tokens: &TokenLine<'_, '_>,
    start: usize,
    line_number: u32,
) -> ParseResult<Vec3> {
    let x = required_token(exec, tokens, start)?;
    let y = required_token(exec, tokens, start + 1)?;
    let z = required_token(exec, tokens, start + 2)?;
    Ok(Vec3::new(
        parse_f32(exec, x, line_number)?,
        parse_f32(exec, y, line_number)?,
        parse_f32(exec, z, line_number)?,
    ))
}

fn required_token<'line>(
    exec: &mut ParseExecution<'_, '_>,
    tokens: &TokenLine<'line, '_>,
    index: usize,
) -> ParseResult<Token<'line>> {
    tokens.get(index).ok_or_else(|| {
        exec.format_failure(StlFailure::content(
            StlFailureCode::InternalInvariant,
            "ascii-token-range",
            "invalid-token-span",
        ))
    })
}

fn parse_f32(
    exec: &mut ParseExecution<'_, '_>,
    token: Token<'_>,
    line_number: u32,
) -> ParseResult<f32> {
    let value = token
        .text
        .parse::<f32>()
        .map_err(|_| exec.format_failure(StlFailure::invalid_float(line_number, token.column)))?;
    if !value.is_finite() {
        return Err(exec.format_failure(StlFailure::located(
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
    exec: &ParseExecution<'_, '_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| exec.allocation_failure(resource))
}

fn diagnostic_range(
    exec: &mut ParseExecution<'_, '_>,
    start: u64,
    end: u64,
) -> ParseResult<DiagnosticByteRange> {
    DiagnosticByteRange::new(start, end).map_err(|_| {
        exec.format_failure(StlFailure::content(
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
    // Conformed text is ASCII, so token bounds also trim surrounding whitespace.
    let start = tokens.spans.get(1)?.start;
    let end = tokens.spans.last()?.end;
    line.get(start..end)
}

fn solid_names_match(
    exec: &mut ParseExecution<'_, '_>,
    start: &str,
    end: &str,
) -> ParseResult<bool> {
    if start.len() != end.len() {
        return Ok(false);
    }
    for (start, end) in start
        .as_bytes()
        .chunks(CHECKPOINT_CHUNK_BYTES)
        .zip(end.as_bytes().chunks(CHECKPOINT_CHUNK_BYTES))
    {
        exec.poll()?;
        if start != end {
            return Ok(false);
        }
    }
    Ok(true)
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
