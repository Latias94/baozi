//! Numeric record shapes are selected before interpreting optional components.

use baozi_import::{DiagnosticCode, SourceScopeEntityKind};

use super::{
    ImportStorageClass, ImportWorkClass, ObjFailure, ObjParser, ParseResult, ParseScope,
    ParsedTexcoord, Token, Vec3, parse_f32,
};

impl ObjParser<'_, '_> {
    pub(super) fn parse_position(
        &mut self,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(self.scope, tokens, &[3, 4, 6, 7], "position", line_number)?;
        let components = tokens.len() - 1;
        let position =
            self.scope
                .with_work(ImportWorkClass::Vertices, 1, |work| -> ParseResult<_> {
                    work.run_all(|| {
                        let mut position = Vec3::new(
                            parse_f32(self.scope, "position", tokens[1], line_number)?,
                            parse_f32(self.scope, "position", tokens[2], line_number)?,
                            parse_f32(self.scope, "position", tokens[3], line_number)?,
                        );
                        if components == 4 {
                            let weight =
                                parse_f32(self.scope, "position-weight", tokens[4], line_number)?;
                            if weight == 0.0 {
                                return Err(self.scope.format_failure(ObjFailure::statement(
                                    "position-weight",
                                    "zero-homogeneous-weight",
                                    line_number,
                                    tokens[4].column,
                                )));
                            }
                            position = Vec3::new(
                                position.x / weight,
                                position.y / weight,
                                position.z / weight,
                            );
                            if !position.is_finite() {
                                return Err(self.scope.format_failure(ObjFailure::non_finite(
                                    "homogeneous-position",
                                    line_number,
                                    tokens[4].column,
                                )));
                            }
                        } else {
                            // Six/seven-component extensions are not homogeneous positions. Validate
                            // the entire suffix before asking for authority to omit its semantics.
                            for token in &tokens[4..] {
                                parse_f32(self.scope, "vertex-color", *token, line_number)?;
                            }
                        }
                        Ok(position)
                    })
                })?;
        if matches!(components, 6 | 7) {
            record_vertex_color_loss(self.scope, tokens, line_number)?;
        }
        self.parsed.positions.try_push(position)
    }

    pub(super) fn parse_texcoord(
        &mut self,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(self.scope, tokens, &[2, 3], "texcoord", line_number)?;
        let lanes = if tokens.len() == 4 { 3 } else { 2 };
        self.scope.with_work(
            ImportWorkClass::AttributeCells,
            1,
            |work| -> ParseResult<()> {
                work.run_all(|| {
                    let value = [
                        parse_f32(self.scope, "texcoord", tokens[1], line_number)?,
                        parse_f32(self.scope, "texcoord", tokens[2], line_number)?,
                        if lanes == 3 {
                            parse_f32(self.scope, "texcoord", tokens[3], line_number)?
                        } else {
                            0.0
                        },
                    ];
                    self.parsed.texcoords.try_push(ParsedTexcoord {
                        values: value,
                        lanes,
                    })
                })
            },
        )
    }

    pub(super) fn parse_normal(
        &mut self,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(self.scope, tokens, &[3], "normal", line_number)?;
        self.scope.with_work(
            ImportWorkClass::AttributeCells,
            1,
            |work| -> ParseResult<()> {
                work.run_all(|| {
                    self.parsed.normals.try_push([
                        parse_f32(self.scope, "normal", tokens[1], line_number)?,
                        parse_f32(self.scope, "normal", tokens[2], line_number)?,
                        parse_f32(self.scope, "normal", tokens[3], line_number)?,
                    ])
                })
            },
        )
    }
}

fn require_record_components(
    scope: &ParseScope<'_>,
    tokens: &[Token<'_>],
    allowed: &[usize],
    subject: &'static str,
    line_number: u32,
) -> ParseResult<()> {
    let components = tokens.len().saturating_sub(1);
    let reason = if components < allowed[0] {
        "missing-components"
    } else if allowed.contains(&components) {
        return Ok(());
    } else {
        "unsupported-component-count"
    };
    Err(scope.format_failure(ObjFailure::statement(
        subject,
        reason,
        line_number,
        tokens.first().map_or(1, |token| token.column),
    )))
}

fn record_vertex_color_loss(
    scope: &ParseScope<'_>,
    tokens: &[Token<'_>],
    line_number: u32,
) -> ParseResult<()> {
    let mut object_path = scope.metered_string(
        ImportStorageClass::FormatNormalization,
        "obj-vertex-color-object-path",
    );
    object_path.try_write_fmt(format_args!("/records/{line_number}/vertex-color"))?;
    let components = tokens.len() - 1;
    let code = DiagnosticCode::new("obj.vertex_color_omitted")
        .expect("static OBJ diagnostic code must be valid");
    scope.record_source_semantic_loss(
        code,
        SourceScopeEntityKind::Mesh,
        object_path.as_str(),
        |details| {
            details.unsigned("line", u64::from(line_number));
            details.unsigned("column", u64::from(tokens[4].column));
            details.unsigned("component_count", components as u64);
            details.unsigned("discarded_components", (components - 3) as u64);
        },
        format_args!(
            "OBJ {components}-component vertex record at line {line_number} has an unsupported color suffix; retaining XYZ and omitting the suffix requires explicit source semantic loss authorization"
        ),
    )
}
