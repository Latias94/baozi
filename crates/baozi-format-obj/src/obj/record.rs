//! Numeric record shapes are selected before interpreting optional components.

use baozi_import::{DiagnosticCode, SourceScopeEntityKind};

use super::{
    ImportStorageClass, ObjFailure, ObjParser, ParseChildren, ParsePieceError, ParseResult,
    ParseServices, ParsedTexcoord, Token, Vec3, parse_f32, resolve_index,
};

impl<'op, 'session> ObjParser<'op, 'session> {
    pub(super) fn parse_omitted_geometry(
        &mut self,
        exec: &mut ParseServices<'_, 'op, 'session>,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        let is_line = tokens[0].text == "l";
        let minimum = if is_line { 2 } else { 1 };
        if tokens.len() - 1 < minimum {
            return Err(exec.format_failure(ObjFailure::statement(
                "point-or-line",
                "too-few-vertices",
                line_number,
                tokens[0].column,
            )));
        }
        // Validate supported point/line reference shapes before requesting omission authority.
        // Normal indices and free-form line variants are outside this bounded OBJ profile.
        for chunk in tokens[1..].chunks(64) {
            exec.poll()?;
            for token in chunk {
                let mut parts = token.text.split('/');
                let position = parts.next().unwrap_or_default();
                let texcoord = parts.next();
                if position.is_empty()
                    || parts.next().is_some()
                    || (!is_line && texcoord.is_some())
                    || texcoord == Some("")
                {
                    return Err(exec.format_failure(ObjFailure::statement(
                        "point-or-line",
                        "invalid-vertex-reference",
                        line_number,
                        token.column,
                    )));
                }
                resolve_index(
                    exec,
                    position,
                    self.parsed.positions.len(),
                    "position",
                    line_number,
                    token.column,
                )?;
                if let Some(texcoord) = texcoord {
                    resolve_index(
                        exec,
                        texcoord,
                        self.parsed.texcoords.len(),
                        "texcoord",
                        line_number,
                        token.column,
                    )?;
                }
            }
        }
        let mut path = exec.metered_string(
            ImportStorageClass::FormatNormalization,
            "obj-omitted-geometry-path",
        );
        path.try_write_fmt(
            exec,
            format_args!("/records/{line_number}/{}", tokens[0].text),
        )?;
        let code = DiagnosticCode::new(if is_line {
            "obj.line_geometry_omitted"
        } else {
            "obj.point_geometry_omitted"
        })
        .expect("static OBJ diagnostic code must be valid");
        exec.record_source_semantic_loss(
            code,
            SourceScopeEntityKind::Primitive,
            path.as_str(),
            |details| {
                details.unsigned("line", u64::from(line_number));
                details.unsigned("column", u64::from(tokens[0].column));
                details.string("statement", tokens[0].text);
                details.unsigned("reference_count", (tokens.len() - 1) as u64);
            },
            format_args!(
                "OBJ '{}' geometry at line {line_number} is not represented; omission requires explicit source semantic loss authorization",
                tokens[0].text,
            ),
        )
    }

    pub(super) fn parse_position(
        &mut self,
        children: &mut ParseChildren<'_, 'op, 'session>,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(
            &mut children.services()?,
            tokens,
            &[3, 4, 6, 7],
            "position",
            line_number,
        )?;
        let components = tokens.len() - 1;
        let errors = children.error_view();
        let position = children.child(&self.vertex_work, [1], |work| {
            work.run_returned_with_storage(|_storage| {
                let mut position = Vec3::new(
                    parse_f32(errors, "position", tokens[1], line_number)?,
                    parse_f32(errors, "position", tokens[2], line_number)?,
                    parse_f32(errors, "position", tokens[3], line_number)?,
                );
                if components == 4 {
                    let weight = parse_f32(errors, "position-weight", tokens[4], line_number)?;
                    if weight == 0.0 {
                        return Err(ParsePieceError::Format(ObjFailure::statement(
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
                        return Err(ParsePieceError::Format(ObjFailure::non_finite(
                            "homogeneous-position",
                            line_number,
                            tokens[4].column,
                        )));
                    }
                } else {
                    // Six/seven-component extensions are not homogeneous positions. Validate
                    // the entire suffix before asking for authority to omit its semantics.
                    for token in &tokens[4..] {
                        parse_f32(errors, "vertex-color", *token, line_number)?;
                    }
                }
                Ok((position, [1]))
            })
        })?;
        if matches!(components, 6 | 7) {
            record_vertex_color_loss(&mut children.services()?, tokens, line_number)?;
        }
        self.parsed
            .positions
            .try_push(&mut children.services()?, position)
    }

    pub(super) fn parse_texcoord(
        &mut self,
        children: &mut ParseChildren<'_, 'op, 'session>,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(
            &mut children.services()?,
            tokens,
            &[2, 3],
            "texcoord",
            line_number,
        )?;
        let lanes = if tokens.len() == 4 { 3 } else { 2 };
        let errors = children.error_view();
        children.child(&self.attribute_work, [1], |work| {
            work.run_returned_with_storage(|mut storage| {
                let value = [
                    parse_f32(errors, "texcoord", tokens[1], line_number)?,
                    parse_f32(errors, "texcoord", tokens[2], line_number)?,
                    if lanes == 3 {
                        parse_f32(errors, "texcoord", tokens[3], line_number)?
                    } else {
                        0.0
                    },
                ];
                storage.push_vec(
                    &mut self.parsed.texcoords,
                    ParsedTexcoord {
                        values: value,
                        lanes,
                    },
                )?;
                Ok(((), [1]))
            })
        })
    }

    pub(super) fn parse_normal(
        &mut self,
        children: &mut ParseChildren<'_, 'op, 'session>,
        tokens: &[Token<'_>],
        line_number: u32,
    ) -> ParseResult<()> {
        require_record_components(
            &mut children.services()?,
            tokens,
            &[3],
            "normal",
            line_number,
        )?;
        let errors = children.error_view();
        children.child(&self.attribute_work, [1], |work| {
            work.run_returned_with_storage(|mut storage| {
                let normal = [
                    parse_f32(errors, "normal", tokens[1], line_number)?,
                    parse_f32(errors, "normal", tokens[2], line_number)?,
                    parse_f32(errors, "normal", tokens[3], line_number)?,
                ];
                storage.push_vec(&mut self.parsed.normals, normal)?;
                Ok(((), [1]))
            })
        })
    }
}

fn require_record_components(
    exec: &mut ParseServices<'_, '_, '_>,
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
    Err(exec.format_failure(ObjFailure::statement(
        subject,
        reason,
        line_number,
        tokens.first().map_or(1, |token| token.column),
    )))
}

fn record_vertex_color_loss(
    exec: &mut ParseServices<'_, '_, '_>,
    tokens: &[Token<'_>],
    line_number: u32,
) -> ParseResult<()> {
    let mut object_path = exec.metered_string(
        ImportStorageClass::FormatNormalization,
        "obj-vertex-color-object-path",
    );
    object_path.try_write_fmt(exec, format_args!("/records/{line_number}/vertex-color"))?;
    let components = tokens.len() - 1;
    let code = DiagnosticCode::new("obj.vertex_color_omitted")
        .expect("static OBJ diagnostic code must be valid");
    exec.record_source_semantic_loss(
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
