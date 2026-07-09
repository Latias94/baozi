use crate::mtl;
use baozi_core::{BaoziError, Result, SourceLocation, Vec2, Vec3};
use baozi_import::ImportContext;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedObj {
    pub positions: Vec<Vec3>,
    pub texcoords: Vec<Vec2>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub mtllibs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Face {
    pub vertices: Vec<FaceVertex>,
    pub object: Option<String>,
    pub group: Option<String>,
    pub material: Option<String>,
    pub smoothing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FaceVertex {
    pub position: usize,
    pub texcoord: Option<usize>,
    pub normal: Option<usize>,
}

pub(crate) fn parse(ctx: &mut ImportContext<'_>, text: &str) -> Result<ParsedObj> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = ObjParser {
        ctx,
        parsed: ParsedObj {
            positions: Vec::new(),
            texcoords: Vec::new(),
            normals: Vec::new(),
            faces: Vec::new(),
            mtllibs: Vec::new(),
        },
        object: None,
        group: None,
        material: None,
        smoothing: None,
    };

    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = (line_index + 1) as u32;
        parser.parse_line(raw_line, line_number)?;
    }

    Ok(parser.parsed)
}

struct ObjParser<'ctx, 'import> {
    ctx: &'ctx mut ImportContext<'import>,
    parsed: ParsedObj,
    object: Option<String>,
    group: Option<String>,
    material: Option<String>,
    smoothing: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Token<'line> {
    text: &'line str,
    column: u32,
}

impl ObjParser<'_, '_> {
    fn parse_line(&mut self, raw_line: &str, line_number: u32) -> Result<()> {
        if raw_line.len() > self.ctx.options.limits.max_line_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_line_bytes",
            });
        }

        let line = raw_line.split_once('#').map_or(raw_line, |(head, _)| head);
        let tokens = tokenize(self.ctx, line, line_number)?;
        let Some(first) = tokens.first() else {
            return Ok(());
        };

        match first.text {
            "v" => self.parse_position(&tokens, line_number),
            "vt" => self.parse_texcoord(&tokens, line_number),
            "vn" => self.parse_normal(&tokens, line_number),
            "f" => self.parse_face(&tokens, line_number),
            "o" => {
                self.object = trailing_text_after_first_token(line, &tokens)
                    .map(|name| checked_string(self.ctx, name))
                    .transpose()?;
                Ok(())
            }
            "g" => {
                self.group = trailing_text_after_first_token(line, &tokens)
                    .map(|name| checked_string(self.ctx, name))
                    .transpose()?;
                Ok(())
            }
            "usemtl" => {
                self.material = trailing_text_after_first_token(line, &tokens)
                    .map(|name| checked_string(self.ctx, name))
                    .transpose()?;
                Ok(())
            }
            "s" => {
                self.smoothing = trailing_text_after_first_token(line, &tokens)
                    .map(|name| checked_string(self.ctx, name))
                    .transpose()?;
                Ok(())
            }
            "mtllib" => {
                if let Some(path) = trailing_text_after_first_token(line, &tokens) {
                    self.parsed.mtllibs.push(checked_string(self.ctx, path)?);
                }
                Ok(())
            }
            keyword if is_unsupported_statement(keyword) => {
                let source = self.ctx.source.to_string();
                mtl::push_warning(
                    self.ctx,
                    source,
                    Some(location(line_number, first.column)),
                    "obj.unsupported_statement",
                    format!("OBJ statement '{keyword}' is not supported yet"),
                );
                Ok(())
            }
            keyword => {
                let source = self.ctx.source.to_string();
                mtl::push_warning(
                    self.ctx,
                    source,
                    Some(location(line_number, first.column)),
                    "obj.unknown_statement",
                    format!("OBJ statement '{keyword}' is not recognized"),
                );
                Ok(())
            }
        }
    }

    fn parse_position(&mut self, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        require_components(self.ctx, tokens, 4, "OBJ position", line_number)?;
        let next = self
            .parsed
            .positions
            .len()
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_vertices",
            })?;
        if next > self.ctx.options.limits.max_vertices {
            return Err(BaoziError::LimitExceeded {
                limit: "max_vertices",
            });
        }
        self.parsed.positions.push(Vec3::new(
            parse_f32(self.ctx, tokens[1], line_number)?,
            parse_f32(self.ctx, tokens[2], line_number)?,
            parse_f32(self.ctx, tokens[3], line_number)?,
        ));
        Ok(())
    }

    fn parse_texcoord(&mut self, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        require_components(self.ctx, tokens, 3, "OBJ texcoord", line_number)?;
        enforce_stream_limit(self.ctx, self.parsed.texcoords.len())?;
        self.parsed.texcoords.push(Vec2::new(
            parse_f32(self.ctx, tokens[1], line_number)?,
            parse_f32(self.ctx, tokens[2], line_number)?,
        ));
        Ok(())
    }

    fn parse_normal(&mut self, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        require_components(self.ctx, tokens, 4, "OBJ normal", line_number)?;
        enforce_stream_limit(self.ctx, self.parsed.normals.len())?;
        self.parsed.normals.push(Vec3::new(
            parse_f32(self.ctx, tokens[1], line_number)?,
            parse_f32(self.ctx, tokens[2], line_number)?,
            parse_f32(self.ctx, tokens[3], line_number)?,
        ));
        Ok(())
    }

    fn parse_face(&mut self, tokens: &[Token<'_>], line_number: u32) -> Result<()> {
        if tokens.len() < 4 {
            return Err(parse_error(
                self.ctx,
                tokens
                    .first()
                    .map(|token| location(line_number, token.column)),
                "OBJ face must have at least three vertices",
            ));
        }

        let next = self
            .parsed
            .faces
            .len()
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
        if next > self.ctx.options.limits.max_faces {
            return Err(BaoziError::LimitExceeded { limit: "max_faces" });
        }

        let mut vertices = Vec::with_capacity(tokens.len() - 1);
        for token in &tokens[1..] {
            vertices.push(parse_face_vertex(
                self.ctx,
                *token,
                &self.parsed,
                line_number,
            )?);
        }
        self.parsed.faces.push(Face {
            vertices,
            object: self.object.clone(),
            group: self.group.clone(),
            material: self.material.clone(),
            smoothing: self.smoothing.clone(),
        });
        Ok(())
    }
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
    line_number: u32,
) -> Result<()> {
    let text = &line[start..end];
    if text.len() > ctx.options.limits.max_token_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_token_bytes",
        });
    }
    if text.contains('\0') {
        return Err(parse_error(
            ctx,
            Some(location(line_number, (start + 1) as u32)),
            "OBJ token contains NUL",
        ));
    }
    tokens.push(Token {
        text,
        column: (start + 1) as u32,
    });
    Ok(())
}

fn parse_face_vertex(
    ctx: &ImportContext<'_>,
    token: Token<'_>,
    parsed: &ParsedObj,
    line_number: u32,
) -> Result<FaceVertex> {
    let mut parts = token.text.split('/');
    let position = parts.next().unwrap_or_default();
    let texcoord = parts.next();
    let normal = parts.next();
    if parts.next().is_some() || position.is_empty() {
        return Err(parse_error(
            ctx,
            Some(location(line_number, token.column)),
            format!("invalid OBJ face vertex '{}'", token.text),
        ));
    }

    Ok(FaceVertex {
        position: resolve_index(
            ctx,
            position,
            parsed.positions.len(),
            "position",
            line_number,
            token.column,
        )?,
        texcoord: match texcoord {
            Some("") | None => None,
            Some(value) => Some(resolve_index(
                ctx,
                value,
                parsed.texcoords.len(),
                "texcoord",
                line_number,
                token.column,
            )?),
        },
        normal: match normal {
            Some("") | None => None,
            Some(value) => Some(resolve_index(
                ctx,
                value,
                parsed.normals.len(),
                "normal",
                line_number,
                token.column,
            )?),
        },
    })
}

fn resolve_index(
    ctx: &ImportContext<'_>,
    text: &str,
    len: usize,
    kind: &str,
    line_number: u32,
    column: u32,
) -> Result<usize> {
    let value = text.parse::<i64>().map_err(|_| {
        parse_error(
            ctx,
            Some(location(line_number, column)),
            format!("invalid OBJ {kind} index '{text}'"),
        )
    })?;
    if value == 0 {
        return Err(parse_error(
            ctx,
            Some(location(line_number, column)),
            format!("OBJ {kind} index 0 is invalid"),
        ));
    }

    let resolved = if value > 0 {
        value - 1
    } else {
        len as i64 + value
    };
    if resolved < 0 || resolved >= len as i64 {
        return Err(parse_error(
            ctx,
            Some(location(line_number, column)),
            format!("OBJ {kind} index {value} is out of range"),
        ));
    }
    Ok(resolved as usize)
}

fn require_components(
    ctx: &ImportContext<'_>,
    tokens: &[Token<'_>],
    minimum: usize,
    label: &str,
    line_number: u32,
) -> Result<()> {
    if tokens.len() < minimum {
        return Err(parse_error(
            ctx,
            tokens
                .first()
                .map(|token| location(line_number, token.column)),
            format!("{label} requires {} components", minimum - 1),
        ));
    }
    Ok(())
}

fn enforce_stream_limit(ctx: &ImportContext<'_>, current_len: usize) -> Result<()> {
    let next = current_len
        .checked_add(1)
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_vertices",
        })?;
    if next > ctx.options.limits.max_vertices {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }
    Ok(())
}

fn parse_f32(ctx: &ImportContext<'_>, token: Token<'_>, line_number: u32) -> Result<f32> {
    let value = token.text.parse::<f32>().map_err(|_| {
        parse_error(
            ctx,
            Some(location(line_number, token.column)),
            format!("invalid OBJ float '{}'", token.text),
        )
    })?;
    if !value.is_finite() {
        return Err(parse_error(
            ctx,
            Some(location(line_number, token.column)),
            "OBJ float is non-finite",
        ));
    }
    Ok(value)
}

fn checked_string(ctx: &ImportContext<'_>, text: &str) -> Result<String> {
    let text = text.trim();
    if text.len() > ctx.options.limits.max_string_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        });
    }
    Ok(text.to_owned())
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

fn parse_error(
    ctx: &ImportContext<'_>,
    location: Option<SourceLocation>,
    message: impl Into<String>,
) -> BaoziError {
    BaoziError::parse(ctx.source.to_string(), location, message)
}

fn location(line: u32, column: u32) -> SourceLocation {
    SourceLocation::line_column(line, column)
}

fn is_unsupported_statement(keyword: &str) -> bool {
    matches!(
        keyword,
        "p" | "l"
            | "curv"
            | "curv2"
            | "surf"
            | "parm"
            | "trim"
            | "hole"
            | "scrv"
            | "sp"
            | "end"
            | "cstype"
            | "deg"
            | "bmat"
            | "step"
    )
}
