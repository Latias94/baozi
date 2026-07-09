use crate::parser::{ParsedMesh, ParsedStl};
use baozi_core::{
    BaoziError, Diagnostic, DiagnosticCode, DiagnosticSeverity, Result, SourceLocation, Vec3,
};
use baozi_import::ImportContext;

pub(crate) fn parse(ctx: &mut ImportContext<'_>, bytes: &[u8]) -> Result<ParsedStl> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        BaoziError::parse(
            ctx.source().to_string(),
            Some(SourceLocation::byte(error.valid_up_to() as u64)),
            "ASCII STL is not valid UTF-8",
        )
    })?;

    let parser = AsciiParser::new(ctx);
    parser.parse(text)
}

struct AsciiParser<'ctx, 'import> {
    ctx: &'ctx mut ImportContext<'import>,
    parsed: ParsedStl,
    state: State,
    current: Option<SolidBuilder>,
    total_faces: usize,
    total_vertices: usize,
    solid_count: usize,
}

impl<'ctx, 'import> AsciiParser<'ctx, 'import> {
    fn new(ctx: &'ctx mut ImportContext<'import>) -> Self {
        Self {
            ctx,
            parsed: ParsedStl {
                storage: "ascii",
                material_color: None,
                meshes: Vec::new(),
            },
            state: State::Start,
            current: None,
            total_faces: 0,
            total_vertices: 0,
            solid_count: 0,
        }
    }

    fn parse(mut self, text: &str) -> Result<ParsedStl> {
        for (line_index, raw_line) in text.lines().enumerate() {
            let line_number = (line_index + 1) as u32;
            let line = if line_index == 0 {
                raw_line.strip_prefix('\u{feff}').unwrap_or(raw_line)
            } else {
                raw_line
            };
            self.parse_line(line, line_number)?;
        }
        self.finish_file()?;
        Ok(self.parsed)
    }

    fn parse_line(&mut self, line: &str, line_number: u32) -> Result<()> {
        if line.len() > self.ctx.limits().max_line_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_line_bytes",
            });
        }

        let tokens = tokenize(self.ctx, line, line_number)?;
        if tokens.is_empty() {
            return Ok(());
        }

        let state = std::mem::replace(&mut self.state, State::Start);
        match state {
            State::Start => self.parse_start_line(line, &tokens, line_number),
            State::InSolid => self.parse_solid_line(&tokens, line_number),
            State::NeedOuterLoop { normal } => {
                if tokens_match(&tokens, &["outer", "loop"]) {
                    self.state = State::InLoop {
                        normal,
                        vertices: Vec::new(),
                    };
                    Ok(())
                } else {
                    Err(self.parse_error(
                        line_number,
                        tokens[0].column,
                        "expected outer loop after facet normal",
                    ))
                }
            }
            State::InLoop {
                normal,
                mut vertices,
            } => {
                if keyword_is(&tokens[0], "vertex") {
                    if vertices.len() == 3 {
                        return Err(self.parse_error(
                            line_number,
                            tokens[0].column,
                            "ASCII STL facet has more than three vertices",
                        ));
                    }
                    let vertex = parse_vec3(self.ctx, &tokens, "vertex", line_number)?;
                    vertices.push(vertex);
                    self.state = State::InLoop { normal, vertices };
                    Ok(())
                } else if tokens_match(&tokens, &["endloop"]) {
                    if vertices.len() != 3 {
                        return Err(self.parse_error(
                            line_number,
                            tokens[0].column,
                            "ASCII STL facet ended before three vertices",
                        ));
                    }
                    self.state = State::NeedEndFacet { normal, vertices };
                    Ok(())
                } else {
                    Err(self.parse_error(
                        line_number,
                        tokens[0].column,
                        "expected vertex or endloop inside ASCII STL facet",
                    ))
                }
            }
            State::NeedEndFacet { normal, vertices } => {
                if tokens_match(&tokens, &["endfacet"]) {
                    self.push_facet(normal, &vertices)?;
                    self.state = State::InSolid;
                    Ok(())
                } else {
                    Err(self.parse_error(
                        line_number,
                        tokens[0].column,
                        "expected endfacet after endloop",
                    ))
                }
            }
        }
    }

    fn parse_start_line(
        &mut self,
        line: &str,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> Result<()> {
        if !keyword_is(&tokens[0], "solid") {
            return Err(self.parse_error(
                line_number,
                tokens[0].column,
                "expected solid at start of ASCII STL",
            ));
        }
        self.start_solid(line, tokens, line_number)
    }

    fn parse_solid_line(&mut self, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        if keyword_is(&tokens[0], "facet") {
            let normal = parse_facet_normal(self.ctx, tokens, line_number)?;
            self.state = State::NeedOuterLoop { normal };
            Ok(())
        } else if keyword_is(&tokens[0], "endsolid") {
            if tokens.len() > 2 {
                return Err(self.parse_error(
                    line_number,
                    tokens[2].column,
                    "endsolid accepts at most one trailing name token",
                ));
            }
            self.finish_solid(line_number, tokens[0].column)
        } else if keyword_is(&tokens[0], "solid") {
            Err(self.parse_error(
                line_number,
                tokens[0].column,
                "nested ASCII STL solid blocks are not supported",
            ))
        } else {
            Err(self.parse_error(
                line_number,
                tokens[0].column,
                "expected facet normal or endsolid inside ASCII STL solid",
            ))
        }
    }

    fn start_solid(&mut self, line: &str, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        if tokens.is_empty() {
            return Err(self.parse_error(line_number, 1, "expected solid token"));
        }
        self.solid_count = self
            .solid_count
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_solids",
            })?;
        if self.solid_count > self.ctx.limits().max_solids {
            return Err(BaoziError::LimitExceeded {
                limit: "max_solids",
            });
        }

        let name = trailing_text_after_first_token(line, tokens)
            .map(|name| checked_name(self.ctx, name))
            .transpose()?;
        self.current = Some(SolidBuilder::new(
            name,
            location(line_number, tokens[0].column),
        ));
        self.state = State::InSolid;
        Ok(())
    }

    fn finish_solid(&mut self, line_number: u32, column: u32) -> Result<()> {
        let Some(current) = self.current.take() else {
            return Err(self.parse_error(line_number, column, "endsolid without active solid"));
        };

        if current.face_count == 0 {
            self.ctx.push_diagnostic(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: DiagnosticCode("stl.empty_solid"),
                source: Some(self.ctx.source().to_string()),
                location: Some(current.start),
                message: "ASCII STL solid contains no facets".to_owned(),
            });
        } else {
            let next_meshes =
                self.parsed
                    .meshes
                    .len()
                    .checked_add(1)
                    .ok_or(BaoziError::LimitExceeded {
                        limit: "max_meshes",
                    })?;
            if next_meshes > self.ctx.limits().max_meshes {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_meshes",
                });
            }
            self.parsed.meshes.push(ParsedMesh {
                name: current.name,
                positions: current.positions,
                normals: current.normals,
                colors: None,
            });
        }

        self.state = State::Start;
        Ok(())
    }

    fn push_facet(&mut self, normal: Vec3, vertices: &[Vec3]) -> Result<()> {
        let next_faces = self
            .total_faces
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
        if next_faces > self.ctx.limits().max_faces {
            return Err(BaoziError::LimitExceeded { limit: "max_faces" });
        }

        let next_vertices =
            self.total_vertices
                .checked_add(3)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_vertices",
                })?;
        if next_vertices > self.ctx.limits().max_vertices {
            return Err(BaoziError::LimitExceeded {
                limit: "max_vertices",
            });
        }

        let Some(current) = self.current.as_mut() else {
            return Err(BaoziError::parse(
                self.ctx.source().to_string(),
                None,
                "facet encountered outside ASCII STL solid",
            ));
        };
        current.positions.extend_from_slice(vertices);
        current.normals.extend([normal, normal, normal]);
        current.face_count += 1;
        self.total_faces = next_faces;
        self.total_vertices = next_vertices;
        Ok(())
    }

    fn finish_file(&self) -> Result<()> {
        match &self.state {
            State::Start => Ok(()),
            State::InSolid => Err(BaoziError::parse(
                self.ctx.source().to_string(),
                None,
                "ASCII STL ended before endsolid",
            )),
            State::NeedOuterLoop { .. } => Err(BaoziError::parse(
                self.ctx.source().to_string(),
                None,
                "ASCII STL ended before outer loop",
            )),
            State::InLoop { .. } => Err(BaoziError::parse(
                self.ctx.source().to_string(),
                None,
                "ASCII STL ended inside facet vertices",
            )),
            State::NeedEndFacet { .. } => Err(BaoziError::parse(
                self.ctx.source().to_string(),
                None,
                "ASCII STL ended before endfacet",
            )),
        }
    }

    fn parse_error(&self, line: u32, column: u32, message: impl Into<String>) -> BaoziError {
        BaoziError::parse(
            self.ctx.source().to_string(),
            Some(location(line, column)),
            message,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SolidBuilder {
    name: Option<String>,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    face_count: usize,
    start: SourceLocation,
}

impl SolidBuilder {
    fn new(name: Option<String>, start: SourceLocation) -> Self {
        Self {
            name,
            positions: Vec::new(),
            normals: Vec::new(),
            face_count: 0,
            start,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Start,
    InSolid,
    NeedOuterLoop { normal: Vec3 },
    InLoop { normal: Vec3, vertices: Vec<Vec3> },
    NeedEndFacet { normal: Vec3, vertices: Vec<Vec3> },
}

#[derive(Debug, Clone, Copy)]
struct Token<'line> {
    text: &'line str,
    column: u32,
}

fn tokenize<'line>(
    ctx: &ImportContext<'_>,
    line: &'line str,
    line_number: u32,
) -> Result<Vec<Token<'line>>> {
    let mut tokens = Vec::new();
    let mut start = None;

    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                push_token(ctx, &mut tokens, line, token_start, index, line_number)?;
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }

    if let Some(token_start) = start {
        push_token(ctx, &mut tokens, line, token_start, line.len(), line_number)?;
    }

    Ok(tokens)
}

fn push_token<'line>(
    ctx: &ImportContext<'_>,
    tokens: &mut Vec<Token<'line>>,
    line: &'line str,
    start: usize,
    end: usize,
    _line_number: u32,
) -> Result<()> {
    let text = &line[start..end];
    if text.len() > ctx.limits().max_token_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_token_bytes",
        });
    }
    tokens.push(Token {
        text,
        column: (start + 1) as u32,
    });
    Ok(())
}

fn parse_facet_normal(
    ctx: &ImportContext<'_>,
    tokens: &[Token<'_>],
    line_number: u32,
) -> Result<Vec3> {
    if tokens.len() != 5 || !tokens_match(&tokens[..2], &["facet", "normal"]) {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            tokens
                .first()
                .map(|token| location(line_number, token.column)),
            "expected facet normal with three components",
        ));
    }
    parse_vec3_components(ctx, tokens, 2, line_number)
}

fn parse_vec3(
    ctx: &ImportContext<'_>,
    tokens: &[Token<'_>],
    keyword: &str,
    line_number: u32,
) -> Result<Vec3> {
    if tokens.len() != 4 || !keyword_is(&tokens[0], keyword) {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            tokens
                .first()
                .map(|token| location(line_number, token.column)),
            format!("expected {keyword} with three components"),
        ));
    }
    parse_vec3_components(ctx, tokens, 1, line_number)
}

fn parse_vec3_components(
    ctx: &ImportContext<'_>,
    tokens: &[Token<'_>],
    start: usize,
    line_number: u32,
) -> Result<Vec3> {
    Ok(Vec3::new(
        parse_f32(ctx, tokens[start], line_number)?,
        parse_f32(ctx, tokens[start + 1], line_number)?,
        parse_f32(ctx, tokens[start + 2], line_number)?,
    ))
}

fn parse_f32(ctx: &ImportContext<'_>, token: Token<'_>, line_number: u32) -> Result<f32> {
    let value = token.text.parse::<f32>().map_err(|_| {
        BaoziError::parse(
            ctx.source().to_string(),
            Some(location(line_number, token.column)),
            format!("invalid ASCII STL float '{}'", token.text),
        )
    })?;
    if !value.is_finite() {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            Some(location(line_number, token.column)),
            "ASCII STL float is non-finite",
        ));
    }
    Ok(value)
}

fn checked_name(ctx: &ImportContext<'_>, name: &str) -> Result<String> {
    if name.len() > ctx.limits().max_string_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        });
    }
    Ok(name.to_owned())
}

fn trailing_text_after_first_token<'line>(
    line: &'line str,
    tokens: &[Token<'line>],
) -> Option<&'line str> {
    let first = tokens.first()?;
    let start = (first.column as usize - 1).checked_add(first.text.len())?;
    let text = line.get(start..)?.trim();
    (!text.is_empty()).then_some(text)
}

fn tokens_match(tokens: &[Token<'_>], expected: &[&str]) -> bool {
    tokens.len() == expected.len()
        && tokens
            .iter()
            .zip(expected)
            .all(|(token, expected)| keyword_is(token, expected))
}

fn keyword_is(token: &Token<'_>, expected: &str) -> bool {
    token.text.eq_ignore_ascii_case(expected)
}

fn location(line: u32, column: u32) -> SourceLocation {
    SourceLocation::line_column(line, column)
}
