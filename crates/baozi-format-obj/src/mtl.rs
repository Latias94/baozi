use baozi_core::{
    AlphaMode, BaoziError, Color, Diagnostic, DiagnosticCode, DiagnosticSeverity, MetadataMap,
    MetadataValue, Result, SourceLocation,
};
use baozi_import::{ExternalReferencePolicy, ImportContext};
use baozi_io::AssetPath;
use std::collections::BTreeMap;
use std::io::Read;

#[derive(Debug, Clone, Default)]
pub(crate) struct MaterialLibrary {
    pub materials: BTreeMap<String, ParsedMaterial>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedMaterial {
    pub name: String,
    pub base_color: Color,
    pub emissive: Color,
    pub alpha_mode: AlphaMode,
    pub metadata: MetadataMap,
    pub diffuse_texture: Option<ParsedTexture>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedTexture {
    pub uri: String,
    pub source_key: &'static str,
}

impl ParsedMaterial {
    fn new(name: String) -> Self {
        Self {
            name,
            base_color: Color::WHITE,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            alpha_mode: AlphaMode::Opaque,
            metadata: MetadataMap::new(),
            diffuse_texture: None,
        }
    }
}

pub(crate) fn load_libraries(
    ctx: &mut ImportContext<'_>,
    mtllibs: &[String],
) -> Result<MaterialLibrary> {
    let mut library = MaterialLibrary::default();
    for mtllib in mtllibs {
        load_library(ctx, mtllib, &mut library)?;
    }
    Ok(library)
}

fn load_library(
    ctx: &mut ImportContext<'_>,
    mtllib: &str,
    library: &mut MaterialLibrary,
) -> Result<()> {
    if matches!(
        ctx.options.io.external_references,
        ExternalReferencePolicy::Deny
    ) {
        push_warning(
            ctx,
            ctx.source.to_string(),
            None,
            "obj.mtl_denied",
            format!("MTL sidecar '{mtllib}' was not loaded because external references are denied"),
        );
        return Ok(());
    }

    let path = match ctx.io.resolve(&ctx.source, mtllib) {
        Ok(path) => path,
        Err(error) => {
            push_warning(
                ctx,
                ctx.source.to_string(),
                None,
                "obj.mtl_resolve_failed",
                format!("MTL sidecar '{mtllib}' could not be resolved: {error}"),
            );
            return Ok(());
        }
    };

    let Some(bytes) = read_sidecar_bytes(ctx, &path) else {
        return Ok(());
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) => {
            push_warning(
                ctx,
                path.to_string(),
                Some(SourceLocation::byte(error.valid_up_to() as u64)),
                "obj.mtl_invalid_utf8",
                "MTL sidecar is not valid UTF-8",
            );
            return Ok(());
        }
    };

    parse_mtl(ctx, &path, text, library);
    Ok(())
}

fn read_sidecar_bytes(ctx: &mut ImportContext<'_>, path: &AssetPath) -> Option<Vec<u8>> {
    let mut reader = match ctx.io.open(path) {
        Ok(reader) => reader,
        Err(error) => {
            push_warning(
                ctx,
                path.to_string(),
                None,
                "obj.mtl_missing",
                format!("MTL sidecar '{path}' could not be opened: {error}"),
            );
            return None;
        }
    };

    let limit = ctx.options.limits.max_sidecar_asset_bytes;
    let mut bytes = Vec::new();
    let mut limited = reader.by_ref().take(limit.saturating_add(1));
    if let Err(error) = limited.read_to_end(&mut bytes) {
        push_warning(
            ctx,
            path.to_string(),
            None,
            "obj.mtl_read_failed",
            format!("MTL sidecar '{path}' could not be read: {error}"),
        );
        return None;
    }
    if bytes.len() as u64 > limit {
        push_warning(
            ctx,
            path.to_string(),
            None,
            "obj.mtl_limit_exceeded",
            "MTL sidecar exceeded max_sidecar_asset_bytes and was ignored",
        );
        return None;
    }
    Some(bytes)
}

fn parse_mtl(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    text: &str,
    library: &mut MaterialLibrary,
) {
    let mut current = None;

    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = (line_index + 1) as u32;
        if raw_line.len() > ctx.options.limits.max_line_bytes {
            push_warning(
                ctx,
                path.to_string(),
                Some(SourceLocation::line_column(line_number, 1)),
                "obj.mtl_line_limit",
                "MTL line exceeded max_line_bytes and was ignored",
            );
            continue;
        }

        let line = raw_line.split_once('#').map_or(raw_line, |(head, _)| head);
        let tokens = match tokenize_mtl(ctx, path, line, line_number) {
            Some(tokens) => tokens,
            None => continue,
        };
        let Some(keyword) = tokens.first().copied() else {
            continue;
        };

        match keyword {
            "newmtl" => {
                flush_material(ctx, path, library, current.take());
                let name = trailing_text_after_keyword(line, keyword).unwrap_or_default();
                if name.is_empty() {
                    push_warning(
                        ctx,
                        path.to_string(),
                        Some(SourceLocation::line_column(line_number, 1)),
                        "obj.mtl_empty_name",
                        "MTL newmtl has no material name",
                    );
                } else if name.len() > ctx.options.limits.max_string_bytes {
                    push_warning(
                        ctx,
                        path.to_string(),
                        Some(SourceLocation::line_column(line_number, 1)),
                        "obj.mtl_name_limit",
                        "MTL material name exceeded max_string_bytes",
                    );
                } else {
                    current = Some(ParsedMaterial::new(name.to_owned()));
                }
            }
            "Kd" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(ctx, path, &tokens, line_number, "Kd")
                {
                    material.base_color.r = color.r;
                    material.base_color.g = color.g;
                    material.base_color.b = color.b;
                }
            }
            "d" => {
                if let Some(material) = current.as_mut()
                    && let Some(alpha) = parse_scalar(ctx, path, &tokens, line_number, "d")
                {
                    material.base_color.a = alpha;
                    if alpha < 1.0 {
                        material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Tr" => {
                if let Some(material) = current.as_mut()
                    && let Some(transparency) = parse_scalar(ctx, path, &tokens, line_number, "Tr")
                {
                    let alpha = 1.0 - transparency;
                    material.base_color.a = alpha;
                    if alpha < 1.0 {
                        material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Ke" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(ctx, path, &tokens, line_number, "Ke")
                {
                    material.emissive = color;
                }
            }
            "Ns" | "Ni" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_scalar(ctx, path, &tokens, line_number, keyword)
                {
                    material
                        .metadata
                        .insert(format!("obj:{keyword}"), MetadataValue::F64(value as f64));
                }
            }
            "illum" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_i64(ctx, path, &tokens, line_number, "illum")
                {
                    material
                        .metadata
                        .insert("obj:illum".to_owned(), MetadataValue::I64(value));
                }
            }
            "Ka" | "Ks" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) =
                        parse_vec_metadata(ctx, path, &tokens, line_number, keyword)
                {
                    material
                        .metadata
                        .insert(format!("obj:{keyword}"), MetadataValue::String(value));
                }
            }
            "map_Kd" => {
                if let Some(material) = current.as_mut() {
                    match texture_path_from_map(&tokens[1..]) {
                        Some(texture_path) => match ctx.io.resolve(path, &texture_path) {
                            Ok(resolved) => {
                                material.diffuse_texture = Some(ParsedTexture {
                                    uri: resolved.to_string(),
                                    source_key: "map_Kd",
                                });
                            }
                            Err(error) => push_warning(
                                ctx,
                                path.to_string(),
                                Some(SourceLocation::line_column(line_number, 1)),
                                "obj.mtl_texture_resolve_failed",
                                format!("MTL map_Kd texture could not be resolved: {error}"),
                            ),
                        },
                        None => push_warning(
                            ctx,
                            path.to_string(),
                            Some(SourceLocation::line_column(line_number, 1)),
                            "obj.mtl_texture_missing",
                            "MTL map_Kd has no texture path",
                        ),
                    }
                }
            }
            _ => {}
        }
    }

    flush_material(ctx, path, library, current);
}

fn flush_material(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    library: &mut MaterialLibrary,
    material: Option<ParsedMaterial>,
) {
    let Some(material) = material else {
        return;
    };
    if library
        .materials
        .insert(material.name.clone(), material.clone())
        .is_some()
    {
        push_warning(
            ctx,
            path.to_string(),
            None,
            "obj.mtl_duplicate_material",
            format!(
                "MTL material '{}' was redefined; using the last definition",
                material.name
            ),
        );
    }
}

fn tokenize_mtl<'line>(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    line: &'line str,
    line_number: u32,
) -> Option<Vec<&'line str>> {
    let mut tokens = Vec::new();
    for token in line.split_whitespace() {
        if token.len() > ctx.options.limits.max_token_bytes {
            push_warning(
                ctx,
                path.to_string(),
                Some(SourceLocation::line_column(line_number, 1)),
                "obj.mtl_token_limit",
                "MTL token exceeded max_token_bytes and line was ignored",
            );
            return None;
        }
        tokens.push(token);
    }
    Some(tokens)
}

fn parse_color(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    tokens: &[&str],
    line_number: u32,
    keyword: &str,
) -> Option<Color> {
    if tokens.len() < 4 {
        push_warning(
            ctx,
            path.to_string(),
            Some(SourceLocation::line_column(line_number, 1)),
            "obj.mtl_bad_color",
            format!("MTL {keyword} requires three components"),
        );
        return None;
    }
    Some(Color::linear_rgba(
        parse_f32_warning(ctx, path, tokens[1], line_number, keyword)?,
        parse_f32_warning(ctx, path, tokens[2], line_number, keyword)?,
        parse_f32_warning(ctx, path, tokens[3], line_number, keyword)?,
        1.0,
    ))
}

fn parse_scalar(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    tokens: &[&str],
    line_number: u32,
    keyword: &str,
) -> Option<f32> {
    if tokens.len() < 2 {
        push_warning(
            ctx,
            path.to_string(),
            Some(SourceLocation::line_column(line_number, 1)),
            "obj.mtl_bad_scalar",
            format!("MTL {keyword} requires one value"),
        );
        return None;
    }
    parse_f32_warning(ctx, path, tokens[1], line_number, keyword)
}

fn parse_i64(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    tokens: &[&str],
    line_number: u32,
    keyword: &str,
) -> Option<i64> {
    if tokens.len() < 2 {
        push_warning(
            ctx,
            path.to_string(),
            Some(SourceLocation::line_column(line_number, 1)),
            "obj.mtl_bad_integer",
            format!("MTL {keyword} requires one integer"),
        );
        return None;
    }
    match tokens[1].parse::<i64>() {
        Ok(value) => Some(value),
        Err(_) => {
            push_warning(
                ctx,
                path.to_string(),
                Some(SourceLocation::line_column(line_number, 1)),
                "obj.mtl_bad_integer",
                format!("invalid MTL {keyword} integer '{}'", tokens[1]),
            );
            None
        }
    }
}

fn parse_vec_metadata(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    tokens: &[&str],
    line_number: u32,
    keyword: &str,
) -> Option<String> {
    let color = parse_color(ctx, path, tokens, line_number, keyword)?;
    Some(format!("{} {} {}", color.r, color.g, color.b))
}

fn parse_f32_warning(
    ctx: &mut ImportContext<'_>,
    path: &AssetPath,
    text: &str,
    line_number: u32,
    keyword: &str,
) -> Option<f32> {
    let Ok(value) = text.parse::<f32>() else {
        push_warning(
            ctx,
            path.to_string(),
            Some(SourceLocation::line_column(line_number, 1)),
            "obj.mtl_bad_float",
            format!("invalid MTL {keyword} float '{text}'"),
        );
        return None;
    };
    if !value.is_finite() {
        push_warning(
            ctx,
            path.to_string(),
            Some(SourceLocation::line_column(line_number, 1)),
            "obj.mtl_bad_float",
            format!("MTL {keyword} float is non-finite"),
        );
        return None;
    }
    Some(value)
}

fn texture_path_from_map(tokens: &[&str]) -> Option<String> {
    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index];
        if token.starts_with('-') {
            index = index.saturating_add(1 + map_option_arity(token));
        } else {
            return Some(tokens[index..].join(" "));
        }
    }
    None
}

fn map_option_arity(option: &str) -> usize {
    match option {
        "-s" | "-o" | "-t" => 3,
        "-mm" => 2,
        "-bm" | "-boost" | "-clamp" | "-type" | "-texres" => 1,
        _ => 1,
    }
}

fn trailing_text_after_keyword<'line>(line: &'line str, keyword: &str) -> Option<&'line str> {
    let start = line.find(keyword)?.checked_add(keyword.len())?;
    let text = line.get(start..)?.trim();
    (!text.is_empty()).then_some(text)
}

pub(crate) fn push_warning(
    ctx: &mut ImportContext<'_>,
    source: String,
    location: Option<SourceLocation>,
    code: &'static str,
    message: impl Into<String>,
) {
    ctx.push_diagnostic(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: DiagnosticCode(code),
        source: Some(source),
        location,
        message: message.into(),
    });
}

#[allow(dead_code)]
fn _parse_error(
    source: impl Into<String>,
    location: Option<SourceLocation>,
    message: impl Into<String>,
) -> BaoziError {
    BaoziError::parse(source, location, message)
}
