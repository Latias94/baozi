use crate::allocation::{LiveStorage, decimal_digits};
use crate::{PlyFailure, PlyFailureCode};
use baozi_core::{
    Aabb, Color, Mesh, MeshBinding, MetadataMap, MetadataValue, Node, PrimitiveTopology,
    SceneDraft, Vec2, Vec3, VertexAttribute, VertexAttributeData, VertexAttributeSemantic,
};
use baozi_import::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity, ImportProgressPhase,
    ImportStorageClass, ImportWorkClass, ParseFailure, ParseResult, ParseScope,
};
use std::fmt;

const SCAN_QUANTUM: usize = 4 * 1024;
const MAX_NUMERIC_LITERAL_BYTES: usize = SCAN_QUANTUM;
const UTF8_LOOKAHEAD_BYTES: usize = 3;
const PROFILE_SCAN_CHUNK_BYTES: usize = SCAN_QUANTUM - UTF8_LOOKAHEAD_BYTES;

pub(crate) fn read_ply(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let bytes = scope.primary_bytes();
    let decode = scope.begin_work(
        ImportWorkClass::DecodeWork,
        usize_amount(scope, bytes.len(), "ply-decode-work")?,
    )?;
    let header = parse_header(scope, bytes)?;
    let decoded_utf8_bytes = match header.format {
        PlyFormat::Ascii => bytes.len(),
        PlyFormat::BinaryLittleEndian | PlyFormat::BinaryBigEndian => header.data_start,
    };
    scope.enforce_resource_limit(
        "ply-decoded-utf8-bytes",
        usize_amount(scope, decoded_utf8_bytes, "ply-decoded-utf8-bytes")?,
        scope.limits().max_decoded_utf8_bytes,
    )?;
    let parsed = parse_body(scope, bytes, &header)?;
    let draft = scene_from_parsed(scope, header, parsed)?;
    decode.complete()?;
    Ok(draft)
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
        custom_attribute: Option<usize>,
        source_offset: usize,
    },
    List {
        name: String,
        count_ty: ScalarType,
        item_ty: ScalarType,
        source_offset: usize,
    },
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
    source_offset: usize,
}

struct Header<'scope> {
    format: PlyFormat,
    elements: Vec<Element>,
    comments: Vec<String>,
    obj_infos: Vec<String>,
    data_start: usize,
    _storage: LiveStorage<'scope>,
}

struct ParsedPly<'scope> {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    texcoords: Vec<Vec2>,
    colors: Vec<Color>,
    indices: Vec<u32>,
    face_vertex_counts: Vec<u32>,
    has_polygon: bool,
    custom_attributes: Vec<VertexAttribute>,
    storage: LiveStorage<'scope>,
    face_count_storage: LiveStorage<'scope>,
}

#[derive(Debug, Clone, Copy)]
struct VertexChannels {
    normals: bool,
    texcoords: bool,
    colors: bool,
}

#[derive(Debug, Clone, Copy)]
struct VertexPropertyProfile {
    channels: VertexChannels,
    custom_scalar_count: usize,
    stream_count: usize,
}

struct ProfiledAsciiLine<'scope> {
    value: String,
    _storage: LiveStorage<'scope>,
}

impl ProfiledAsciiLine<'_> {
    fn as_str(&self) -> &str {
        &self.value
    }
}

fn parse_header<'scope>(ctx: &'scope ParseScope<'_>, bytes: &[u8]) -> ParseResult<Header<'scope>> {
    let mut offset = 0usize;
    let first = next_line(ctx, bytes, &mut offset)?.ok_or_else(|| {
        ply_parse_error(
            ctx,
            PlyFailureCode::InvalidHeader,
            None,
            "PLY input is empty",
        )
    })?;
    let first = ascii_profile_line(
        ctx,
        first,
        0,
        line_terminated_by_lf(bytes, offset),
        AsciiTextSurface::Header,
    )?;
    if first.as_str() != "ply" {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidHeader,
            Some(0),
            "PLY header must start with 'ply'",
        ));
    }
    drop(first);

    let mut format = None;
    let mut elements = Vec::new();
    let mut current_element: Option<Element> = None;
    let mut comments = Vec::new();
    let mut obj_infos = Vec::new();
    let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut saw_vertex_element = false;
    let mut saw_face_element = false;

    let mut header_line_index = 0usize;
    loop {
        poll_collection(ctx, header_line_index)?;
        header_line_index = header_line_index
            .checked_add(1)
            .ok_or_else(|| ctx.resource_limit_exceeded("ply-header-lines", u64::MAX))?;
        let line_offset = offset;
        let Some(line) = next_line(ctx, bytes, &mut offset)? else {
            return Err(ply_parse_error(
                ctx,
                PlyFailureCode::InvalidHeader,
                None,
                "PLY header has no end_header",
            ));
        };
        let line = ascii_profile_line(
            ctx,
            line,
            line_offset,
            line_terminated_by_lf(bytes, offset),
            AsciiTextSurface::Header,
        )?;
        if line.as_str().is_empty() {
            continue;
        }
        if line.as_str() == "end_header" {
            if let Some(element) = current_element.take() {
                storage.reserve_vec(ctx, &mut elements, 1, "ply-header-elements")?;
                elements.push(element);
            }
            break;
        }
        if let Some(comment) = line.as_str().strip_prefix("comment ") {
            let comment = bounded_string(ctx, &mut storage, comment, "comment")?;
            storage.reserve_vec(ctx, &mut comments, 1, "ply-header-comments")?;
            comments.push(comment);
            continue;
        }
        if let Some(info) = line.as_str().strip_prefix("obj_info ") {
            let info = bounded_string(ctx, &mut storage, info, "obj_info")?;
            storage.reserve_vec(ctx, &mut obj_infos, 1, "ply-header-obj-infos")?;
            obj_infos.push(info);
            continue;
        }
        let tokens = checked_tokens(ctx, line.as_str())?;
        match tokens.as_slice() {
            ["format", value, version] => {
                if format.is_some() {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidHeader,
                        Some(line_offset),
                        "PLY header has multiple format lines",
                    ));
                }
                if *version != "1.0" {
                    return Err(format_failure(
                        ctx,
                        PlyFailureCode::UnsupportedEncoding,
                        "unsupported PLY version",
                        Some(line_offset),
                        Some("format-version"),
                    ));
                }
                format = Some(match *value {
                    "ascii" => PlyFormat::Ascii,
                    "binary_little_endian" => PlyFormat::BinaryLittleEndian,
                    "binary_big_endian" => PlyFormat::BinaryBigEndian,
                    other => {
                        let _ = other;
                        return Err(format_failure(
                            ctx,
                            PlyFailureCode::UnsupportedEncoding,
                            "unsupported PLY encoding",
                            Some(line_offset),
                            Some("format-encoding"),
                        ));
                    }
                });
            }
            ["element", name, count] => {
                if let Some(element) = current_element.take() {
                    storage.reserve_vec(ctx, &mut elements, 1, "ply-header-elements")?;
                    elements.push(element);
                }
                let count = parse_count(ctx, count, line_offset)?;
                match *name {
                    "vertex" => {
                        if saw_vertex_element {
                            return Err(ply_parse_error(
                                ctx,
                                PlyFailureCode::InvalidSchema,
                                Some(line_offset),
                                "multiple PLY vertex elements are not supported",
                            ));
                        }
                        saw_vertex_element = true;
                        preflight_limit(
                            ctx,
                            count,
                            ctx.limits().max_vertices,
                            ImportWorkClass::Vertices,
                        )?;
                    }
                    "face" => {
                        if saw_face_element {
                            return Err(ply_parse_error(
                                ctx,
                                PlyFailureCode::InvalidSchema,
                                Some(line_offset),
                                "multiple PLY face elements are not supported",
                            ));
                        }
                        saw_face_element = true;
                        preflight_limit(
                            ctx,
                            count,
                            ctx.limits().max_faces,
                            ImportWorkClass::Faces,
                        )?;
                    }
                    _ => {}
                }
                current_element = Some(Element {
                    name: bounded_string(ctx, &mut storage, name, "element name")?,
                    count,
                    properties: Vec::new(),
                    source_offset: line_offset,
                });
            }
            ["property", "list", count_ty, item_ty, name] => {
                let Some(element) = current_element.as_mut() else {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidSchema,
                        Some(line_offset),
                        "PLY property appeared before an element",
                    ));
                };
                let property = Property::List {
                    name: bounded_string(ctx, &mut storage, name, "property name")?,
                    count_ty: parse_scalar_type(ctx, count_ty, line_offset)?,
                    item_ty: parse_scalar_type(ctx, item_ty, line_offset)?,
                    source_offset: line_offset,
                };
                storage.reserve_vec(ctx, &mut element.properties, 1, "ply-header-properties")?;
                element.properties.push(property);
            }
            ["property", ty, name] => {
                let Some(element) = current_element.as_mut() else {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidSchema,
                        Some(line_offset),
                        "PLY property appeared before an element",
                    ));
                };
                let property = Property::Scalar {
                    name: bounded_string(ctx, &mut storage, name, "property name")?,
                    ty: parse_scalar_type(ctx, ty, line_offset)?,
                    custom_attribute: None,
                    source_offset: line_offset,
                };
                storage.reserve_vec(ctx, &mut element.properties, 1, "ply-header-properties")?;
                element.properties.push(property);
            }
            _ => {
                return Err(format_failure(
                    ctx,
                    PlyFailureCode::InvalidHeader,
                    "unsupported PLY header line",
                    Some(line_offset),
                    Some("header-line"),
                ));
            }
        }
    }

    bind_custom_attribute_slots(ctx, &mut elements)?;
    validate_face_topology_schemas(ctx, &elements)?;

    let format = format.ok_or_else(|| {
        ply_parse_error(
            ctx,
            PlyFailureCode::InvalidHeader,
            None,
            "PLY header has no format line",
        )
    })?;
    if format != PlyFormat::Ascii {
        validate_binary_record_schema(ctx, &elements)?;
    }
    validate_record_budget(ctx, &elements)?;
    if format != PlyFormat::Ascii {
        validate_binary_minimum_payload(ctx, bytes, offset, &elements)?;
    }

    Ok(Header {
        format,
        elements,
        comments,
        obj_infos,
        data_start: offset,
        _storage: storage,
    })
}

fn bind_custom_attribute_slots(ctx: &ParseScope<'_>, elements: &mut [Element]) -> ParseResult<()> {
    let mut vertex = None;
    for (element_index, element) in elements.iter_mut().enumerate() {
        poll_collection(ctx, element_index)?;
        if element.name == "vertex" {
            vertex = Some(element);
            break;
        }
    }
    let Some(vertex) = vertex else {
        return Ok(());
    };

    let mut next_slot = 0usize;
    for (property_index, property) in vertex.properties.iter_mut().enumerate() {
        poll_collection(ctx, property_index)?;
        let Property::Scalar {
            name,
            custom_attribute,
            ..
        } = property
        else {
            continue;
        };
        if is_builtin_vertex_property(name) {
            continue;
        }
        *custom_attribute = Some(next_slot);
        next_slot = next_slot.checked_add(1).ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_vertex_attribute_streams_per_mesh",
                ctx.limits().max_vertex_attribute_streams_per_mesh,
            )
        })?;
    }
    Ok(())
}

fn validate_face_topology_schemas(ctx: &ParseScope<'_>, elements: &[Element]) -> ParseResult<()> {
    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        if element.name != "face" {
            continue;
        }
        let mut topology_properties = 0usize;
        let mut topology_types = None;
        for (property_index, property) in element.properties.iter().enumerate() {
            poll_collection(ctx, property_index)?;
            let Property::List {
                name,
                count_ty,
                item_ty,
                ..
            } = property
            else {
                continue;
            };
            if name != "vertex_indices" && name != "vertex_index" {
                continue;
            }
            topology_properties = topology_properties.checked_add(1).ok_or_else(|| {
                resource_limit_failure(ctx, "ply-face-topology-properties", usize::MAX)
            })?;
            topology_types.get_or_insert((*count_ty, *item_ty));
        }
        if topology_properties != 1 {
            return Err(ply_parse_error(
                ctx,
                PlyFailureCode::InvalidSchema,
                None,
                "PLY face element must declare exactly one topology list property ('vertex_indices' or 'vertex_index')",
            ));
        }
        let Some((count_ty, item_ty)) = topology_types else {
            return Err(format_failure(
                ctx,
                PlyFailureCode::InternalInvariant,
                "PLY topology schema disappeared after validation",
                None,
                Some("face-topology-schema"),
            ));
        };
        if !is_integer_scalar(count_ty) || !is_integer_scalar(item_ty) {
            return Err(format_failure(
                ctx,
                PlyFailureCode::InvalidSchema,
                "PLY face topology count and indices must use integer scalar types",
                None,
                Some("face-topology-types"),
            ));
        }
    }
    Ok(())
}

fn is_integer_scalar(ty: ScalarType) -> bool {
    !matches!(ty, ScalarType::F32 | ScalarType::F64)
}

fn validate_binary_record_schema(ctx: &ParseScope<'_>, elements: &[Element]) -> ParseResult<()> {
    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        if element.count != 0 && element.properties.is_empty() {
            return Err(format_failure(
                ctx,
                PlyFailureCode::InvalidSchema,
                "PLY binary element with records has no properties",
                None,
                Some("zero-width-binary-element"),
            ));
        }
    }
    Ok(())
}

fn validate_record_budget(ctx: &ParseScope<'_>, elements: &[Element]) -> ParseResult<()> {
    let limit = usize_amount(ctx, ctx.limits().max_records, "ply-record-limit")?;
    let mut declared_records = 0_u64;
    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        let count = usize_amount(ctx, element.count, "ply-record-count")?;
        let Some(next) = declared_records.checked_add(count) else {
            return Err(ctx.resource_limit_exceeded("max_records", limit));
        };
        declared_records = next;
    }
    ctx.enforce_resource_limit("max_records", declared_records, limit)
}

fn validate_binary_minimum_payload(
    ctx: &ParseScope<'_>,
    bytes: &[u8],
    data_start: usize,
    elements: &[Element],
) -> ParseResult<()> {
    let mut minimum_payload_bytes = 0usize;

    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        let mut minimum_record_bytes = 0usize;
        for (property_index, property) in element.properties.iter().enumerate() {
            poll_collection(ctx, property_index)?;
            let property_bytes = match property {
                Property::Scalar { ty, .. } => scalar_width(*ty),
                Property::List { count_ty, .. } => scalar_width(*count_ty),
            };
            minimum_record_bytes = minimum_record_bytes
                .checked_add(property_bytes)
                .ok_or_else(|| {
                    resource_limit_failure(ctx, "ply-binary-payload-bytes", bytes.len())
                })?;
        }
        let element_bytes = minimum_record_bytes
            .checked_mul(element.count)
            .ok_or_else(|| resource_limit_failure(ctx, "ply-binary-payload-bytes", bytes.len()))?;
        minimum_payload_bytes = minimum_payload_bytes
            .checked_add(element_bytes)
            .ok_or_else(|| resource_limit_failure(ctx, "ply-binary-payload-bytes", bytes.len()))?;
    }

    let payload_bytes = bytes.len().checked_sub(data_start).ok_or_else(|| {
        ply_parse_error(
            ctx,
            PlyFailureCode::TruncatedPayload,
            Some(data_start),
            "PLY binary payload starts outside the input",
        )
    })?;
    if minimum_payload_bytes > payload_bytes {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::TruncatedPayload,
            Some(data_start),
            "PLY binary payload is shorter than the declared element records require",
        ));
    }
    Ok(())
}

fn scalar_width(ty: ScalarType) -> usize {
    match ty {
        ScalarType::I8 | ScalarType::U8 => 1,
        ScalarType::I16 | ScalarType::U16 => 2,
        ScalarType::I32 | ScalarType::U32 | ScalarType::F32 => 4,
        ScalarType::F64 => 8,
    }
}

fn parse_body<'scope>(
    ctx: &'scope ParseScope<'_>,
    bytes: &[u8],
    header: &Header<'_>,
) -> ParseResult<ParsedPly<'scope>> {
    let mut vertex_element = None;
    let mut face_count = 0usize;
    for (element_index, element) in header.elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        match element.name.as_str() {
            "vertex" => vertex_element = Some(element),
            "face" => face_count = element.count,
            _ => {}
        }
    }
    let Some(vertex_element) = vertex_element else {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidSchema,
            None,
            "PLY has no vertex element",
        ));
    };
    if vertex_element.count == 0 {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidSchema,
            None,
            "PLY vertex element is empty",
        ));
    }
    preflight_limit(ctx, 1, ctx.limits().max_meshes, ImportWorkClass::Meshes)?;
    preflight_limit(
        ctx,
        1,
        ctx.limits().max_primitives,
        ImportWorkClass::Primitives,
    )?;

    let vertex_count = vertex_element.count;
    let vertex_profile = vertex_property_profile(ctx, vertex_element)?;
    validate_vertex_attribute_budget(ctx, vertex_count, vertex_profile.stream_count)?;
    let mut storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let mut positions = Vec::new();
    storage.reserve_vec(ctx, &mut positions, vertex_count, "ply-positions")?;
    let mut normals = Vec::new();
    let channels = vertex_profile.channels;
    if channels.normals {
        storage.reserve_vec(ctx, &mut normals, vertex_count, "ply-normals")?;
    }
    let mut texcoords = Vec::new();
    if channels.texcoords {
        storage.reserve_vec(ctx, &mut texcoords, vertex_count, "ply-texcoords")?;
    }
    let mut colors = Vec::new();
    if channels.colors {
        storage.reserve_vec(ctx, &mut colors, vertex_count, "ply-colors")?;
    }
    let mut face_count_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let mut face_vertex_counts = Vec::new();
    face_count_storage.reserve_vec(ctx, &mut face_vertex_counts, face_count, "ply-face-counts")?;
    let custom_attributes = custom_accumulators(
        ctx,
        vertex_element,
        vertex_profile.custom_scalar_count,
        &mut storage,
    )?;
    let mut parsed = ParsedPly {
        positions,
        normals,
        texcoords,
        colors,
        indices: Vec::new(),
        face_vertex_counts,
        has_polygon: false,
        custom_attributes,
        storage,
        face_count_storage,
    };

    match header.format {
        PlyFormat::Ascii => parse_ascii_body(ctx, bytes, header, &mut parsed)?,
        PlyFormat::BinaryLittleEndian => parse_binary_body(ctx, bytes, header, false, &mut parsed)?,
        PlyFormat::BinaryBigEndian => parse_binary_body(ctx, bytes, header, true, &mut parsed)?,
    }

    Ok(parsed)
}

fn parse_ascii_body<'scope>(
    ctx: &'scope ParseScope<'_>,
    bytes: &[u8],
    header: &Header<'_>,
    parsed: &mut ParsedPly<'scope>,
) -> ParseResult<()> {
    let mut offset = header.data_start;
    for (element_index, element) in header.elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        warn_for_element_schema(ctx, element)?;
        for record_index in 0..element.count {
            poll_collection(ctx, record_index)?;
            let record = ctx.begin_work(ImportWorkClass::Records, 1)?;
            let line_offset = offset;
            let line = next_line(ctx, bytes, &mut offset)?.ok_or_else(|| {
                format_failure(
                    ctx,
                    PlyFailureCode::TruncatedPayload,
                    "PLY ended inside an element",
                    Some(line_offset),
                    Some("ascii-element-row"),
                )
            })?;
            let text = ascii_profile_line(
                ctx,
                line,
                line_offset,
                line_terminated_by_lf(bytes, offset),
                AsciiTextSurface::Body,
            )?;
            let mut cursor = AsciiRow::new(text.as_str(), line_offset);
            parse_row(ctx, element, &mut cursor, parsed)?;
            cursor.finish(ctx, &element.name)?;
            record.complete()?;
        }
    }
    Ok(())
}

fn parse_binary_body<'scope>(
    ctx: &'scope ParseScope<'_>,
    bytes: &[u8],
    header: &Header<'_>,
    big_endian: bool,
    parsed: &mut ParsedPly<'scope>,
) -> ParseResult<()> {
    let mut cursor = BinaryCursor::new(bytes, header.data_start, big_endian);
    for (element_index, element) in header.elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        warn_for_element_schema(ctx, element)?;
        for record_index in 0..element.count {
            poll_collection(ctx, record_index)?;
            let record = ctx.begin_work(ImportWorkClass::Records, 1)?;
            parse_row(ctx, element, &mut cursor, parsed)?;
            record.complete()?;
        }
    }
    Ok(())
}

fn parse_row<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'scope>,
) -> ParseResult<()> {
    if element.name == "vertex" {
        let vertex = ctx.begin_work(ImportWorkClass::Vertices, 1)?;
        parse_vertex_row(ctx, element, reader, parsed)?;
        vertex.complete()
    } else if element.name == "face" {
        let face = ctx.begin_work(ImportWorkClass::Faces, 1)?;
        parse_face_row(ctx, element, reader, parsed)?;
        face.complete()
    } else {
        skip_unknown_row(ctx, element, reader)
    }
}

fn warn_for_element_schema(ctx: &ParseScope<'_>, element: &Element) -> ParseResult<()> {
    if element.count == 0 {
        return Ok(());
    }

    match element.name.as_str() {
        "vertex" => warn_for_vertex_schema(ctx, element),
        "face" => warn_for_face_schema(ctx, element),
        _ => warn(
            ctx,
            "ply.element_ignored",
            Some(element.source_offset),
            format_args!("PLY element '{}' was ignored", element.name),
        ),
    }
}

fn warn_for_vertex_schema(ctx: &ParseScope<'_>, element: &Element) -> ParseResult<()> {
    let mut has_nx = false;
    let mut has_ny = false;
    let mut has_nz = false;
    let mut has_s = false;
    let mut has_t = false;
    let mut has_red = false;
    let mut has_green = false;
    let mut has_blue = false;

    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar { name, .. } => match name.as_str() {
                "nx" => has_nx = true,
                "ny" => has_ny = true,
                "nz" => has_nz = true,
                "s" | "u" | "texture_u" => has_s = true,
                "t" | "v" | "texture_v" => has_t = true,
                "red" | "r" => has_red = true,
                "green" | "g" => has_green = true,
                "blue" | "b" => has_blue = true,
                _ => {}
            },
            Property::List {
                name,
                source_offset,
                ..
            } => warn(
                ctx,
                "ply.vertex_list_property_ignored",
                Some(*source_offset),
                format_args!("PLY vertex list property '{name}' was ignored"),
            )?,
        }
    }

    let normal_components = usize::from(has_nx) + usize::from(has_ny) + usize::from(has_nz);
    if normal_components != 0 && normal_components != 3 {
        warn(
            ctx,
            "ply.partial_normals_ignored",
            Some(element.source_offset),
            format_args!("PLY vertex has partial normal properties; normals were ignored"),
        )?;
    }
    if has_s != has_t {
        warn(
            ctx,
            "ply.partial_texcoords_ignored",
            Some(element.source_offset),
            format_args!(
                "PLY vertex has partial texture coordinate properties; texcoords were ignored"
            ),
        )?;
    }
    let color_components = usize::from(has_red) + usize::from(has_green) + usize::from(has_blue);
    if color_components != 0 && color_components != 3 {
        warn(
            ctx,
            "ply.partial_colors_ignored",
            Some(element.source_offset),
            format_args!("PLY vertex has partial color properties; colors were ignored"),
        )?;
    }
    Ok(())
}

fn vertex_property_profile(
    ctx: &ParseScope<'_>,
    element: &Element,
) -> ParseResult<VertexPropertyProfile> {
    let mut has_nx = false;
    let mut has_ny = false;
    let mut has_nz = false;
    let mut has_s = false;
    let mut has_t = false;
    let mut has_red = false;
    let mut has_green = false;
    let mut has_blue = false;
    let mut custom_scalar_count = 0usize;

    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        let Property::Scalar {
            name,
            custom_attribute,
            ..
        } = property
        else {
            continue;
        };
        match name.as_str() {
            "nx" => has_nx = true,
            "ny" => has_ny = true,
            "nz" => has_nz = true,
            "s" | "u" | "texture_u" => has_s = true,
            "t" | "v" | "texture_v" => has_t = true,
            "red" | "r" => has_red = true,
            "green" | "g" => has_green = true,
            "blue" | "b" => has_blue = true,
            _ => {}
        }
        if custom_attribute.is_some() {
            custom_scalar_count = custom_scalar_count.checked_add(1).ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_vertex_attribute_streams_per_mesh",
                    ctx.limits().max_vertex_attribute_streams_per_mesh,
                )
            })?;
        }
    }

    let channels = VertexChannels {
        normals: has_nx && has_ny && has_nz,
        texcoords: has_s && has_t,
        colors: has_red && has_green && has_blue,
    };
    let builtin_streams = 1usize
        .checked_add(usize::from(channels.normals))
        .and_then(|count| count.checked_add(usize::from(channels.texcoords)))
        .and_then(|count| count.checked_add(usize::from(channels.colors)))
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_vertex_attribute_streams_per_mesh",
                ctx.limits().max_vertex_attribute_streams_per_mesh,
            )
        })?;
    let stream_count = builtin_streams
        .checked_add(custom_scalar_count)
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_vertex_attribute_streams_per_mesh",
                ctx.limits().max_vertex_attribute_streams_per_mesh,
            )
        })?;
    Ok(VertexPropertyProfile {
        channels,
        custom_scalar_count,
        stream_count,
    })
}

fn warn_for_face_schema(ctx: &ParseScope<'_>, element: &Element) -> ParseResult<()> {
    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::List { name, .. } if name == "vertex_indices" || name == "vertex_index" => {}
            Property::List {
                name,
                source_offset,
                ..
            } => warn(
                ctx,
                "ply.face_list_property_ignored",
                Some(*source_offset),
                format_args!("PLY face list property '{name}' was ignored"),
            )?,
            Property::Scalar {
                name,
                source_offset,
                ..
            } => warn(
                ctx,
                "ply.face_scalar_property_ignored",
                Some(*source_offset),
                format_args!("PLY face scalar property '{name}' was ignored"),
            )?,
        }
    }
    Ok(())
}

fn parse_vertex_row<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'scope>,
) -> ParseResult<()> {
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

    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar {
                name,
                ty,
                custom_attribute,
                ..
            } => {
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
                    _ => {
                        let Some(attribute_index) = *custom_attribute else {
                            return Err(format_failure(
                                ctx,
                                PlyFailureCode::InternalInvariant,
                                "PLY custom property has no bound attribute slot",
                                None,
                                Some("custom-attribute-slot"),
                            ));
                        };
                        push_custom_scalar(ctx, parsed, attribute_index, name, value)?;
                    }
                }
            }
            Property::List {
                count_ty, item_ty, ..
            } => {
                skip_list(ctx, reader, *count_ty, *item_ty)?;
            }
        }
    }

    let position = Vec3::new(
        x.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing x"))?,
        y.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing y"))?,
        z.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing z"))?,
    );
    if !position.is_finite() {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidScalar,
            None,
            "PLY vertex position contains a non-finite value",
        ));
    }
    parsed.positions.push(position);

    match (nx, ny, nz) {
        (Some(nx), Some(ny), Some(nz)) => parsed.normals.push(Vec3::new(nx, ny, nz)),
        (None, None, None) => {}
        _ => {}
    }
    match (s, t) {
        (Some(s), Some(t)) => parsed.texcoords.push(Vec2::new(s, t)),
        (None, None) => {}
        _ => {}
    }
    match (red, green, blue) {
        (Some(red), Some(green), Some(blue)) => {
            parsed
                .colors
                .push(Color::linear_rgba(red, green, blue, alpha.unwrap_or(1.0)))
        }
        (None, None, None) => {}
        _ => {}
    }
    Ok(())
}

fn parse_face_row<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'scope>,
) -> ParseResult<()> {
    let mut found_vertices = false;
    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::List {
                name,
                count_ty,
                item_ty,
                ..
            } if name == "vertex_indices" || name == "vertex_index" => {
                found_vertices = true;
                let count = read_list_count(ctx, reader, *count_ty)?;
                if count < 3 {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidTopology,
                        None,
                        "PLY face has fewer than three vertices",
                    ));
                }
                preflight_limit(
                    ctx,
                    count,
                    ctx.limits().max_vertices,
                    ImportWorkClass::Vertices,
                )?;
                let next_indices = parsed.indices.len().checked_add(count).ok_or_else(|| {
                    resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        ctx.limits().max_total_topology_cells,
                    )
                })?;
                let next_face_counts =
                    parsed
                        .face_vertex_counts
                        .len()
                        .checked_add(1)
                        .ok_or_else(|| {
                            resource_limit_failure(ctx, "max_faces", ctx.limits().max_faces)
                        })?;
                let was_polygon = parsed.has_polygon;
                let has_polygon = was_polygon || count != 3;
                let canonical_cells = if has_polygon {
                    next_indices.checked_add(next_face_counts).ok_or_else(|| {
                        resource_limit_failure(
                            ctx,
                            "max_total_topology_cells",
                            ctx.limits().max_total_topology_cells,
                        )
                    })?
                } else {
                    next_indices
                };
                preflight_limit(
                    ctx,
                    canonical_cells,
                    ctx.limits().max_total_topology_cells,
                    ImportWorkClass::TopologyCells,
                )?;
                parsed.face_count_storage.reserve_vec(
                    ctx,
                    &mut parsed.face_vertex_counts,
                    1,
                    "ply-face-counts",
                )?;
                parsed
                    .storage
                    .reserve_vec(ctx, &mut parsed.indices, count, "ply-indices")?;
                let face_count_cells = if !was_polygon && count != 3 {
                    next_face_counts
                } else if was_polygon {
                    1
                } else {
                    0
                };
                let charged_cells = count.checked_add(face_count_cells).ok_or_else(|| {
                    resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        ctx.limits().max_total_topology_cells,
                    )
                })?;
                let topology = ctx.begin_work(
                    ImportWorkClass::TopologyCells,
                    usize_amount(ctx, charged_cells, "ply-topology-cells")?,
                )?;
                parsed.face_vertex_counts.push(count as u32);
                for position in 0..count {
                    if position.is_multiple_of(4096) {
                        ctx.poll()?;
                    }
                    let index = reader.read_scalar(ctx, *item_ty)?.to_u32(ctx, name)?;
                    parsed.indices.push(index);
                }
                parsed.has_polygon = has_polygon;
                topology.complete()?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => {
                skip_list(ctx, reader, *count_ty, *item_ty)?;
            }
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(ctx, *ty)?;
            }
        }
    }
    if !found_vertices {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidSchema,
            None,
            "PLY face element has no vertex_indices list property",
        ));
    }
    Ok(())
}

fn skip_unknown_row(
    ctx: &ParseScope<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
) -> ParseResult<()> {
    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
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

fn scene_from_parsed<'scope>(
    ctx: &'scope ParseScope<'_>,
    header: Header<'scope>,
    parsed: ParsedPly<'scope>,
) -> ParseResult<SceneDraft> {
    preflight_limit(
        ctx,
        2,
        ctx.limits().max_graph_nodes,
        ImportWorkClass::GraphNodes,
    )?;
    preflight_limit(
        ctx,
        1,
        ctx.limits().max_graph_edges,
        ImportWorkClass::GraphEdges,
    )?;
    preflight_limit(
        ctx,
        2,
        ctx.limits().max_graph_depth,
        ImportWorkClass::GraphDepth,
    )?;
    if parsed.positions.is_empty() {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidTopology,
            None,
            "PLY produced no vertices",
        ));
    }
    preflight_limit(
        ctx,
        parsed.positions.len(),
        ctx.limits().max_vertices,
        ImportWorkClass::Vertices,
    )?;
    let face_count = parsed.face_vertex_counts.len();
    preflight_limit(
        ctx,
        face_count,
        ctx.limits().max_faces,
        ImportWorkClass::Faces,
    )?;
    let index_work = ctx.begin_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, parsed.indices.len(), "ply-index-validation-work")?,
    )?;
    for (position, index) in parsed.indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            ctx.poll()?;
        }
        if *index as usize >= parsed.positions.len() {
            return Err(ply_parse_error(
                ctx,
                PlyFailureCode::InvalidTopology,
                None,
                "PLY face index references a missing vertex",
            ));
        }
    }
    index_work.complete()?;

    let keep_face_counts = parsed.has_polygon;
    let topology = if parsed.indices.is_empty() {
        PrimitiveTopology::Points
    } else if !keep_face_counts {
        PrimitiveTopology::Triangles
    } else {
        PrimitiveTopology::Polygons
    };
    let topology_cells = parsed
        .indices
        .len()
        .checked_add(if keep_face_counts {
            parsed.face_vertex_counts.len()
        } else {
            0
        })
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_total_topology_cells",
                ctx.limits().max_total_topology_cells,
            )
        })?;
    preflight_limit(
        ctx,
        topology_cells,
        ctx.limits().max_total_topology_cells,
        ImportWorkClass::TopologyCells,
    )?;
    let bounds = compute_bounds(ctx, &parsed.positions)?;

    let canonical_string_bytes = canonical_string_bytes(ctx, &header)?;
    let canonical_strings = ctx.begin_work(
        ImportWorkClass::CanonicalStringBytes,
        usize_amount(ctx, canonical_string_bytes, "ply-canonical-string-bytes")?,
    )?;
    let mut mesh_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let mut metadata = MetadataMap::new();
    insert_string_metadata(
        ctx,
        &mut mesh_storage,
        &mut metadata,
        "ply:format",
        match header.format {
            PlyFormat::Ascii => "ascii",
            PlyFormat::BinaryLittleEndian => "binary_little_endian",
            PlyFormat::BinaryBigEndian => "binary_big_endian",
        },
    )?;
    insert_string_metadata(
        ctx,
        &mut mesh_storage,
        &mut metadata,
        "ply:source",
        ctx.source().as_str(),
    )?;
    for (index, comment) in header.comments.iter().enumerate() {
        poll_collection(ctx, index)?;
        insert_indexed_string_metadata(
            ctx,
            &mut mesh_storage,
            &mut metadata,
            "ply:comment.",
            index,
            comment,
        )?;
    }
    for (index, info) in header.obj_infos.iter().enumerate() {
        poll_collection(ctx, index)?;
        insert_indexed_string_metadata(
            ctx,
            &mut mesh_storage,
            &mut metadata,
            "ply:obj_info.",
            index,
            info,
        )?;
    }
    let mesh_name = mesh_storage.copy_string(ctx, "PLY Mesh", "ply-mesh-name")?;
    canonical_strings.complete()?;
    drop(header);

    let ParsedPly {
        positions,
        normals,
        texcoords,
        colors,
        indices,
        face_vertex_counts,
        custom_attributes,
        storage,
        face_count_storage,
        ..
    } = parsed;
    let face_vertex_counts = if keep_face_counts {
        face_vertex_counts
    } else {
        Vec::new()
    };
    let face_count_storage = keep_face_counts.then_some(face_count_storage);
    let texcoords =
        optional_nested_stream(ctx, &mut mesh_storage, texcoords, "ply-texcoord-streams")?;
    let colors = optional_nested_stream(ctx, &mut mesh_storage, colors, "ply-color-streams")?;
    let mesh = Mesh {
        name: Some(mesh_name),
        topology,
        positions,
        normals: optional_stream(normals),
        texcoords,
        colors,
        indices,
        face_vertex_counts,
        custom_attributes,
        bounds,
        metadata,
        ..Mesh::default()
    };

    let mesh_permit = ctx.begin_work(ImportWorkClass::Meshes, 1)?;
    let primitive_permit = ctx.begin_work(ImportWorkClass::Primitives, 1)?;
    let mut builder = ctx.scene_builder()?;
    storage.adopt(&mut builder)?;
    if let Some(face_count_storage) = face_count_storage {
        face_count_storage.adopt(&mut builder)?;
    }
    mesh_storage.adopt(&mut builder)?;
    let mesh_id = builder.add_mesh(mesh)?;
    let mut node_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let node_name = node_storage.copy_string(ctx, "PLY", "ply-node-name")?;
    let mut mesh_bindings = Vec::new();
    node_storage.reserve_vec(ctx, &mut mesh_bindings, 1, "ply-mesh-bindings")?;
    mesh_bindings.push(MeshBinding::new(mesh_id));
    let graph_nodes = ctx.begin_work(ImportWorkClass::GraphNodes, 2)?;
    let graph_edges = ctx.begin_work(ImportWorkClass::GraphEdges, 1)?;
    let graph_depth = ctx.begin_work(ImportWorkClass::GraphDepth, 2)?;
    node_storage.adopt(&mut builder)?;
    builder.add_child_node(
        builder.root(),
        Node {
            name: Some(node_name),
            mesh_bindings,
            ..Node::default()
        },
    )?;
    graph_depth.complete()?;
    graph_edges.complete()?;
    graph_nodes.complete()?;
    primitive_permit.complete()?;
    mesh_permit.complete()?;
    builder.finish_draft()
}

trait RowReader {
    fn read_scalar(&mut self, ctx: &ParseScope<'_>, ty: ScalarType) -> ParseResult<ScalarValue>;
}

struct AsciiRow<'line> {
    line: &'line str,
    byte_index: usize,
    line_offset: usize,
}

impl<'line> AsciiRow<'line> {
    fn new(line: &'line str, line_offset: usize) -> Self {
        Self {
            line,
            byte_index: 0,
            line_offset,
        }
    }

    fn finish(&mut self, ctx: &ParseScope<'_>, element_name: &str) -> ParseResult<()> {
        if next_checked_token(ctx, self.line, &mut self.byte_index)?.is_some() {
            let _ = element_name;
            return Err(format_failure(
                ctx,
                PlyFailureCode::InvalidRecord,
                "PLY row has trailing tokens",
                Some(self.line_offset),
                Some("ascii-row"),
            ));
        }
        Ok(())
    }
}

impl RowReader for AsciiRow<'_> {
    fn read_scalar(&mut self, ctx: &ParseScope<'_>, ty: ScalarType) -> ParseResult<ScalarValue> {
        let token = next_checked_token(ctx, self.line, &mut self.byte_index)?.ok_or_else(|| {
            ply_parse_error(
                ctx,
                PlyFailureCode::TruncatedPayload,
                Some(self.line_offset),
                "PLY ASCII row ended before all properties were read",
            )
        })?;
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

    fn take(&mut self, ctx: &ParseScope<'_>, len: usize) -> ParseResult<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or_else(|| {
            format_failure(
                ctx,
                PlyFailureCode::TruncatedPayload,
                "PLY binary payload offset overflowed",
                Some(self.offset),
                Some("binary-offset"),
            )
        })?;
        let slice = self.bytes.get(self.offset..end).ok_or_else(|| {
            ply_parse_error(
                ctx,
                PlyFailureCode::TruncatedPayload,
                Some(self.offset),
                "PLY binary payload ended inside a property",
            )
        })?;
        self.offset = end;
        Ok(slice)
    }
}

impl RowReader for BinaryCursor<'_> {
    fn read_scalar(&mut self, ctx: &ParseScope<'_>, ty: ScalarType) -> ParseResult<ScalarValue> {
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
    fn to_f32(self, ctx: &ParseScope<'_>, name: &str) -> ParseResult<f32> {
        let value = match self {
            Self::I32(value) => value as f32,
            Self::U32(value) => value as f32,
            Self::F64(value) => value as f32,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            let _ = name;
            Err(format_failure(
                ctx,
                PlyFailureCode::InvalidScalar,
                "PLY property contains a non-finite value",
                None,
                Some("scalar"),
            ))
        }
    }

    fn to_u32(self, ctx: &ParseScope<'_>, name: &str) -> ParseResult<u32> {
        match self {
            Self::U32(value) => Ok(value),
            Self::I32(value) if value >= 0 => Ok(value as u32),
            Self::I32(_) | Self::F64(_) => {
                let _ = name;
                Err(format_failure(
                    ctx,
                    PlyFailureCode::InvalidScalar,
                    "PLY property is not a valid unsigned index",
                    None,
                    Some("unsigned-index"),
                ))
            }
        }
    }

    fn to_color_component(
        self,
        ctx: &ParseScope<'_>,
        ty: ScalarType,
        name: &str,
    ) -> ParseResult<f32> {
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
    ctx: &ParseScope<'_>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
) -> ParseResult<usize> {
    let count = reader
        .read_scalar(ctx, count_ty)?
        .to_u32(ctx, "list count")? as usize;
    preflight_limit(
        ctx,
        count,
        ctx.limits().max_vertices,
        ImportWorkClass::Vertices,
    )?;
    Ok(count)
}

fn skip_list(
    ctx: &ParseScope<'_>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
    item_ty: ScalarType,
) -> ParseResult<()> {
    let count = read_list_count(ctx, reader, count_ty)?;
    let work = ctx.begin_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, count, "ply-list-work")?,
    )?;
    for index in 0..count {
        if index.is_multiple_of(4096) {
            ctx.poll()?;
        }
        let _ = reader.read_scalar(ctx, item_ty)?;
    }
    work.complete()?;
    Ok(())
}

fn custom_accumulators<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element,
    custom_scalar_count: usize,
    storage: &mut LiveStorage<'scope>,
) -> ParseResult<Vec<VertexAttribute>> {
    let mut attributes = Vec::new();
    storage.reserve_vec(
        ctx,
        &mut attributes,
        custom_scalar_count,
        "ply-custom-attributes",
    )?;
    for (property_index, property) in element.properties.iter().enumerate() {
        poll_collection(ctx, property_index)?;
        let Property::Scalar {
            name,
            ty,
            custom_attribute: Some(attribute_index),
            ..
        } = property
        else {
            continue;
        };
        if *attribute_index != attributes.len() {
            return Err(format_failure(
                ctx,
                PlyFailureCode::InternalInvariant,
                "PLY custom property has an invalid attribute slot",
                None,
                Some("custom-attribute-slot"),
            ));
        }
        if is_builtin_vertex_property(name) {
            continue;
        }
        let data = match ty {
            ScalarType::F32 | ScalarType::F64 => {
                let mut values = Vec::new();
                storage.reserve_vec(ctx, &mut values, element.count, "ply-custom-f32-values")?;
                VertexAttributeData::F32(values)
            }
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 => {
                let mut values = Vec::new();
                storage.reserve_vec(ctx, &mut values, element.count, "ply-custom-u32-values")?;
                VertexAttributeData::U32(values)
            }
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => {
                let mut values = Vec::new();
                storage.reserve_vec(ctx, &mut values, element.count, "ply-custom-i32-values")?;
                VertexAttributeData::I32(values)
            }
        };
        let attr_name_bytes = "ply:".len().checked_add(name.len()).ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                ctx.limits().max_canonical_string_bytes,
            )
        })?;
        let canonical_bytes = attr_name_bytes.checked_mul(2).ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                ctx.limits().max_canonical_string_bytes,
            )
        })?;
        let canonical_strings = ctx.begin_work(
            ImportWorkClass::CanonicalStringBytes,
            usize_amount(ctx, canonical_bytes, "ply-custom-attribute-name")?,
        )?;
        let attr_name =
            storage.copy_prefixed_string(ctx, "ply:", name, "ply-custom-attribute-name")?;
        let semantic_name =
            storage.copy_prefixed_string(ctx, "ply:", name, "ply-custom-attribute-semantic")?;
        attributes.push(VertexAttribute {
            name: attr_name,
            semantic: VertexAttributeSemantic::Custom(semantic_name),
            data,
            metadata: MetadataMap::new(),
        });
        canonical_strings.complete()?;
    }
    Ok(attributes)
}

fn validate_vertex_attribute_budget(
    ctx: &ParseScope<'_>,
    vertex_count: usize,
    stream_count: usize,
) -> ParseResult<()> {
    ctx.enforce_resource_limit(
        "max_vertex_attribute_streams_per_mesh",
        usize_amount(ctx, stream_count, "ply-attribute-stream-count")?,
        usize_amount(
            ctx,
            ctx.limits().max_vertex_attribute_streams_per_mesh,
            "ply-attribute-stream-limit",
        )?,
    )?;
    let streams = ctx.begin_work(
        ImportWorkClass::AttributeStreams,
        usize_amount(ctx, stream_count, "ply-attribute-stream-count")?,
    )?;
    let cells = stream_count.checked_mul(vertex_count).ok_or_else(|| {
        resource_limit_failure(
            ctx,
            "max_total_vertex_attribute_cells",
            ctx.limits().max_total_vertex_attribute_cells,
        )
    })?;
    let cells = ctx.begin_work(
        ImportWorkClass::AttributeCells,
        usize_amount(ctx, cells, "ply-attribute-cell-count")?,
    )?;
    streams.complete()?;
    cells.complete()?;
    Ok(())
}

fn push_custom_scalar<'scope>(
    ctx: &'scope ParseScope<'_>,
    parsed: &mut ParsedPly<'scope>,
    attribute_index: usize,
    name: &str,
    value: ScalarValue,
) -> ParseResult<()> {
    let Some(attribute) = parsed.custom_attributes.get_mut(attribute_index) else {
        let _ = name;
        return Err(format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY custom property references a missing attribute slot",
            None,
            Some("custom-attribute-slot"),
        ));
    };
    match (&mut attribute.data, value) {
        (VertexAttributeData::F32(values), value) => values.push(value.to_f32(ctx, name)?),
        (VertexAttributeData::U32(values), ScalarValue::U32(value)) => values.push(value),
        (VertexAttributeData::U32(values), ScalarValue::I32(value)) if value >= 0 => {
            values.push(value as u32)
        }
        (VertexAttributeData::I32(values), ScalarValue::I32(value)) => values.push(value),
        _ => {
            let _ = name;
            return Err(format_failure(
                ctx,
                PlyFailureCode::InternalInvariant,
                "PLY custom property value does not match its declared type",
                None,
                Some("custom-attribute-type"),
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
    ctx: &ParseScope<'_>,
    token: &str,
    ty: ScalarType,
    line_offset: usize,
) -> ParseResult<ScalarValue> {
    match ty {
        ScalarType::I8 => parse_ascii_number::<i8>(ctx, token, line_offset, "signed-integer")
            .map(|value| ScalarValue::I32(value as i32)),
        ScalarType::I16 => parse_ascii_number::<i16>(ctx, token, line_offset, "signed-integer")
            .map(|value| ScalarValue::I32(value as i32)),
        ScalarType::I32 => parse_ascii_number::<i32>(ctx, token, line_offset, "signed-integer")
            .map(ScalarValue::I32),
        ScalarType::U8 => parse_ascii_number::<u8>(ctx, token, line_offset, "unsigned-integer")
            .map(|value| ScalarValue::U32(value as u32)),
        ScalarType::U16 => parse_ascii_number::<u16>(ctx, token, line_offset, "unsigned-integer")
            .map(|value| ScalarValue::U32(value as u32)),
        ScalarType::U32 => parse_ascii_number::<u32>(ctx, token, line_offset, "unsigned-integer")
            .map(ScalarValue::U32),
        ScalarType::F32 => parse_ascii_number::<f32>(ctx, token, line_offset, "float")
            .map(|value| ScalarValue::F64(value as f64)),
        ScalarType::F64 => {
            parse_ascii_number::<f64>(ctx, token, line_offset, "float").map(ScalarValue::F64)
        }
    }
}

fn parse_ascii_number<T>(
    ctx: &ParseScope<'_>,
    token: &str,
    line_offset: usize,
    context: &'static str,
) -> ParseResult<T>
where
    T: std::str::FromStr,
{
    if token.len() > MAX_NUMERIC_LITERAL_BYTES {
        return Err(format_failure(
            ctx,
            PlyFailureCode::InvalidScalar,
            "invalid PLY scalar value",
            Some(line_offset),
            Some(context),
        ));
    }
    ctx.poll()?;
    token.parse::<T>().map_err(|_| {
        format_failure(
            ctx,
            PlyFailureCode::InvalidScalar,
            "invalid PLY scalar value",
            Some(line_offset),
            Some(context),
        )
    })
}

fn parse_scalar_type(
    ctx: &ParseScope<'_>,
    token: &str,
    line_offset: usize,
) -> ParseResult<ScalarType> {
    match token {
        "char" | "int8" => Ok(ScalarType::I8),
        "uchar" | "uint8" => Ok(ScalarType::U8),
        "short" | "int16" => Ok(ScalarType::I16),
        "ushort" | "uint16" => Ok(ScalarType::U16),
        "int" | "int32" => Ok(ScalarType::I32),
        "uint" | "uint32" => Ok(ScalarType::U32),
        "float" | "float32" => Ok(ScalarType::F32),
        "double" | "float64" => Ok(ScalarType::F64),
        other => {
            let _ = other;
            Err(format_failure(
                ctx,
                PlyFailureCode::InvalidSchema,
                "unsupported PLY scalar type",
                Some(line_offset),
                Some("scalar-type"),
            ))
        }
    }
}

fn parse_count(ctx: &ParseScope<'_>, token: &str, line_offset: usize) -> ParseResult<usize> {
    if token.len() > MAX_NUMERIC_LITERAL_BYTES {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidSchema,
            Some(line_offset),
            "invalid PLY element count",
        ));
    }
    ctx.poll()?;
    token.parse::<usize>().map_err(|_| {
        ply_parse_error(
            ctx,
            PlyFailureCode::InvalidSchema,
            Some(line_offset),
            "invalid PLY element count",
        )
    })
}

struct CheckedTokens<'line, 'scope> {
    values: Vec<&'line str>,
    _storage: LiveStorage<'scope>,
}

impl<'line> CheckedTokens<'line, '_> {
    fn as_slice(&self) -> &[&'line str] {
        &self.values
    }
}

fn checked_tokens<'line, 'scope>(
    ctx: &'scope ParseScope<'_>,
    line: &'line str,
) -> ParseResult<CheckedTokens<'line, 'scope>> {
    let mut storage = LiveStorage::new(ImportStorageClass::BackendDecode);
    let mut values = Vec::new();
    let mut byte_index = 0usize;
    while let Some(token) = next_checked_token(ctx, line, &mut byte_index)? {
        storage.reserve_vec(ctx, &mut values, 1, "ply-tokens")?;
        values.push(token);
    }
    Ok(CheckedTokens {
        values,
        _storage: storage,
    })
}

fn next_checked_token<'line>(
    ctx: &ParseScope<'_>,
    line: &'line str,
    byte_index: &mut usize,
) -> ParseResult<Option<&'line str>> {
    let bytes = line.as_bytes();
    if *byte_index == 0 {
        ctx.poll()?;
    }
    while bytes
        .get(*byte_index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *byte_index += 1;
        if (*byte_index).is_multiple_of(SCAN_QUANTUM) {
            ctx.poll()?;
        }
    }
    if *byte_index == bytes.len() {
        return Ok(None);
    }

    let token_start = *byte_index;
    while bytes
        .get(*byte_index)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        *byte_index += 1;
        if (*byte_index).is_multiple_of(SCAN_QUANTUM) {
            ctx.poll()?;
        }
    }
    let token = line.get(token_start..*byte_index).ok_or_else(|| {
        format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY token range is not a valid ASCII boundary",
            None,
            Some("ascii-token-range"),
        )
    })?;
    let token_permit = ctx.begin_work(ImportWorkClass::Tokens, 1)?;
    let byte_permit = ctx.begin_work(
        ImportWorkClass::TokenBytes,
        usize_amount(ctx, token.len(), "ply-token-bytes")?,
    )?;
    byte_permit.complete()?;
    token_permit.complete()?;
    Ok(Some(token))
}

fn bounded_string<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    value: &str,
    label: &'static str,
) -> ParseResult<String> {
    let permit = ctx.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(ctx, value.len(), "ply-string-bytes")?,
    )?;
    if value.is_empty() {
        return Err(format_failure(
            ctx,
            PlyFailureCode::InvalidSchema,
            "PLY named header value must not be empty",
            None,
            Some(label),
        ));
    }
    let value = storage.copy_string(ctx, value, "ply-header-string")?;
    permit.complete()?;
    Ok(value)
}

fn optional_stream<T>(values: Vec<T>) -> Vec<T> {
    values
}

fn optional_nested_stream<'scope, T>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    values: Vec<T>,
    resource: &'static str,
) -> ParseResult<Vec<Vec<T>>> {
    if values.is_empty() {
        Ok(Vec::new())
    } else {
        let mut nested = Vec::new();
        storage.reserve_vec(ctx, &mut nested, 1, resource)?;
        nested.push(values);
        Ok(nested)
    }
}

fn compute_bounds(ctx: &ParseScope<'_>, positions: &[Vec3]) -> ParseResult<Option<Aabb>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    let work = ctx.begin_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, positions.len(), "ply-bounds-work")?,
    )?;
    let mut min = first;
    let mut max = first;
    for (index, position) in positions.iter().copied().enumerate().skip(1) {
        if index.is_multiple_of(4096) {
            ctx.poll()?;
        }
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    work.complete()?;
    Ok(Some(Aabb { min, max }))
}

fn next_line<'a>(
    ctx: &ParseScope<'_>,
    bytes: &'a [u8],
    offset: &mut usize,
) -> ParseResult<Option<&'a [u8]>> {
    if *offset >= bytes.len() {
        return Ok(None);
    }
    let start = *offset;
    let mut end = bytes.len();
    while *offset < bytes.len() {
        let byte = bytes[*offset];
        *offset += 1;
        let line_bytes = *offset - start;
        if byte != b'\n' && line_bytes > ctx.limits().max_line_bytes {
            ctx.enforce_resource_limit(
                "max_line_bytes",
                usize_amount(ctx, line_bytes, "ply-line-bytes")?,
                usize_amount(ctx, ctx.limits().max_line_bytes, "ply-line-byte-limit")?,
            )?;
        }
        if line_bytes.is_multiple_of(4096) {
            ctx.poll()?;
        }
        if byte == b'\n' {
            end = (*offset).saturating_sub(1);
            break;
        }
    }
    ctx.enforce_resource_limit(
        "max_line_bytes",
        usize_amount(ctx, end.saturating_sub(start), "ply-line-bytes")?,
        usize_amount(ctx, ctx.limits().max_line_bytes, "ply-line-byte-limit")?,
    )?;
    Ok(Some(&bytes[start..end]))
}

#[derive(Debug, Clone, Copy)]
enum AsciiTextSurface {
    Header,
    Body,
}

impl AsciiTextSurface {
    const fn failure_code(self) -> PlyFailureCode {
        match self {
            Self::Header => PlyFailureCode::InvalidHeader,
            Self::Body => PlyFailureCode::InvalidRecord,
        }
    }

    const fn ascii_context(self) -> &'static str {
        match self {
            Self::Header => "header-ascii",
            Self::Body => "ascii-body",
        }
    }

    const fn newline_context(self) -> &'static str {
        match self {
            Self::Header => "header-newline",
            Self::Body => "ascii-body-newline",
        }
    }

    const fn control_context(self) -> &'static str {
        match self {
            Self::Header => "header-control",
            Self::Body => "ascii-body-control",
        }
    }
}

fn ascii_profile_line<'scope>(
    ctx: &'scope ParseScope<'_>,
    raw_line: &[u8],
    line_offset: usize,
    terminated_by_lf: bool,
    surface: AsciiTextSurface,
) -> ParseResult<ProfiledAsciiLine<'scope>> {
    if matches!(surface, AsciiTextSurface::Header)
        && line_offset == 0
        && raw_line.starts_with(&[0xef, 0xbb, 0xbf])
    {
        return Err(ctx.format_failure(
            PlyFailure::new(
                PlyFailureCode::InvalidHeader,
                "PLY text profile forbids a UTF-8 byte order mark",
            )
            .with_context("header-bom")
            .at_byte(0),
        ));
    }

    let strips_trailing_cr = terminated_by_lf && raw_line.last() == Some(&b'\r');
    let output_length = raw_line
        .len()
        .checked_sub(usize::from(strips_trailing_cr))
        .ok_or_else(|| ctx.resource_limit_exceeded("ply-profiled-line-bytes", u64::MAX))?;
    let mut storage = LiveStorage::new(ImportStorageClass::BackendDecode);
    let mut value =
        storage.empty_string_with_capacity(ctx, output_length, "ply-profiled-ascii-line")?;

    let mut chunk_start = 0usize;
    while chunk_start < raw_line.len() {
        ctx.poll()?;
        let chunk_end = chunk_start
            .checked_add(PROFILE_SCAN_CHUNK_BYTES)
            .map_or(raw_line.len(), |end| end.min(raw_line.len()));
        let chunk = raw_line.get(chunk_start..chunk_end).ok_or_else(|| {
            ctx.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY text profile scan range is invalid",
            ))
        })?;
        let validation_end = chunk_end
            .checked_add(UTF8_LOOKAHEAD_BYTES)
            .map_or(raw_line.len(), |end| end.min(raw_line.len()));
        let validation_chunk = raw_line.get(chunk_start..validation_end).ok_or_else(|| {
            ctx.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY UTF-8 validation range is invalid",
            ))
        })?;
        let utf8 = std::str::from_utf8(validation_chunk);

        ctx.poll()?;
        for (relative, byte) in chunk.iter().copied().enumerate() {
            let line_relative = chunk_start
                .checked_add(relative)
                .ok_or_else(|| ctx.resource_limit_exceeded("ply-text-byte-offset", u64::MAX))?;
            let context = if !byte.is_ascii() {
                Some(surface.ascii_context())
            } else if byte == b'\r'
                && !(terminated_by_lf
                    && line_relative
                        .checked_add(1)
                        .is_some_and(|end| end == raw_line.len()))
            {
                Some(surface.newline_context())
            } else if byte.is_ascii_control() && !matches!(byte, b'\t' | b'\r') {
                Some(surface.control_context())
            } else {
                None
            };
            let Some(context) = context else {
                continue;
            };
            let offending_offset = line_offset
                .checked_add(line_relative)
                .ok_or_else(|| ctx.resource_limit_exceeded("ply-text-byte-offset", u64::MAX))?;
            let mut failure = PlyFailure::new(
                surface.failure_code(),
                "PLY input violates its ASCII text profile",
            )
            .with_context(context)
            .at_byte(usize_amount(ctx, offending_offset, "ply-text-byte-offset")?);
            if !byte.is_ascii()
                && let Err(source) = utf8.as_ref()
                && source.valid_up_to() == relative
            {
                failure = failure.with_utf8_source(*source);
            }
            return Err(ctx.format_failure(failure));
        }

        ctx.poll()?;
        let chunk_text = std::str::from_utf8(chunk).map_err(|source| {
            ctx.format_failure(
                PlyFailure::new(
                    PlyFailureCode::InternalInvariant,
                    "ASCII-profiled PLY chunk is not UTF-8",
                )
                .with_utf8_source(source),
            )
        })?;
        let chunk_text = if strips_trailing_cr && chunk_end == raw_line.len() {
            chunk_text.strip_suffix('\r').ok_or_else(|| {
                format_failure(
                    ctx,
                    PlyFailureCode::InternalInvariant,
                    "PLY CRLF profile lost its trailing carriage return",
                    Some(line_offset),
                    Some("profiled-line-copy"),
                )
            })?
        } else {
            chunk_text
        };
        value.push_str(chunk_text);
        chunk_start = chunk_end;
    }
    if value.len() != output_length {
        return Err(format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY profiled line copy length changed",
            Some(line_offset),
            Some("profiled-line-copy"),
        ));
    }
    Ok(ProfiledAsciiLine {
        value,
        _storage: storage,
    })
}

fn line_terminated_by_lf(bytes: &[u8], offset: usize) -> bool {
    offset.checked_sub(1).and_then(|index| bytes.get(index)) == Some(&b'\n')
}

fn warn(
    ctx: &ParseScope<'_>,
    code: &'static str,
    byte_offset: Option<usize>,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).map_err(|_| {
        ctx.format_failure(PlyFailure::new(
            PlyFailureCode::InternalInvariant,
            "PLY diagnostic code is invalid",
        ))
    })?;
    let byte_range = byte_offset
        .map(|offset| {
            let offset = u64::try_from(offset)
                .map_err(|_| ctx.resource_limit_exceeded("ply-diagnostic-byte-offset", u64::MAX))?;
            DiagnosticByteRange::new(offset, offset).map_err(|_| {
                ctx.format_failure(PlyFailure::new(
                    PlyFailureCode::InternalInvariant,
                    "PLY diagnostic byte range is invalid",
                ))
            })
        })
        .transpose()?;
    ctx.emit_diagnostic_with(
        DiagnosticSeverity::Warning,
        code,
        ImportProgressPhase::Decode,
        ctx.diagnostic_source(),
        byte_range,
        None,
        |details| details.string("format", "ply"),
        message,
    )
}

fn ply_parse_error(
    ctx: &ParseScope<'_>,
    code: PlyFailureCode,
    byte_offset: Option<usize>,
    message: &'static str,
) -> ParseFailure {
    let mut failure = PlyFailure::new(code, message);
    if let Some(offset) = byte_offset {
        failure = failure.at_byte(u64::try_from(offset).unwrap_or(u64::MAX));
    }
    ctx.format_failure(failure)
}

fn format_failure(
    ctx: &ParseScope<'_>,
    code: PlyFailureCode,
    message: &'static str,
    byte_offset: Option<usize>,
    context: Option<&'static str>,
) -> ParseFailure {
    let mut failure = PlyFailure::new(code, message);
    if let Some(offset) = byte_offset {
        failure = failure.at_byte(u64::try_from(offset).unwrap_or(u64::MAX));
    }
    if let Some(context) = context {
        failure = failure.with_context(context);
    }
    ctx.format_failure(failure)
}

fn parse_error(
    ctx: &ParseScope<'_>,
    byte_offset: Option<usize>,
    message: &'static str,
) -> ParseFailure {
    ply_parse_error(ctx, PlyFailureCode::InvalidRecord, byte_offset, message)
}

fn usize_amount(ctx: &ParseScope<'_>, value: usize, resource: &'static str) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| ctx.resource_limit_exceeded(resource, u64::MAX))
}

fn resource_limit_failure(
    ctx: &ParseScope<'_>,
    resource: &'static str,
    limit: usize,
) -> ParseFailure {
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    ctx.resource_limit_exceeded(resource, limit)
}

fn preflight_limit(
    ctx: &ParseScope<'_>,
    amount: usize,
    maximum: usize,
    class: ImportWorkClass,
) -> ParseResult<()> {
    if amount > maximum {
        return ctx
            .begin_work(class, usize_amount(ctx, amount, "ply-limit-amount")?)?
            .complete();
    }
    Ok(())
}

fn insert_string_metadata<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    metadata: &mut MetadataMap,
    key: &str,
    value: &str,
) -> ParseResult<()> {
    storage.reserve_ordered_map(ctx, metadata, 1, "ply-metadata-entry")?;
    let key = storage.copy_string(ctx, key, "ply-metadata-key")?;
    let value = storage.copy_string(ctx, value, "ply-metadata-value")?;
    insert_reserved_metadata(ctx, metadata, key, value)
}

fn insert_indexed_string_metadata<'scope>(
    ctx: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    metadata: &mut MetadataMap,
    prefix: &str,
    index: usize,
    value: &str,
) -> ParseResult<()> {
    storage.reserve_ordered_map(ctx, metadata, 1, "ply-metadata-entry")?;
    let key = storage.copy_indexed_string(ctx, prefix, index, "ply-metadata-key")?;
    let value = storage.copy_string(ctx, value, "ply-metadata-value")?;
    insert_reserved_metadata(ctx, metadata, key, value)
}

fn insert_reserved_metadata(
    ctx: &ParseScope<'_>,
    metadata: &mut MetadataMap,
    key: String,
    value: String,
) -> ParseResult<()> {
    let insertion_work = metadata.len().checked_add(1).ok_or_else(|| {
        ctx.resource_limit_exceeded("max_work_units", ctx.limits().max_work_units)
    })?;
    let insertion = ctx.begin_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, insertion_work, "ply-metadata-insertion-work")?,
    )?;
    let replaced = metadata
        .insert_reserved(key, MetadataValue::String(value))
        .map_err(|_| {
            format_failure(
                ctx,
                PlyFailureCode::InternalInvariant,
                "PLY metadata insertion violated its reserved-capacity contract",
                None,
                Some("metadata-capacity"),
            )
        })?;
    insertion.complete()?;
    if replaced.is_some() {
        return Err(format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY metadata key was inserted more than once",
            None,
            Some("metadata-key"),
        ));
    }
    Ok(())
}

fn canonical_string_bytes(ctx: &ParseScope<'_>, header: &Header<'_>) -> ParseResult<usize> {
    let format_len = match header.format {
        PlyFormat::Ascii => "ascii".len(),
        PlyFormat::BinaryLittleEndian => "binary_little_endian".len(),
        PlyFormat::BinaryBigEndian => "binary_big_endian".len(),
    };
    let mut total = "ply:format"
        .len()
        .checked_add(format_len)
        .and_then(|value| value.checked_add("ply:source".len()))
        .and_then(|value| value.checked_add(ctx.source().as_str().len()))
        .and_then(|value| value.checked_add("PLY Mesh".len()))
        .and_then(|value| value.checked_add("PLY".len()))
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                ctx.limits().max_canonical_string_bytes,
            )
        })?;
    for (index, comment) in header.comments.iter().enumerate() {
        poll_collection(ctx, index)?;
        let key_len = "ply:comment."
            .len()
            .checked_add(decimal_digits(index))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    ctx.limits().max_canonical_string_bytes,
                )
            })?;
        total = total
            .checked_add(key_len)
            .and_then(|value| value.checked_add(comment.len()))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    ctx.limits().max_canonical_string_bytes,
                )
            })?;
    }
    for (index, info) in header.obj_infos.iter().enumerate() {
        poll_collection(ctx, index)?;
        let key_len = "ply:obj_info."
            .len()
            .checked_add(decimal_digits(index))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    ctx.limits().max_canonical_string_bytes,
                )
            })?;
        total = total
            .checked_add(key_len)
            .and_then(|value| value.checked_add(info.len()))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    ctx.limits().max_canonical_string_bytes,
                )
            })?;
    }
    Ok(total)
}

fn poll_collection(ctx: &ParseScope<'_>, index: usize) -> ParseResult<()> {
    if index.is_multiple_of(SCAN_QUANTUM) {
        ctx.poll()?;
    }
    Ok(())
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
