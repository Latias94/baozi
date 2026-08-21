use crate::{ObjFailure, mtl, parser};
use baozi_import::authoring::{Vec2, Vec3};
use baozi_import::{
    ConformedTextInput, ImportStorageClass, ImportWorkClass, MeteredString, MeteredVec,
    ParseResult, ParseScope, TextInputLine, TextInputRecoveryTracker,
};
use std::num::NonZeroU32;

use parser::usize_amount;

const MAX_NUMERIC_LITERAL_BYTES: usize = 128;
const MAX_FACE_REFERENCE_BYTES: usize = MAX_NUMERIC_LITERAL_BYTES * 3 + 2;

pub(crate) struct ParsedObj<'scope> {
    pub positions: MeteredVec<'scope, Vec3>,
    pub texcoords: MeteredVec<'scope, Vec2>,
    pub normals: MeteredVec<'scope, Vec3>,
    pub faces: MeteredVec<'scope, Face<'scope>>,
    pub mtllibs: MeteredVec<'scope, MeteredString<'scope>>,
    state_names: MeteredVec<'scope, MeteredString<'scope>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct StateNameId(usize);

#[cfg(test)]
impl StateNameId {
    pub(crate) const fn from_index(index: usize) -> Self {
        Self(index)
    }
}

pub(crate) struct Face<'scope> {
    pub vertices: MeteredVec<'scope, FaceVertex>,
    pub object: Option<StateNameId>,
    pub group: Option<StateNameId>,
    pub material: Option<StateNameId>,
    pub smoothing: ObjSmoothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjSmoothing {
    Unspecified,
    Off,
    Group(NonZeroU32),
}

impl ObjSmoothing {
    pub(crate) const fn canonical_value(self) -> Option<u32> {
        match self {
            Self::Unspecified => None,
            Self::Off => Some(0),
            Self::Group(group) => Some(group.get()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FaceVertex {
    pub position: usize,
    pub texcoord: Option<usize>,
    pub normal: Option<usize>,
}

impl ParsedObj<'_> {
    pub(crate) fn state_name(&self, id: StateNameId) -> Option<&str> {
        self.state_names
            .as_slice()
            .get(id.0)
            .map(MeteredString::as_str)
    }
}

pub(crate) fn parse<'scope>(
    scope: &'scope ParseScope<'_>,
    text: &ConformedTextInput<'_>,
    recoveries: &mut TextInputRecoveryTracker<'_, '_>,
) -> ParseResult<ParsedObj<'scope>> {
    let mut parser = ObjParser {
        scope,
        parsed: ParsedObj {
            positions: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-positions"),
            texcoords: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-texcoords"),
            normals: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-normals"),
            faces: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-faces"),
            mtllibs: scope.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-material-library-references",
            ),
            state_names: scope
                .metered_vec(ImportStorageClass::FormatNormalization, "obj-state-names"),
        },
        object: None,
        group: None,
        material: None,
        smoothing: ObjSmoothing::Unspecified,
    };

    for line in text.lines() {
        let line_number = u32::try_from(line.number())
            .map_err(|_| scope.resource_limit_exceeded("line-count", u64::from(u32::MAX)))?;
        let line = scope.with_work(ImportWorkClass::Records, 1, |work| -> ParseResult<_> {
            work.complete_all()?;
            Ok(line)
        })?;
        parser.parse_line(recoveries, &line, line_number)?;
    }

    Ok(parser.parsed)
}

struct ObjParser<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    parsed: ParsedObj<'scope>,
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
    smoothing: ObjSmoothing,
}

#[derive(Debug, Clone, Copy)]
struct Token<'line> {
    text: &'line str,
    column: u32,
    start: usize,
    end: usize,
}

struct TokenBuffer<'scope, 'line> {
    values: MeteredVec<'scope, Token<'line>>,
    comment_start: Option<usize>,
}

impl<'scope> ObjParser<'scope, '_> {
    fn parse_line(
        &mut self,
        recoveries: &mut TextInputRecoveryTracker<'_, '_>,
        decoded_line: &TextInputLine<'_, '_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let raw_line = decoded_line.text();
        let tokens = tokenize(self.scope, decoded_line, line_number)?;
        let token_values = tokens.values.as_slice();
        let allowed_value = token_values.first().and_then(|first| match first.text {
            "o" | "g" | "usemtl" | "mtllib" => trailing_token_range(token_values),
            _ => None,
        });
        parser::authorize_line_recoveries(
            self.scope,
            recoveries,
            decoded_line,
            allowed_value,
            tokens.comment_start,
        )?;
        let Some(first) = token_values.first() else {
            return Ok(());
        };

        match first.text {
            "v" => self.parse_position(token_values, line_number),
            "vt" => self.parse_texcoord(token_values, line_number),
            "vn" => self.parse_normal(token_values, line_number),
            "f" => self.parse_face(token_values, line_number),
            "o" => {
                self.object = parse_state_name(
                    self.scope,
                    &mut self.parsed.state_names,
                    self.object,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "g" => {
                self.group = parse_state_name(
                    self.scope,
                    &mut self.parsed.state_names,
                    self.group,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "usemtl" => {
                self.material = parse_state_name(
                    self.scope,
                    &mut self.parsed.state_names,
                    self.material,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "s" => {
                self.smoothing = parse_smoothing_group(self.scope, token_values, line_number)?;
                Ok(())
            }
            "mtllib" => self.parse_material_library_references(&token_values[1..]),
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
        self.scope
            .with_work(ImportWorkClass::Vertices, 1, |work| -> ParseResult<()> {
                work.run_all(|| {
                    self.parsed.positions.try_push(Vec3::new(
                        parse_f32(self.scope, "position", tokens[1], line_number)?,
                        parse_f32(self.scope, "position", tokens[2], line_number)?,
                        parse_f32(self.scope, "position", tokens[3], line_number)?,
                    ))
                })
            })
    }

    fn parse_texcoord(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        require_components(self.scope, tokens, 3, "texcoord", line_number)?;
        self.scope.with_work(
            ImportWorkClass::AttributeCells,
            1,
            |work| -> ParseResult<()> {
                work.run_all(|| {
                    self.parsed.texcoords.try_push(Vec2::new(
                        parse_f32(self.scope, "texcoord", tokens[1], line_number)?,
                        parse_f32(self.scope, "texcoord", tokens[2], line_number)?,
                    ))
                })
            },
        )
    }

    fn parse_normal(&mut self, tokens: &[Token<'_>], line_number: u32) -> ParseResult<()> {
        require_components(self.scope, tokens, 4, "normal", line_number)?;
        self.scope.with_work(
            ImportWorkClass::AttributeCells,
            1,
            |work| -> ParseResult<()> {
                work.run_all(|| {
                    self.parsed.normals.try_push(Vec3::new(
                        parse_f32(self.scope, "normal", tokens[1], line_number)?,
                        parse_f32(self.scope, "normal", tokens[2], line_number)?,
                        parse_f32(self.scope, "normal", tokens[3], line_number)?,
                    ))
                })
            },
        )
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

        let face_vertex_count = tokens.len() - 1;
        self.scope.with_work_group(
            [
                (ImportWorkClass::Faces, 1),
                (
                    ImportWorkClass::TopologyCells,
                    usize_amount(self.scope, face_vertex_count, "obj-face-topology-cells")?,
                ),
            ],
            |work| -> ParseResult<()> {
                let mut vertices = self
                    .scope
                    .metered_vec(ImportStorageClass::FormatNormalization, "obj-face-vertices");
                vertices.try_reserve(face_vertex_count)?;
                for chunk in tokens[1..].chunks(64) {
                    work.poll()?;
                    for token in chunk {
                        let vertex =
                            parse_face_vertex(self.scope, *token, &self.parsed, line_number)?;
                        work.run([0, 1], || vertices.try_push_reserved(vertex))?;
                    }
                }
                work.run([1, 0], || {
                    self.parsed.faces.try_push(Face {
                        vertices,
                        object: self.object,
                        group: self.group,
                        material: self.material,
                        smoothing: self.smoothing,
                    })
                })
            },
        )
    }

    fn parse_material_library_references(&mut self, tokens: &[Token<'_>]) -> ParseResult<()> {
        for token in tokens {
            let mtllib = checked_string(self.scope, token.text)?;
            self.parsed.mtllibs.try_push(mtllib)?;
        }
        Ok(())
    }
}

fn parse_state_name<'scope>(
    scope: &'scope ParseScope<'_>,
    state_names: &mut MeteredVec<'scope, MeteredString<'scope>>,
    current: Option<StateNameId>,
    text: Option<&str>,
) -> ParseResult<Option<StateNameId>> {
    let Some(text) = text.map(str::trim) else {
        return Ok(None);
    };
    parser::enforce_text_value_limit(scope, text.len(), "obj-state-name-bytes")?;
    let string_bytes = usize_amount(scope, text.len(), "obj-state-name-bytes")?;
    if let Some(current) = current {
        let current_name = state_names.as_slice().get(current.0).ok_or_else(|| {
            scope.format_failure(ObjFailure::internal("state-name-id-out-of-range"))
        })?;
        let matches_current = mtl::interruptible_str_eq(
            scope,
            current_name.as_str(),
            text,
            "obj-state-name-comparison",
        )?;
        if matches_current {
            return scope.with_work(
                ImportWorkClass::StringBytes,
                string_bytes,
                |work| -> ParseResult<_> {
                    work.complete_all()?;
                    Ok(Some(current))
                },
            );
        }
    }

    let mut name = scope.metered_string(ImportStorageClass::FormatNormalization, "obj-state-name");
    scope.with_work(
        ImportWorkClass::StringBytes,
        string_bytes,
        |work| -> ParseResult<_> {
            name.try_reserve(text.len())?;
            for_string_chunks(text, |chunk| {
                work.run(
                    [usize_amount(scope, chunk.len(), "obj-state-name-bytes")?],
                    || name.try_push_str_reserved(chunk),
                )
            })?;
            let id = StateNameId(state_names.len());
            state_names.try_push(name)?;
            Ok(Some(id))
        },
    )
}

fn parse_smoothing_group(
    scope: &ParseScope<'_>,
    tokens: &[Token<'_>],
    line_number: u32,
) -> ParseResult<ObjSmoothing> {
    let keyword_column = tokens.first().map_or(1, |token| token.column);
    let Some(value) = tokens.get(1).copied() else {
        return Err(scope.format_failure(ObjFailure::statement(
            "smoothing-group",
            "missing-value",
            line_number,
            keyword_column,
        )));
    };
    if let Some(extra) = tokens.get(2) {
        return Err(scope.format_failure(ObjFailure::statement(
            "smoothing-group",
            "extra-value",
            line_number,
            extra.column,
        )));
    }
    if value.text == "off" {
        return Ok(ObjSmoothing::Off);
    }

    enforce_numeric_literal(scope, value.text)?;
    let negative_digits = value.text.strip_prefix('-').is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    });
    let reason = if negative_digits {
        Some("negative")
    } else if value.text.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        Some("invalid-integer")
    };
    if let Some(reason) = reason {
        return Err(scope.format_failure(ObjFailure::statement(
            "smoothing-group",
            reason,
            line_number,
            value.column,
        )));
    }

    let group = value.text.parse::<u32>().map_err(|_| {
        scope.format_failure(ObjFailure::statement(
            "smoothing-group",
            "out-of-range",
            line_number,
            value.column,
        ))
    })?;
    Ok(NonZeroU32::new(group).map_or(ObjSmoothing::Off, ObjSmoothing::Group))
}

fn tokenize<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    decoded_line: &'line TextInputLine<'_, '_>,
    line_number: u32,
) -> ParseResult<TokenBuffer<'scope, 'line>> {
    let line = decoded_line.text();
    let mut tokens = TokenBuffer {
        values: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-tokens"),
        comment_start: None,
    };

    tokens.comment_start = parser::scan_token_ranges(
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
    scope.with_work_group(
        [
            (ImportWorkClass::Tokens, 1),
            (
                ImportWorkClass::TokenBytes,
                usize_amount(scope, text.len(), "obj-token-bytes")?,
            ),
        ],
        |work| -> ParseResult<()> {
            work.run_all(|| {
                let column = start
                    .checked_add(1)
                    .and_then(|column| u32::try_from(column).ok())
                    .ok_or_else(|| scope.resource_limit_exceeded("line-columns", u64::MAX))?;
                tokens.values.try_push(Token {
                    text,
                    column,
                    start,
                    end,
                })
            })
        },
    )
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
    let value = text.parse::<i64>().map_err(|_| {
        scope.format_failure(ObjFailure::invalid_index(
            kind,
            "invalid-integer",
            line_number,
            column,
        ))
    })?;
    if value == 0 {
        return Err(scope.format_failure(ObjFailure::invalid_index(
            kind,
            "zero",
            line_number,
            column,
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
    let value = token.text.parse::<f32>().map_err(|_| {
        scope.format_failure(ObjFailure::invalid_float(
            subject,
            line_number,
            token.column,
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
    text: &str,
) -> ParseResult<MeteredString<'scope>> {
    parser::enforce_text_value_limit(scope, text.len(), "obj-material-library-reference")?;
    let mut owned = scope.metered_string(
        ImportStorageClass::FormatNormalization,
        "obj-material-library-reference",
    );
    scope.with_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, text.len(), "obj-string-bytes")?,
        |work| -> ParseResult<_> {
            owned.try_reserve(text.len())?;
            for_string_chunks(text, |chunk| {
                work.run(
                    [usize_amount(scope, chunk.len(), "obj-string-bytes")?],
                    || owned.try_push_str_reserved(chunk),
                )
            })?;
            Ok(owned)
        },
    )
}

fn for_string_chunks(
    value: &str,
    mut visit: impl FnMut(&str) -> ParseResult<()>,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < value.len() {
        let mut end = start
            .checked_add(parser::SCAN_CHUNK_BYTES)
            .map_or(value.len(), |end| end.min(value.len()));
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        visit(&value[start..end])?;
        start = end;
    }
    Ok(())
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
    fn plain_obj_records_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<FaceVertex>();
        assert_send_sync::<StateNameId>();
    }
}
