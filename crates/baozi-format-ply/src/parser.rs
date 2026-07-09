use baozi_core::{
    Aabb, BaoziError, Color, Diagnostic, Mesh, MeshBinding, MetadataMap, MetadataValue, Node,
    PrimitiveTopology, Result, Scene, SceneBuilder, SourceLocation, Vec2, Vec3, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic,
};
use baozi_import::ImportContext;

pub(crate) fn read_ply(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = ctx.read_primary_to_end()?;
    let header = parse_header(ctx, &bytes)?;
    let parsed = parse_body(ctx, &bytes, &header)?;
    scene_from_parsed(ctx, &header, parsed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlyFormat {
    Ascii,
    BinaryLittleEndian,
    BinaryBigEndian,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

#[derive(Debug, Clone)]
enum Property {
    Scalar {
        name: String,
        ty: ScalarType,
    },
    List {
        name: String,
        count_ty: ScalarType,
        item_ty: ScalarType,
    },
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
}

#[derive(Debug, Clone)]
struct Header {
    format: PlyFormat,
    elements: Vec<Element>,
    comments: Vec<String>,
    obj_infos: Vec<String>,
    data_start: usize,
}

#[derive(Debug, Clone)]
struct ParsedPly {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    texcoords: Vec<Vec2>,
    colors: Vec<Color>,
    indices: Vec<u32>,
    face_vertex_counts: Vec<u32>,
    custom_attributes: Vec<VertexAttribute>,
}

fn parse_header(ctx: &ImportContext<'_>, bytes: &[u8]) -> Result<Header> {
    let mut offset = 0usize;
    let first = next_line(bytes, &mut offset)
        .ok_or_else(|| ply_parse_error(ctx, None, "PLY input is empty"))?;
    if trim_line(first) != b"ply" {
        return Err(ply_parse_error(
            ctx,
            Some(0),
            "PLY header must start with 'ply'",
        ));
    }

    let mut format = None;
    let mut elements = Vec::new();
    let mut current_element: Option<Element> = None;
    let mut comments = Vec::new();
    let mut obj_infos = Vec::new();

    loop {
        let line_offset = offset;
        let Some(line) = next_line(bytes, &mut offset) else {
            return Err(ply_parse_error(ctx, None, "PLY header has no end_header"));
        };
        if line.len() > ctx.limits().max_line_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_line_bytes",
            });
        }
        let line = std::str::from_utf8(trim_line(line)).map_err(|error| {
            BaoziError::parse(
                ctx.source().to_string(),
                Some(SourceLocation::byte(
                    (line_offset + error.valid_up_to()) as u64,
                )),
                "PLY header is not UTF-8",
            )
        })?;
        if line.is_empty() {
            continue;
        }
        if line == "end_header" {
            if let Some(element) = current_element.take() {
                elements.push(element);
            }
            break;
        }
        if let Some(comment) = line.strip_prefix("comment ") {
            comments.push(bounded_string(ctx, comment, "comment")?);
            continue;
        }
        if let Some(info) = line.strip_prefix("obj_info ") {
            obj_infos.push(bounded_string(ctx, info, "obj_info")?);
            continue;
        }
        let tokens = checked_tokens(ctx, line)?;
        match tokens.as_slice() {
            ["format", value, _version] => {
                format = Some(match *value {
                    "ascii" => PlyFormat::Ascii,
                    "binary_little_endian" => PlyFormat::BinaryLittleEndian,
                    "binary_big_endian" => PlyFormat::BinaryBigEndian,
                    other => {
                        return Err(ply_parse_error(
                            ctx,
                            Some(line_offset),
                            format!("unsupported PLY format '{other}'"),
                        ));
                    }
                });
            }
            ["element", name, count] => {
                if let Some(element) = current_element.take() {
                    elements.push(element);
                }
                let count = parse_count(ctx, count, line_offset)?;
                if *name == "vertex" && count > ctx.limits().max_vertices {
                    return Err(BaoziError::LimitExceeded {
                        limit: "max_vertices",
                    });
                }
                if *name == "face" && count > ctx.limits().max_faces {
                    return Err(BaoziError::LimitExceeded { limit: "max_faces" });
                }
                current_element = Some(Element {
                    name: bounded_string(ctx, name, "element name")?,
                    count,
                    properties: Vec::new(),
                });
            }
            ["property", "list", count_ty, item_ty, name] => {
                let Some(element) = current_element.as_mut() else {
                    return Err(ply_parse_error(
                        ctx,
                        Some(line_offset),
                        "PLY property appeared before an element",
                    ));
                };
                element.properties.push(Property::List {
                    name: bounded_string(ctx, name, "property name")?,
                    count_ty: parse_scalar_type(ctx, count_ty, line_offset)?,
                    item_ty: parse_scalar_type(ctx, item_ty, line_offset)?,
                });
            }
            ["property", ty, name] => {
                let Some(element) = current_element.as_mut() else {
                    return Err(ply_parse_error(
                        ctx,
                        Some(line_offset),
                        "PLY property appeared before an element",
                    ));
                };
                element.properties.push(Property::Scalar {
                    name: bounded_string(ctx, name, "property name")?,
                    ty: parse_scalar_type(ctx, ty, line_offset)?,
                });
            }
            _ => {
                return Err(ply_parse_error(
                    ctx,
                    Some(line_offset),
                    format!("unsupported PLY header line '{line}'"),
                ));
            }
        }
    }

    Ok(Header {
        format: format
            .ok_or_else(|| ply_parse_error(ctx, None, "PLY header has no format line"))?,
        elements,
        comments,
        obj_infos,
        data_start: offset,
    })
}

fn parse_body(ctx: &mut ImportContext<'_>, bytes: &[u8], header: &Header) -> Result<ParsedPly> {
    let vertex_element = header
        .elements
        .iter()
        .find(|element| element.name == "vertex");
    let Some(vertex_element) = vertex_element else {
        return Err(ply_parse_error(ctx, None, "PLY has no vertex element"));
    };
    if vertex_element.count == 0 {
        return Err(ply_parse_error(ctx, None, "PLY vertex element is empty"));
    }

    let mut parsed = ParsedPly {
        positions: Vec::with_capacity(vertex_element.count),
        normals: Vec::new(),
        texcoords: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        face_vertex_counts: Vec::new(),
        custom_attributes: custom_accumulators(ctx, vertex_element)?,
    };

    match header.format {
        PlyFormat::Ascii => parse_ascii_body(ctx, bytes, header, &mut parsed)?,
        PlyFormat::BinaryLittleEndian => parse_binary_body(ctx, bytes, header, false, &mut parsed)?,
        PlyFormat::BinaryBigEndian => parse_binary_body(ctx, bytes, header, true, &mut parsed)?,
    }

    Ok(parsed)
}

fn parse_ascii_body(
    ctx: &mut ImportContext<'_>,
    bytes: &[u8],
    header: &Header,
    parsed: &mut ParsedPly,
) -> Result<()> {
    let mut offset = header.data_start;
    for element in &header.elements {
        for row_index in 0..element.count {
            let line_offset = offset;
            let line = next_line(bytes, &mut offset).ok_or_else(|| {
                ply_parse_error(
                    ctx,
                    Some(line_offset),
                    format!("PLY ended inside element '{}'", element.name),
                )
            })?;
            if line.len() > ctx.limits().max_line_bytes {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_line_bytes",
                });
            }
            let text = std::str::from_utf8(trim_line(line)).map_err(|error| {
                BaoziError::parse(
                    ctx.source().to_string(),
                    Some(SourceLocation::byte(
                        (line_offset + error.valid_up_to()) as u64,
                    )),
                    "PLY ASCII row is not UTF-8",
                )
            })?;
            let tokens = checked_tokens(ctx, text)?;
            let mut cursor = AsciiRow::new(&tokens, line_offset);
            parse_row(ctx, element, row_index, &mut cursor, parsed)?;
            cursor.finish(ctx, &element.name)?;
        }
    }
    Ok(())
}

fn parse_binary_body(
    ctx: &mut ImportContext<'_>,
    bytes: &[u8],
    header: &Header,
    big_endian: bool,
    parsed: &mut ParsedPly,
) -> Result<()> {
    let mut cursor = BinaryCursor::new(bytes, header.data_start, big_endian);
    for element in &header.elements {
        for row_index in 0..element.count {
            parse_row(ctx, element, row_index, &mut cursor, parsed)?;
        }
    }
    Ok(())
}

fn parse_row(
    ctx: &mut ImportContext<'_>,
    element: &Element,
    row_index: usize,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly,
) -> Result<()> {
    if element.name == "vertex" {
        parse_vertex_row(ctx, element, row_index, reader, parsed)
    } else if element.name == "face" {
        parse_face_row(ctx, element, row_index, reader, parsed)
    } else {
        skip_unknown_row(ctx, element, reader)
    }
}

fn parse_vertex_row(
    ctx: &mut ImportContext<'_>,
    element: &Element,
    _row_index: usize,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly,
) -> Result<()> {
    let mut x = None;
    let mut y = None;
    let mut z = None;
    let mut nx = None;
    let mut ny = None;
    let mut nz = None;
    let mut s = None;
    let mut t = None;
    let mut red = None;
    let mut green = None;
    let mut blue = None;
    let mut alpha = None;

    for property in &element.properties {
        match property {
            Property::Scalar { name, ty } => {
                let value = reader.read_scalar(ctx, *ty)?;
                match name.as_str() {
                    "x" => x = Some(value.to_f32(ctx, "x")?),
                    "y" => y = Some(value.to_f32(ctx, "y")?),
                    "z" => z = Some(value.to_f32(ctx, "z")?),
                    "nx" => nx = Some(value.to_f32(ctx, "nx")?),
                    "ny" => ny = Some(value.to_f32(ctx, "ny")?),
                    "nz" => nz = Some(value.to_f32(ctx, "nz")?),
                    "s" | "u" | "texture_u" => s = Some(value.to_f32(ctx, name)?),
                    "t" | "v" | "texture_v" => t = Some(value.to_f32(ctx, name)?),
                    "red" | "r" => red = Some(value.to_color_component(ctx, *ty, name)?),
                    "green" | "g" => green = Some(value.to_color_component(ctx, *ty, name)?),
                    "blue" | "b" => blue = Some(value.to_color_component(ctx, *ty, name)?),
                    "alpha" | "a" => alpha = Some(value.to_color_component(ctx, *ty, name)?),
                    _ => push_custom_scalar(parsed, name, value)?,
                }
            }
            Property::List {
                name,
                count_ty,
                item_ty,
            } => {
                warn(
                    ctx,
                    "ply.vertex_list_property_ignored",
                    format!("PLY vertex list property '{name}' was ignored"),
                );
                skip_list(ctx, reader, *count_ty, *item_ty)?;
            }
        }
    }

    let position = Vec3::new(
        x.ok_or_else(|| ply_parse_error(ctx, None, "PLY vertex is missing x"))?,
        y.ok_or_else(|| ply_parse_error(ctx, None, "PLY vertex is missing y"))?,
        z.ok_or_else(|| ply_parse_error(ctx, None, "PLY vertex is missing z"))?,
    );
    if !position.is_finite() {
        return Err(ply_parse_error(
            ctx,
            None,
            "PLY vertex position contains a non-finite value",
        ));
    }
    parsed.positions.push(position);

    match (nx, ny, nz) {
        (Some(nx), Some(ny), Some(nz)) => parsed.normals.push(Vec3::new(nx, ny, nz)),
        (None, None, None) => {}
        _ => warn(
            ctx,
            "ply.partial_normals_ignored",
            "PLY vertex has partial normal properties; normals were ignored",
        ),
    }
    match (s, t) {
        (Some(s), Some(t)) => parsed.texcoords.push(Vec2::new(s, t)),
        (None, None) => {}
        _ => warn(
            ctx,
            "ply.partial_texcoords_ignored",
            "PLY vertex has partial texture coordinate properties; texcoords were ignored",
        ),
    }
    match (red, green, blue) {
        (Some(red), Some(green), Some(blue)) => {
            parsed
                .colors
                .push(Color::linear_rgba(red, green, blue, alpha.unwrap_or(1.0)))
        }
        (None, None, None) => {}
        _ => warn(
            ctx,
            "ply.partial_colors_ignored",
            "PLY vertex has partial color properties; colors were ignored",
        ),
    }
    Ok(())
}

fn parse_face_row(
    ctx: &mut ImportContext<'_>,
    element: &Element,
    _row_index: usize,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly,
) -> Result<()> {
    let mut found_vertices = false;
    for property in &element.properties {
        match property {
            Property::List {
                name,
                count_ty,
                item_ty,
            } if name == "vertex_indices" || name == "vertex_index" => {
                found_vertices = true;
                let count = read_list_count(ctx, reader, *count_ty)?;
                if count < 3 {
                    return Err(ply_parse_error(
                        ctx,
                        None,
                        "PLY face has fewer than three vertices",
                    ));
                }
                if count > ctx.limits().max_vertices {
                    return Err(BaoziError::LimitExceeded {
                        limit: "max_vertices",
                    });
                }
                parsed.face_vertex_counts.push(count as u32);
                for _ in 0..count {
                    let index = reader.read_scalar(ctx, *item_ty)?.to_u32(ctx, name)?;
                    parsed.indices.push(index);
                }
            }
            Property::List {
                name,
                count_ty,
                item_ty,
            } => {
                warn(
                    ctx,
                    "ply.face_list_property_ignored",
                    format!("PLY face list property '{name}' was ignored"),
                );
                skip_list(ctx, reader, *count_ty, *item_ty)?;
            }
            Property::Scalar { name, ty } => {
                warn(
                    ctx,
                    "ply.face_scalar_property_ignored",
                    format!("PLY face scalar property '{name}' was ignored"),
                );
                let _ = reader.read_scalar(ctx, *ty)?;
            }
        }
    }
    if !found_vertices {
        return Err(ply_parse_error(
            ctx,
            None,
            "PLY face element has no vertex_indices list property",
        ));
    }
    Ok(())
}

fn skip_unknown_row(
    ctx: &mut ImportContext<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
) -> Result<()> {
    warn(
        ctx,
        "ply.element_ignored",
        format!("PLY element '{}' was ignored", element.name),
    );
    for property in &element.properties {
        match property {
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(ctx, *ty)?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => skip_list(ctx, reader, *count_ty, *item_ty)?,
        }
    }
    Ok(())
}

fn scene_from_parsed(
    ctx: &ImportContext<'_>,
    header: &Header,
    mut parsed: ParsedPly,
) -> Result<Scene> {
    if parsed.positions.is_empty() {
        return Err(ply_parse_error(ctx, None, "PLY produced no vertices"));
    }
    if parsed.positions.len() > ctx.limits().max_vertices {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }
    let face_count = parsed.face_vertex_counts.len();
    if face_count > ctx.limits().max_faces {
        return Err(BaoziError::LimitExceeded { limit: "max_faces" });
    }
    if parsed
        .indices
        .iter()
        .any(|index| *index as usize >= parsed.positions.len())
    {
        return Err(ply_parse_error(
            ctx,
            None,
            "PLY face index references a missing vertex",
        ));
    }

    let topology = if parsed.indices.is_empty() {
        PrimitiveTopology::Points
    } else if parsed.face_vertex_counts.iter().all(|count| *count == 3) {
        parsed.face_vertex_counts.clear();
        PrimitiveTopology::Triangles
    } else {
        PrimitiveTopology::Polygons
    };
    let bounds = compute_bounds(&parsed.positions);

    let mut metadata = MetadataMap::new();
    metadata.insert(
        "ply.format".to_owned(),
        MetadataValue::String(
            match header.format {
                PlyFormat::Ascii => "ascii",
                PlyFormat::BinaryLittleEndian => "binary_little_endian",
                PlyFormat::BinaryBigEndian => "binary_big_endian",
            }
            .to_owned(),
        ),
    );
    metadata.insert(
        "ply.source".to_owned(),
        MetadataValue::String(ctx.source().to_string()),
    );
    for (index, comment) in header.comments.iter().enumerate() {
        metadata.insert(
            format!("ply.comment.{index}"),
            MetadataValue::String(comment.clone()),
        );
    }
    for (index, info) in header.obj_infos.iter().enumerate() {
        metadata.insert(
            format!("ply.obj_info.{index}"),
            MetadataValue::String(info.clone()),
        );
    }

    let mesh = Mesh {
        name: Some("PLY Mesh".to_owned()),
        topology,
        positions: parsed.positions,
        normals: optional_stream(parsed.normals),
        texcoords: optional_nested_stream(parsed.texcoords),
        colors: optional_nested_stream(parsed.colors),
        indices: parsed.indices,
        face_vertex_counts: parsed.face_vertex_counts,
        custom_attributes: parsed.custom_attributes,
        bounds,
        metadata,
        ..Mesh::default()
    };

    if ctx.limits().max_meshes == 0 {
        return Err(BaoziError::LimitExceeded {
            limit: "max_meshes",
        });
    }
    let mut builder = SceneBuilder::new();
    let mesh_id = builder.add_mesh(mesh);
    builder.add_child_node(
        builder.root(),
        Node {
            name: Some("PLY".to_owned()),
            mesh_bindings: vec![MeshBinding::new(mesh_id)],
            ..Node::default()
        },
    )?;
    builder.finish()
}

trait RowReader {
    fn read_scalar(&mut self, ctx: &ImportContext<'_>, ty: ScalarType) -> Result<ScalarValue>;
}

struct AsciiRow<'a> {
    tokens: &'a [&'a str],
    token_index: usize,
    line_offset: usize,
}

impl<'a> AsciiRow<'a> {
    fn new(tokens: &'a [&'a str], line_offset: usize) -> Self {
        Self {
            tokens,
            token_index: 0,
            line_offset,
        }
    }

    fn finish(&self, ctx: &ImportContext<'_>, element_name: &str) -> Result<()> {
        if self.token_index != self.tokens.len() {
            return Err(ply_parse_error(
                ctx,
                Some(self.line_offset),
                format!("PLY element '{element_name}' row has trailing tokens"),
            ));
        }
        Ok(())
    }
}

impl RowReader for AsciiRow<'_> {
    fn read_scalar(&mut self, ctx: &ImportContext<'_>, ty: ScalarType) -> Result<ScalarValue> {
        let token = self.tokens.get(self.token_index).ok_or_else(|| {
            ply_parse_error(
                ctx,
                Some(self.line_offset),
                "PLY ASCII row ended before all properties were read",
            )
        })?;
        self.token_index += 1;
        parse_ascii_scalar(ctx, token, ty, self.line_offset)
    }
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    big_endian: bool,
}

impl<'a> BinaryCursor<'a> {
    fn new(bytes: &'a [u8], offset: usize, big_endian: bool) -> Self {
        Self {
            bytes,
            offset,
            big_endian,
        }
    }

    fn take(&mut self, ctx: &ImportContext<'_>, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_primary_asset_bytes",
            })?;
        let slice = self.bytes.get(self.offset..end).ok_or_else(|| {
            ply_parse_error(
                ctx,
                Some(self.offset),
                "PLY binary payload ended inside a property",
            )
        })?;
        self.offset = end;
        Ok(slice)
    }
}

impl RowReader for BinaryCursor<'_> {
    fn read_scalar(&mut self, ctx: &ImportContext<'_>, ty: ScalarType) -> Result<ScalarValue> {
        let value = match ty {
            ScalarType::I8 => ScalarValue::I32(i8::from_ne_bytes([self.take(ctx, 1)?[0]]) as i32),
            ScalarType::U8 => ScalarValue::U32(self.take(ctx, 1)?[0] as u32),
            ScalarType::I16 => {
                let bytes = array2(self.take(ctx, 2)?);
                ScalarValue::I32(if self.big_endian {
                    i16::from_be_bytes(bytes)
                } else {
                    i16::from_le_bytes(bytes)
                } as i32)
            }
            ScalarType::U16 => {
                let bytes = array2(self.take(ctx, 2)?);
                ScalarValue::U32(if self.big_endian {
                    u16::from_be_bytes(bytes)
                } else {
                    u16::from_le_bytes(bytes)
                } as u32)
            }
            ScalarType::I32 => {
                let bytes = array4(self.take(ctx, 4)?);
                ScalarValue::I32(if self.big_endian {
                    i32::from_be_bytes(bytes)
                } else {
                    i32::from_le_bytes(bytes)
                })
            }
            ScalarType::U32 => {
                let bytes = array4(self.take(ctx, 4)?);
                ScalarValue::U32(if self.big_endian {
                    u32::from_be_bytes(bytes)
                } else {
                    u32::from_le_bytes(bytes)
                })
            }
            ScalarType::F32 => {
                let bytes = array4(self.take(ctx, 4)?);
                ScalarValue::F64(if self.big_endian {
                    f32::from_be_bytes(bytes)
                } else {
                    f32::from_le_bytes(bytes)
                } as f64)
            }
            ScalarType::F64 => {
                let bytes = array8(self.take(ctx, 8)?);
                ScalarValue::F64(if self.big_endian {
                    f64::from_be_bytes(bytes)
                } else {
                    f64::from_le_bytes(bytes)
                })
            }
        };
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy)]
enum ScalarValue {
    I32(i32),
    U32(u32),
    F64(f64),
}

impl ScalarValue {
    fn to_f32(self, ctx: &ImportContext<'_>, name: &str) -> Result<f32> {
        let value = match self {
            Self::I32(value) => value as f32,
            Self::U32(value) => value as f32,
            Self::F64(value) => value as f32,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(ply_parse_error(
                ctx,
                None,
                format!("PLY property '{name}' contains a non-finite value"),
            ))
        }
    }

    fn to_u32(self, ctx: &ImportContext<'_>, name: &str) -> Result<u32> {
        match self {
            Self::U32(value) => Ok(value),
            Self::I32(value) if value >= 0 => Ok(value as u32),
            Self::I32(_) | Self::F64(_) => Err(ply_parse_error(
                ctx,
                None,
                format!("PLY property '{name}' is not a valid unsigned index"),
            )),
        }
    }

    fn to_color_component(
        self,
        ctx: &ImportContext<'_>,
        ty: ScalarType,
        name: &str,
    ) -> Result<f32> {
        let value = self.to_f32(ctx, name)?;
        let normalized = match ty {
            ScalarType::U8 => value / 255.0,
            ScalarType::U16 => value / 65535.0,
            _ => value,
        };
        Ok(normalized.clamp(0.0, 1.0))
    }
}

fn read_list_count(
    ctx: &ImportContext<'_>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
) -> Result<usize> {
    let count = reader
        .read_scalar(ctx, count_ty)?
        .to_u32(ctx, "list count")? as usize;
    if count > ctx.limits().max_vertices {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }
    Ok(count)
}

fn skip_list(
    ctx: &ImportContext<'_>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
    item_ty: ScalarType,
) -> Result<()> {
    let count = read_list_count(ctx, reader, count_ty)?;
    for _ in 0..count {
        let _ = reader.read_scalar(ctx, item_ty)?;
    }
    Ok(())
}

fn custom_accumulators(
    ctx: &mut ImportContext<'_>,
    element: &Element,
) -> Result<Vec<VertexAttribute>> {
    let mut attributes = Vec::new();
    for property in &element.properties {
        let Property::Scalar { name, ty } = property else {
            continue;
        };
        if is_builtin_vertex_property(name) {
            continue;
        }
        let data = match ty {
            ScalarType::F32 | ScalarType::F64 => {
                VertexAttributeData::F32(Vec::with_capacity(element.count))
            }
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 => {
                VertexAttributeData::U32(Vec::with_capacity(element.count))
            }
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => {
                VertexAttributeData::I32(Vec::with_capacity(element.count))
            }
        };
        let attr_name = format!("ply:{name}");
        attributes.push(VertexAttribute {
            name: attr_name.clone(),
            semantic: VertexAttributeSemantic::Custom(attr_name),
            data,
            metadata: MetadataMap::new(),
        });
    }
    if attributes.len() > ctx.limits().max_token_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_token_bytes",
        });
    }
    Ok(attributes)
}

fn push_custom_scalar(parsed: &mut ParsedPly, name: &str, value: ScalarValue) -> Result<()> {
    let attr_name = format!("ply:{name}");
    let Some(attribute) = parsed
        .custom_attributes
        .iter_mut()
        .find(|attribute| attribute.name == attr_name)
    else {
        return Ok(());
    };
    match (&mut attribute.data, value) {
        (VertexAttributeData::F32(values), ScalarValue::F64(value)) => values.push(value as f32),
        (VertexAttributeData::F32(values), ScalarValue::I32(value)) => values.push(value as f32),
        (VertexAttributeData::F32(values), ScalarValue::U32(value)) => values.push(value as f32),
        (VertexAttributeData::U32(values), ScalarValue::U32(value)) => values.push(value),
        (VertexAttributeData::U32(values), ScalarValue::I32(value)) if value >= 0 => {
            values.push(value as u32)
        }
        (VertexAttributeData::I32(values), ScalarValue::I32(value)) => values.push(value),
        _ => {
            return Err(BaoziError::parse(
                "ply",
                None,
                format!("PLY custom property '{name}' value does not match its declared type"),
            ));
        }
    }
    Ok(())
}

fn is_builtin_vertex_property(name: &str) -> bool {
    matches!(
        name,
        "x" | "y"
            | "z"
            | "nx"
            | "ny"
            | "nz"
            | "s"
            | "t"
            | "u"
            | "v"
            | "texture_u"
            | "texture_v"
            | "red"
            | "green"
            | "blue"
            | "alpha"
            | "r"
            | "g"
            | "b"
            | "a"
    )
}

fn parse_ascii_scalar(
    ctx: &ImportContext<'_>,
    token: &str,
    ty: ScalarType,
    line_offset: usize,
) -> Result<ScalarValue> {
    match ty {
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => token
            .parse::<i32>()
            .map(ScalarValue::I32)
            .map_err(|_| ply_parse_error(ctx, Some(line_offset), "invalid PLY signed integer")),
        ScalarType::U8 | ScalarType::U16 | ScalarType::U32 => token
            .parse::<u32>()
            .map(ScalarValue::U32)
            .map_err(|_| ply_parse_error(ctx, Some(line_offset), "invalid PLY unsigned integer")),
        ScalarType::F32 | ScalarType::F64 => token
            .parse::<f64>()
            .map(ScalarValue::F64)
            .map_err(|_| ply_parse_error(ctx, Some(line_offset), "invalid PLY float")),
    }
}

fn parse_scalar_type(
    ctx: &ImportContext<'_>,
    token: &str,
    line_offset: usize,
) -> Result<ScalarType> {
    match token {
        "char" | "int8" => Ok(ScalarType::I8),
        "uchar" | "uint8" => Ok(ScalarType::U8),
        "short" | "int16" => Ok(ScalarType::I16),
        "ushort" | "uint16" => Ok(ScalarType::U16),
        "int" | "int32" => Ok(ScalarType::I32),
        "uint" | "uint32" => Ok(ScalarType::U32),
        "float" | "float32" => Ok(ScalarType::F32),
        "double" | "float64" => Ok(ScalarType::F64),
        other => Err(ply_parse_error(
            ctx,
            Some(line_offset),
            format!("unsupported PLY scalar type '{other}'"),
        )),
    }
}

fn parse_count(ctx: &ImportContext<'_>, token: &str, line_offset: usize) -> Result<usize> {
    token
        .parse::<usize>()
        .map_err(|_| ply_parse_error(ctx, Some(line_offset), "invalid PLY element count"))
}

fn checked_tokens<'a>(ctx: &ImportContext<'_>, line: &'a str) -> Result<Vec<&'a str>> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    for token in &tokens {
        if token.len() > ctx.limits().max_token_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_token_bytes",
            });
        }
    }
    Ok(tokens)
}

fn bounded_string(ctx: &ImportContext<'_>, value: &str, label: &str) -> Result<String> {
    if value.len() > ctx.limits().max_string_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        });
    }
    if value.is_empty() {
        return Err(ply_parse_error(
            ctx,
            None,
            format!("PLY {label} must not be empty"),
        ));
    }
    Ok(value.to_owned())
}

fn optional_stream<T>(values: Vec<T>) -> Vec<T> {
    values
}

fn optional_nested_stream<T>(values: Vec<T>) -> Vec<Vec<T>> {
    if values.is_empty() {
        Vec::new()
    } else {
        vec![values]
    }
}

fn compute_bounds(positions: &[Vec3]) -> Option<Aabb> {
    let first = *positions.first()?;
    let mut min = first;
    let mut max = first;
    for position in positions.iter().copied().skip(1) {
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    Some(Aabb { min, max })
}

fn next_line<'a>(bytes: &'a [u8], offset: &mut usize) -> Option<&'a [u8]> {
    if *offset >= bytes.len() {
        return None;
    }
    let start = *offset;
    while *offset < bytes.len() && bytes[*offset] != b'\n' {
        *offset += 1;
    }
    let end = *offset;
    if *offset < bytes.len() {
        *offset += 1;
    }
    Some(&bytes[start..end])
}

fn trim_line(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn warn(ctx: &mut ImportContext<'_>, code: &'static str, message: impl Into<String>) {
    let mut diagnostic = Diagnostic::warning(code, message);
    diagnostic.source = Some(ctx.source().to_string());
    ctx.push_diagnostic(diagnostic);
}

fn ply_parse_error(
    ctx: &ImportContext<'_>,
    byte_offset: Option<usize>,
    message: impl Into<String>,
) -> BaoziError {
    BaoziError::parse(
        ctx.source().to_string(),
        byte_offset.map(|offset| SourceLocation::byte(offset as u64)),
        message,
    )
}

fn array2(bytes: &[u8]) -> [u8; 2] {
    [bytes[0], bytes[1]]
}

fn array4(bytes: &[u8]) -> [u8; 4] {
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn array8(bytes: &[u8]) -> [u8; 8] {
    [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]
}
