use crate::allocation::{LiveStorage, usize_to_u64 as usize_amount};
use crate::{ObjFailure, mtl, parser};
use baozi_core::{Vec2, Vec3};
use baozi_import::{ImportStorageClass, ImportWorkClass, ParseResult, ParseScope};

const MAX_NUMERIC_LITERAL_BYTES: usize = 128;
const MAX_FACE_REFERENCE_BYTES: usize = MAX_NUMERIC_LITERAL_BYTES * 3 + 2;

pub(crate) struct ParsedObj<'scope> {
    pub positions: Vec<Vec3>,
    pub texcoords: Vec<Vec2>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub mtllibs: Vec<String>,
    state_names: Vec<String>,
    storage: LiveStorage<'scope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct StateNameId(usize);

#[cfg(test)]
impl StateNameId {
    pub(crate) const fn from_index(index: usize) -> Self {
        Self(index)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Face {
    pub vertices: Vec<FaceVertex>,
    pub object: Option<StateNameId>,
    pub group: Option<StateNameId>,
    pub material: Option<StateNameId>,
    pub smoothing: Option<StateNameId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FaceVertex {
    pub position: usize,
    pub texcoord: Option<usize>,
    pub normal: Option<usize>,
}

impl ParsedObj<'_> {
    pub(crate) fn state_name(&self, id: StateNameId) -> Option<&str> {
        self.state_names.get(id.0).map(String::as_str)
    }
}

pub(crate) fn parse<'scope>(
    scope: &'scope ParseScope<'_>,
    text: &parser::DecodedText<'_>,
) -> ParseResult<ParsedObj<'scope>> {
    let mut parser = ObjParser {
        scope,
        parsed: ParsedObj {
            positions: Vec::new(),
            texcoords: Vec::new(),
            normals: Vec::new(),
            faces: Vec::new(),
            mtllibs: Vec::new(),
            state_names: Vec::new(),
            storage: LiveStorage::new(ImportStorageClass::FormatNormalization),
        },
        object: None,
        group: None,
        material: None,
        smoothing: None,
    };

    parser::scan_lines(scope, text, |line, line_number| {
        let record = scope.begin_work(ImportWorkClass::Records, 1)?;
        parser.parse_line(&line, line_number)?;
        record.complete()
    })?;

    Ok(parser.parsed)
}

struct ObjParser<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    parsed: ParsedObj<'scope>,
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
    smoothing: Option<StateNameId>,
}

#[derive(Debug, Clone, Copy)]
struct Token<'line> {
    text: &'line str,
    column: u32,
    start: usize,
    end: usize,
}

struct TokenBuffer<'scope, 'line> {
    values: Vec<Token<'line>>,
    storage: LiveStorage<'scope>,
}

impl<'scope> ObjParser<'scope, '_> {
    fn parse_line(
        &mut self,
        decoded_line: &parser::DecodedLine<'_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let raw_line = decoded_line.text();
        let tokens = tokenize(self.scope, decoded_line, line_number)?;
        let allowed_value = tokens.values.first().and_then(|first| match first.text {
            "o" | "g" | "usemtl" | "s" | "mtllib" => trailing_token_range(&tokens.values),
            _ => None,
        });
        parser::authorize_line_recoveries(
            self.scope,
            decoded_line,
            allowed_value,
            decoded_line.comment_start(),
            || self.scope.diagnostic_source(),
        )?;
        let Some(first) = tokens.values.first() else {
            return Ok(());
        };

        match first.text {
            "v" => self.parse_position(&tokens.values, line_number),
            "vt" => self.parse_texcoord(&tokens.values, line_number),
            "vn" => self.parse_normal(&tokens.values, line_number),
            "f" => self.parse_face(&tokens.values, line_number),
            "o" => {
                self.object = parse_state_name(
                    self.scope,
                    &mut self.parsed.storage,
                    &mut self.parsed.state_names,
                    self.object,
                    trailing_text_after_first_token(raw_line, &tokens.values),
                )?;
                Ok(())
            }
            "g" => {
                self.group = parse_state_name(
                    self.scope,
                    &mut self.parsed.storage,
                    &mut self.parsed.state_names,
                    self.group,
                    trailing_text_after_first_token(raw_line, &tokens.values),
                )?;
                Ok(())
            }
            "usemtl" => {
                self.material = parse_state_name(
                    self.scope,
                    &mut self.parsed.storage,
                    &mut self.parsed.state_names,
                    self.material,
                    trailing_text_after_first_token(raw_line, &tokens.values),
                )?;
                Ok(())
            }
            "s" => {
                self.smoothing = parse_state_name(
                    self.scope,
                    &mut self.parsed.storage,
                    &mut self.parsed.state_names,
                    self.smoothing,
                    trailing_text_after_first_token(raw_line, &tokens.values),
                )?;
                Ok(())
            }
            "mtllib" => {
                for token in &tokens.values[1..] {
                    let mtllib = checked_string(self.scope, &mut self.parsed.storage, token.text)?;
                    self.parsed.storage.reserve_vec(
                        self.scope,
                        &mut self.parsed.mtllibs,
                        1,
                        "obj-material-library-references",
                    )?;
                    self.parsed.mtllibs.push(mtllib);
                }
                Ok(())
            }
            keyword if is_unsupported_statement(keyword) => mtl::push_warning(
                self.scope,
                Some((line_number, first.column)),
                "obj.unsupported_statement",
                format_args!("OBJ statement '{keyword}' is not supported yet"),
            ),
            keyword => mtl::push_warning(
                self.scope,
                Some((line_number, first.column)),
                "obj.unknown_statement",
                format_args!("OBJ statement '{keyword}' is not recognized"),
            ),
        }
    }

    fn parse_position(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        require_components(self.scope, tokens, 4, "position", line_number)?;
        let vertex = self.scope.begin_work(ImportWorkClass::Vertices, 1)?;
        self.parsed.storage.reserve_vec(
            self.scope,
            &mut self.parsed.positions,
            1,
            "obj-positions",
        )?;
        self.parsed.positions.push(Vec3::new(
            parse_f32(self.scope, "position", tokens[1], line_number)?,
            parse_f32(self.scope, "position", tokens[2], line_number)?,
            parse_f32(self.scope, "position", tokens[3], line_number)?,
        ));
        vertex.complete()
    }

    fn parse_texcoord(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        require_components(self.scope, tokens, 3, "texcoord", line_number)?;
        let attribute = self.scope.begin_work(ImportWorkClass::AttributeCells, 1)?;
        self.parsed.storage.reserve_vec(
            self.scope,
            &mut self.parsed.texcoords,
            1,
            "obj-texcoords",
        )?;
        self.parsed.texcoords.push(Vec2::new(
            parse_f32(self.scope, "texcoord", tokens[1], line_number)?,
            parse_f32(self.scope, "texcoord", tokens[2], line_number)?,
        ));
        attribute.complete()
    }

    fn parse_normal(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        require_components(self.scope, tokens, 4, "normal", line_number)?;
        let attribute = self.scope.begin_work(ImportWorkClass::AttributeCells, 1)?;
        self.parsed
            .storage
            .reserve_vec(self.scope, &mut self.parsed.normals, 1, "obj-normals")?;
        self.parsed.normals.push(Vec3::new(
            parse_f32(self.scope, "normal", tokens[1], line_number)?,
            parse_f32(self.scope, "normal", tokens[2], line_number)?,
            parse_f32(self.scope, "normal", tokens[3], line_number)?,
        ));
        attribute.complete()
    }

    fn parse_face(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        if tokens.len() < 4 {
            let column = tokens.first().map_or(1, |token| token.column);
            return Err(self.scope.format_failure(ObjFailure::face(
                "too-few-vertices",
                line_number,
                column,
            )));
        }

        let face_permit = self.scope.begin_work(ImportWorkClass::Faces, 1)?;
        let face_vertex_count = tokens.len() - 1;
        let topology_permit = self.scope.begin_work(
            ImportWorkClass::TopologyCells,
            usize_amount(self.scope, face_vertex_count, "obj-face-topology-cells")?,
        )?;

        let mut vertices = Vec::new();
        self.parsed.storage.reserve_vec(
            self.scope,
            &mut vertices,
            face_vertex_count,
            "obj-face-vertices",
        )?;
        for chunk in tokens[1..].chunks(64) {
            self.scope.poll()?;
            for token in chunk {
                vertices.push(parse_face_vertex(
                    self.scope,
                    *token,
                    &self.parsed,
                    line_number,
                )?);
            }
        }
        self.parsed
            .storage
            .reserve_vec(self.scope, &mut self.parsed.faces, 1, "obj-faces")?;
        self.parsed.faces.push(Face {
            vertices,
            object: self.object,
            group: self.group,
            material: self.material,
            smoothing: self.smoothing,
        });
        topology_permit.complete()?;
        face_permit.complete()
    }
}

fn parse_state_name<'scope>(
    scope: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    state_names: &mut Vec<String>,
    current: Option<StateNameId>,
    text: Option<&str>,
) -> ParseResult<Option<StateNameId>> {
    let Some(text) = text.map(str::trim) else {
        return Ok(None);
    };
    parser::enforce_text_value_limit(scope, text.len(), "obj-state-name-bytes")?;
    let string_permit = scope.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, text.len(), "obj-state-name-bytes")?,
    )?;
    if let Some(current) = current {
        let current_name = state_names.get(current.0).ok_or_else(|| {
            scope.format_failure(ObjFailure::internal("state-name-id-out-of-range"))
        })?;
        let matches_current =
            mtl::interruptible_str_eq(scope, current_name, text, "obj-state-name-comparison")?;
        if matches_current {
            string_permit.complete()?;
            return Ok(Some(current));
        }
    }

    storage.reserve_vec(scope, state_names, 1, "obj-state-names")?;
    let name = storage.copy_string(scope, text, "obj-state-name")?;
    let id = StateNameId(state_names.len());
    state_names.push(name);
    string_permit.complete()?;
    Ok(Some(id))
}

fn tokenize<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    decoded_line: &'line parser::DecodedLine<'_>,
    line_number: u32,
) -> ParseResult<TokenBuffer<'scope, 'line>> {
    let line = decoded_line.text();
    let mut tokens = TokenBuffer {
        values: Vec::new(),
        storage: LiveStorage::new(ImportStorageClass::FormatNormalization),
    };

    parser::scan_token_ranges(
        scope,
        line,
        "obj-token-scan-work",
        |index, character| {
            if character == '\0' {
                let column = index
                    .checked_add(1)
                    .and_then(|column| u32::try_from(column).ok())
                    .ok_or_else(|| scope.resource_limit_exceeded("line-columns", u64::MAX))?;
                return Err(scope.format_failure(ObjFailure::statement(
                    "token",
                    "contains-nul",
                    line_number,
                    column,
                )));
            }
            Ok(())
        },
        |token_start, token_end| push_token(scope, &mut tokens, line, token_start, token_end),
    )?;
    Ok(tokens)
}

fn push_token<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    tokens: &mut TokenBuffer<'scope, 'line>,
    line: &'line str,
    start: usize,
    end: usize,
) -> ParseResult<()> {
    let text = &line[start..end];
    let token = scope.begin_work(ImportWorkClass::Tokens, 1)?;
    let token_bytes = scope.begin_work(
        ImportWorkClass::TokenBytes,
        usize_amount(scope, text.len(), "obj-token-bytes")?,
    )?;
    let column = start
        .checked_add(1)
        .and_then(|column| u32::try_from(column).ok())
        .ok_or_else(|| scope.resource_limit_exceeded("line-columns", u64::MAX))?;
    tokens
        .storage
        .reserve_vec(scope, &mut tokens.values, 1, "obj-tokens")?;
    tokens.values.push(Token {
        text,
        column,
        start,
        end,
    });
    token_bytes.complete()?;
    token.complete()
}

fn parse_face_vertex(
    scope: &ParseScope<'_>,
    token: Token<'_>,
    parsed: &ParsedObj<'_>,
    line_number: u32,
) -> ParseResult<FaceVertex> {
    scope.enforce_resource_limit(
        "obj-face-reference-bytes",
        usize_amount(scope, token.text.len(), "obj-face-reference-bytes")?,
        MAX_FACE_REFERENCE_BYTES as u64,
    )?;
    let mut parts = token.text.split('/');
    let position = parts.next().unwrap_or_default();
    let texcoord = parts.next();
    let normal = parts.next();
    if parts.next().is_some() || position.is_empty() {
        return Err(scope.format_failure(ObjFailure::face(
            "invalid-vertex-reference",
            line_number,
            token.column,
        )));
    }

    Ok(FaceVertex {
        position: resolve_index(
            scope,
            position,
            parsed.positions.len(),
            "position",
            line_number,
            token.column,
        )?,
        texcoord: match texcoord {
            Some("") | None => None,
            Some(value) => Some(resolve_index(
                scope,
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
                scope,
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
    scope: &ParseScope<'_>,
    text: &str,
    len: usize,
    kind: &'static str,
    line_number: u32,
    column: u32,
) -> ParseResult<usize> {
    enforce_numeric_literal(scope, text)?;
    let value = text.parse::<i64>().map_err(|error| {
        scope.format_failure(ObjFailure::invalid_index(
            kind,
            "invalid-integer",
            line_number,
            column,
            Some(error),
        ))
    })?;
    if value == 0 {
        return Err(scope.format_failure(ObjFailure::invalid_index(
            kind,
            "zero",
            line_number,
            column,
            None,
        )));
    }

    let resolved = if value > 0 {
        value - 1
    } else {
        len as i64 + value
    };
    if resolved < 0 || resolved >= len as i64 {
        return Err(scope.format_failure(ObjFailure::invalid_index(
            kind,
            "out-of-range",
            line_number,
            column,
            None,
        )));
    }
    Ok(resolved as usize)
}

fn require_components(
    scope: &ParseScope<'_>,
    tokens: &[Token<'_>],
    minimum: usize,
    subject: &'static str,
    line_number: u32,
) -> ParseResult<()> {
    if tokens.len() < minimum {
        let column = tokens.first().map_or(1, |token| token.column);
        return Err(scope.format_failure(ObjFailure::statement(
            subject,
            "missing-components",
            line_number,
            column,
        )));
    }
    Ok(())
}

fn parse_f32(
    scope: &ParseScope<'_>,
    subject: &'static str,
    token: Token<'_>,
    line_number: u32,
) -> ParseResult<f32> {
    enforce_numeric_literal(scope, token.text)?;
    let value = token.text.parse::<f32>().map_err(|error| {
        scope.format_failure(ObjFailure::invalid_float(
            subject,
            line_number,
            token.column,
            error,
        ))
    })?;
    if !value.is_finite() {
        return Err(scope.format_failure(ObjFailure::non_finite(
            subject,
            line_number,
            token.column,
        )));
    }
    Ok(value)
}

fn checked_string<'scope>(
    scope: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    text: &str,
) -> ParseResult<String> {
    parser::enforce_text_value_limit(scope, text.len(), "obj-material-library-reference")?;
    let string_permit = scope.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, text.len(), "obj-string-bytes")?,
    )?;
    let owned = storage.copy_string(scope, text, "obj-material-library-reference")?;
    string_permit.complete()?;
    Ok(owned)
}

fn enforce_numeric_literal(scope: &ParseScope<'_>, text: &str) -> ParseResult<()> {
    scope.enforce_resource_limit(
        "obj-number-bytes",
        usize_amount(scope, text.len(), "obj-number-bytes")?,
        MAX_NUMERIC_LITERAL_BYTES as u64,
    )
}

fn trailing_text_after_first_token<'line>(
    line: &'line str,
    tokens: &[Token<'line>],
) -> Option<&'line str> {
    let first_value = tokens.get(1)?;
    let last_value = tokens.last()?;
    line.get(first_value.start..last_value.end)
}

fn trailing_token_range(tokens: &[Token<'_>]) -> Option<std::ops::Range<usize>> {
    let first_value = tokens.get(1)?;
    let last_value = tokens.last()?;
    Some(first_value.start..last_value.end)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_obj_and_faces_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<Face>();
        assert_send_sync::<ParsedObj<'static>>();
    }
}
