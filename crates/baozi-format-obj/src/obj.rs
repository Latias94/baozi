use crate::{ObjFailure, mtl, parser};
use baozi_import::authoring::Vec3;
use baozi_import::{
    ConformedTextInput, ImportStorageClass, ImportWorkClass, MeteredString, MeteredVec,
    ParseExecution, ParseResult, TextInputLine, TextInputRecoveryTracker,
};
use std::num::NonZeroU32;

use parser::usize_amount;

mod record;

const MAX_NUMERIC_LITERAL_BYTES: usize = 128;
const MAX_FACE_REFERENCE_BYTES: usize = MAX_NUMERIC_LITERAL_BYTES * 3 + 2;

pub(crate) struct ParsedObj<'session> {
    pub positions: MeteredVec<'session, Vec3>,
    pub texcoords: MeteredVec<'session, ParsedTexcoord>,
    pub normals: MeteredVec<'session, [f32; 3]>,
    pub faces: MeteredVec<'session, Face<'session>>,
    pub mtllibs: MeteredVec<'session, MeteredString<'session>>,
    state_names: MeteredVec<'session, MeteredString<'session>>,
}

#[derive(Clone, Copy)]
pub(crate) struct ParsedTexcoord {
    pub values: [f32; 3],
    pub lanes: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct StateNameId(usize);

#[cfg(test)]
impl StateNameId {
    pub(crate) const fn from_index(index: usize) -> Self {
        Self(index)
    }
}

pub(crate) struct Face<'session> {
    pub vertices: MeteredVec<'session, FaceVertex>,
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

pub(crate) fn parse<'region, 'session>(
    exec: &mut ParseExecution<'region, 'session>,
    text: &ConformedTextInput<'_>,
    recoveries: &mut TextInputRecoveryTracker<'_, 'session>,
) -> ParseResult<ParsedObj<'session>> {
    let mut parser = ObjParser {
        face_work: exec
            .bind_work_group([ImportWorkClass::Faces, ImportWorkClass::TopologyCells])?,
        vertex_work: exec.bind_work_group([ImportWorkClass::Vertices])?,
        attribute_work: exec.bind_work_group([ImportWorkClass::AttributeCells])?,
        parsed: ParsedObj {
            positions: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-positions"),
            texcoords: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-texcoords"),
            normals: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-normals"),
            faces: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-faces"),
            mtllibs: exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-material-library-references",
            ),
            state_names: exec
                .metered_vec(ImportStorageClass::FormatNormalization, "obj-state-names"),
        },
        object: None,
        group: None,
        material: None,
        smoothing: ObjSmoothing::Unspecified,
    };
    let mut scan_scratch = parser::TokenScanScratch::new();

    let record_work = exec.bind_work_group([ImportWorkClass::Records])?;
    for line in text.lines() {
        let line_number = u32::try_from(line.number())
            .map_err(|_| exec.resource_limit_exceeded("line-count", u64::from(u32::MAX)))?;
        let line = exec.with_work(&record_work, [1], |_exec, work| -> ParseResult<_> {
            work.complete_all()?;
            Ok(line)
        })?;
        parser.parse_line(exec, &mut scan_scratch, recoveries, &line, line_number)?;
        #[cfg(test)]
        crate::control_tests::face_completed(parser.parsed.faces.len());
    }

    Ok(parser.parsed)
}

struct ObjParser<'region, 'session> {
    face_work: baozi_import::ImportWorkGroup<'region, 2>,
    vertex_work: baozi_import::ImportWorkGroup<'region, 1>,
    attribute_work: baozi_import::ImportWorkGroup<'region, 1>,
    parsed: ParsedObj<'session>,
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

struct TokenBuffer<'session, 'line> {
    values: MeteredVec<'session, Token<'line>>,
    comment_start: Option<usize>,
}

impl<'region, 'session> ObjParser<'region, 'session> {
    fn parse_line(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        scan_scratch: &mut parser::TokenScanScratch,
        recoveries: &mut TextInputRecoveryTracker<'_, 'session>,
        decoded_line: &TextInputLine<'_, '_>,
        line_number: u32,
    ) -> ParseResult<()> {
        let raw_line = decoded_line.text();
        let tokens = tokenize(exec, scan_scratch, decoded_line, line_number)?;
        let token_values = tokens.values.as_slice();
        let allowed_value = token_values.first().and_then(|first| match first.text {
            "o" | "g" | "usemtl" | "mtllib" => trailing_token_range(token_values),
            _ => None,
        });
        parser::authorize_line_recoveries(
            exec,
            recoveries,
            decoded_line,
            allowed_value,
            tokens.comment_start,
        )?;
        let Some(first) = token_values.first() else {
            return Ok(());
        };

        match first.text {
            "v" => self.parse_position(exec, token_values, line_number),
            "vt" => self.parse_texcoord(exec, token_values, line_number),
            "vn" => self.parse_normal(exec, token_values, line_number),
            "f" => self.parse_face(exec, token_values, line_number),
            "p" | "l" => self.parse_omitted_geometry(exec, token_values, line_number),
            "o" => {
                self.object = parse_state_name(
                    exec,
                    &mut self.parsed.state_names,
                    self.object,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "g" => {
                self.group = parse_state_name(
                    exec,
                    &mut self.parsed.state_names,
                    self.group,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "usemtl" => {
                self.material = parse_state_name(
                    exec,
                    &mut self.parsed.state_names,
                    self.material,
                    trailing_text_after_first_token(raw_line, token_values),
                )?;
                Ok(())
            }
            "s" => {
                self.smoothing = parse_smoothing_group(exec, token_values, line_number)?;
                Ok(())
            }
            "mtllib" => self.parse_material_library_references(exec, &token_values[1..]),
            keyword if is_unsupported_statement(keyword) => mtl::push_warning(
                exec,
                Some((line_number, first.column)),
                "obj.unsupported_statement",
                format_args!("OBJ statement '{keyword}' is not supported yet"),
            ),
            keyword => mtl::push_warning(
                exec,
                Some((line_number, first.column)),
                "obj.unknown_statement",
                format_args!("OBJ statement '{keyword}' is not recognized"),
            ),
        }
    }

    fn parse_face(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        if tokens.len() < 4 {
            let column = tokens.first().map_or(1, |token| token.column);
            return Err(exec.format_failure(ObjFailure::face(
                "too-few-vertices",
                line_number,
                column,
            )));
        }

        let face_vertex_count = tokens.len() - 1;
        exec.with_work(
            &self.face_work,
            [
                1,
                usize_amount(exec, face_vertex_count, "obj-face-topology-cells")?,
            ],
            |exec, work| -> ParseResult<()> {
                let mut vertices =
                    exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-face-vertices");
                vertices.try_reserve(exec, face_vertex_count)?;
                for chunk in tokens[1..].chunks(64) {
                    exec.poll()?;
                    for token in chunk {
                        let vertex = parse_face_vertex(exec, *token, &self.parsed, line_number)?;
                        work.run([0, 1], || vertices.try_push_reserved(vertex))?;
                    }
                }
                work.run([1, 0], || {
                    self.parsed.faces.try_push(
                        exec,
                        Face {
                            vertices,
                            object: self.object,
                            group: self.group,
                            material: self.material,
                            smoothing: self.smoothing,
                        },
                    )
                })
            },
        )
    }

    fn parse_material_library_references(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        tokens: &[Token<'_>],
    ) -> ParseResult<()> {
        for token in tokens {
            let mtllib = checked_string(exec, token.text)?;
            self.parsed.mtllibs.try_push(exec, mtllib)?;
        }
        Ok(())
    }
}

fn parse_state_name<'session>(
    exec: &mut ParseExecution<'_, 'session>,
    state_names: &mut MeteredVec<'session, MeteredString<'session>>,
    current: Option<StateNameId>,
    text: Option<&str>,
) -> ParseResult<Option<StateNameId>> {
    let Some(text) = text.map(str::trim) else {
        return Ok(None);
    };
    let string_work = exec.bind_work_group([ImportWorkClass::StringBytes])?;
    parser::enforce_text_value_limit(exec, text.len(), "obj-state-name-bytes")?;
    let string_bytes = usize_amount(exec, text.len(), "obj-state-name-bytes")?;
    if let Some(current) = current {
        let current_name = state_names.as_slice().get(current.0).ok_or_else(|| {
            exec.format_failure(ObjFailure::internal("state-name-id-out-of-range"))
        })?;
        let matches_current = mtl::interruptible_str_eq(
            exec,
            current_name.as_str(),
            text,
            "obj-state-name-comparison",
        )?;
        if matches_current {
            return exec.with_work(
                &string_work,
                [string_bytes],
                |_exec, work| -> ParseResult<_> {
                    work.complete_all()?;
                    Ok(Some(current))
                },
            );
        }
    }

    let mut name = exec.metered_string(ImportStorageClass::FormatNormalization, "obj-state-name");
    exec.with_work(
        &string_work,
        [string_bytes],
        |exec, work| -> ParseResult<_> {
            name.try_reserve(exec, text.len())?;
            for_string_chunks(text, |chunk| {
                work.run(
                    [usize_amount(exec, chunk.len(), "obj-state-name-bytes")?],
                    || name.try_push_str_reserved(chunk),
                )
            })?;
            let id = StateNameId(state_names.len());
            state_names.try_push(exec, name)?;
            Ok(Some(id))
        },
    )
}

fn parse_smoothing_group(
    exec: &mut ParseExecution<'_, '_>,
    tokens: &[Token<'_>],
    line_number: u32,
) -> ParseResult<ObjSmoothing> {
    let keyword_column = tokens.first().map_or(1, |token| token.column);
    let Some(value) = tokens.get(1).copied() else {
        return Err(exec.format_failure(ObjFailure::statement(
            "smoothing-group",
            "missing-value",
            line_number,
            keyword_column,
        )));
    };
    if let Some(extra) = tokens.get(2) {
        return Err(exec.format_failure(ObjFailure::statement(
            "smoothing-group",
            "extra-value",
            line_number,
            extra.column,
        )));
    }
    if value.text == "off" {
        return Ok(ObjSmoothing::Off);
    }

    enforce_numeric_literal(exec, value.text)?;
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
        return Err(exec.format_failure(ObjFailure::statement(
            "smoothing-group",
            reason,
            line_number,
            value.column,
        )));
    }

    let group = value.text.parse::<u32>().map_err(|_| {
        exec.format_failure(ObjFailure::statement(
            "smoothing-group",
            "out-of-range",
            line_number,
            value.column,
        ))
    })?;
    Ok(NonZeroU32::new(group).map_or(ObjSmoothing::Off, ObjSmoothing::Group))
}

fn tokenize<'session, 'line>(
    exec: &mut ParseExecution<'_, 'session>,
    scan_scratch: &mut parser::TokenScanScratch,
    decoded_line: &'line TextInputLine<'_, '_>,
    line_number: u32,
) -> ParseResult<TokenBuffer<'session, 'line>> {
    let line = decoded_line.text();
    let token_work =
        exec.bind_work_group([ImportWorkClass::Tokens, ImportWorkClass::TokenBytes])?;
    let mut tokens = TokenBuffer {
        values: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-tokens"),
        comment_start: None,
    };

    tokens.comment_start = parser::scan_token_ranges(
        exec,
        scan_scratch,
        line,
        "obj-token-scan-work",
        |exec, index, character| {
            if character == '\0' {
                let column = index
                    .checked_add(1)
                    .and_then(|column| u32::try_from(column).ok())
                    .ok_or_else(|| exec.resource_limit_exceeded("line-columns", u64::MAX))?;
                return Err(exec.format_failure(ObjFailure::statement(
                    "token",
                    "contains-nul",
                    line_number,
                    column,
                )));
            }
            Ok(())
        },
        |exec, token_start, token_end| {
            push_token(exec, &token_work, &mut tokens, line, token_start, token_end)
        },
    )?;
    Ok(tokens)
}

fn push_token<'region, 'session, 'line>(
    exec: &mut ParseExecution<'region, 'session>,
    token_work: &baozi_import::ImportWorkGroup<'region, 2>,
    tokens: &mut TokenBuffer<'session, 'line>,
    line: &'line str,
    start: usize,
    end: usize,
) -> ParseResult<()> {
    let text = &line[start..end];
    exec.with_work(
        token_work,
        [1, usize_amount(exec, text.len(), "obj-token-bytes")?],
        |exec, work| -> ParseResult<()> {
            work.run_all(|| {
                let column = start
                    .checked_add(1)
                    .and_then(|column| u32::try_from(column).ok())
                    .ok_or_else(|| exec.resource_limit_exceeded("line-columns", u64::MAX))?;
                tokens.values.try_push(
                    exec,
                    Token {
                        text,
                        column,
                        start,
                        end,
                    },
                )
            })
        },
    )
}

fn parse_face_vertex(
    exec: &mut ParseExecution<'_, '_>,
    token: Token<'_>,
    parsed: &ParsedObj<'_>,
    line_number: u32,
) -> ParseResult<FaceVertex> {
    exec.enforce_resource_limit(
        "obj-face-reference-bytes",
        usize_amount(exec, token.text.len(), "obj-face-reference-bytes")?,
        MAX_FACE_REFERENCE_BYTES as u64,
    )?;
    let mut parts = token.text.split('/');
    let position = parts.next().unwrap_or_default();
    let texcoord = parts.next();
    let normal = parts.next();
    if parts.next().is_some() || position.is_empty() {
        return Err(exec.format_failure(ObjFailure::face(
            "invalid-vertex-reference",
            line_number,
            token.column,
        )));
    }

    Ok(FaceVertex {
        position: resolve_index(
            exec,
            position,
            parsed.positions.len(),
            "position",
            line_number,
            token.column,
        )?,
        texcoord: match texcoord {
            Some("") | None => None,
            Some(value) => Some(resolve_index(
                exec,
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
                exec,
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
    exec: &mut ParseExecution<'_, '_>,
    text: &str,
    len: usize,
    kind: &'static str,
    line_number: u32,
    column: u32,
) -> ParseResult<usize> {
    enforce_numeric_literal(exec, text)?;
    let value = text.parse::<i64>().map_err(|_| {
        exec.format_failure(ObjFailure::invalid_index(
            kind,
            "invalid-integer",
            line_number,
            column,
        ))
    })?;
    if value == 0 {
        return Err(exec.format_failure(ObjFailure::invalid_index(
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
        return Err(exec.format_failure(ObjFailure::invalid_index(
            kind,
            "out-of-range",
            line_number,
            column,
        )));
    }
    Ok(resolved as usize)
}

fn parse_f32(
    exec: &mut ParseExecution<'_, '_>,
    subject: &'static str,
    token: Token<'_>,
    line_number: u32,
) -> ParseResult<f32> {
    enforce_numeric_literal(exec, token.text)?;
    let value = token.text.parse::<f32>().map_err(|_| {
        exec.format_failure(ObjFailure::invalid_float(
            subject,
            line_number,
            token.column,
        ))
    })?;
    if !value.is_finite() {
        return Err(exec.format_failure(ObjFailure::non_finite(
            subject,
            line_number,
            token.column,
        )));
    }
    Ok(value)
}

fn checked_string<'region, 'session>(
    exec: &mut ParseExecution<'region, 'session>,
    text: &str,
) -> ParseResult<MeteredString<'session>> {
    parser::enforce_text_value_limit(exec, text.len(), "obj-material-library-reference")?;
    let mut owned = exec.metered_string(
        ImportStorageClass::FormatNormalization,
        "obj-material-library-reference",
    );
    let string_work = exec.bind_work_group([ImportWorkClass::StringBytes])?;
    exec.with_work(
        &string_work,
        [usize_amount(exec, text.len(), "obj-string-bytes")?],
        |exec, work| -> ParseResult<_> {
            owned.try_reserve(exec, text.len())?;
            for_string_chunks(text, |chunk| {
                work.run(
                    [usize_amount(exec, chunk.len(), "obj-string-bytes")?],
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

fn enforce_numeric_literal(exec: &mut ParseExecution<'_, '_>, text: &str) -> ParseResult<()> {
    exec.enforce_resource_limit(
        "obj-number-bytes",
        usize_amount(exec, text.len(), "obj-number-bytes")?,
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
        "curv"
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
