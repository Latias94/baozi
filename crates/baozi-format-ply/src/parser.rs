use crate::{FORMAT_ID, PlyFailure, PlyFailureCode};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic,
    AuthoredTransform, AuthoringRange, CustomAttributeSemantic, DeformationAuthoringBatch,
    ExtensionValueRef, FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, MetadataAuthoringEntry,
    NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentity,
    ResourceIdentityLocalId, SceneAuthoringBatch, SceneSpace, SessionOpaqueReason,
    SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator,
    SpatialAuthoringBatch,
};
use baozi_import::{
    DiagnosticByteRange, DiagnosticCode, DiagnosticSeverity, ImportProgressPhase,
    ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredString, MeteredVec, ParseFailure,
    ParseResult, ParseScope, SceneDraft, TextConformanceViolation, TextConformanceViolationKind,
    TextDecodingMode, TextDiagnosticLocation, TextInputOptions, TextInputRecordCursor,
};
use std::cell::RefCell;
use std::fmt;

const SCAN_QUANTUM: usize = 4 * 1024;
const MAX_NUMERIC_LITERAL_BYTES: usize = SCAN_QUANTUM;
const BODY_WORK_ENTRIES: usize = 8;
const BODY_RECORDS: usize = 0;
const BODY_VERTICES: usize = 1;
const BODY_FACES: usize = 2;
const BODY_LIST_LIMIT_VERTICES: usize = 3;
const BODY_TOKENS: usize = 4;
const BODY_TOKEN_BYTES: usize = 5;
const BODY_WORK_UNITS: usize = 6;
const BODY_TOPOLOGY_CELLS: usize = 7;
const MESH_NAME: &str = "PLY Mesh";
const NODE_NAME: &str = "PLY";
const FALLBACK_RESOURCE_IDENTITY: &str = "session/ply-primary";
const SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

fn body_completion(index: usize, amount: u64) -> [u64; BODY_WORK_ENTRIES] {
    let mut completed = [0; BODY_WORK_ENTRIES];
    completed[index] = amount;
    completed
}

pub(crate) fn read_ply<'scope, 'session>(
    scope: &'scope mut ParseScope<'session>,
) -> ParseResult<SceneDraft> {
    let bytes = scope.primary_bytes();
    let options = TextInputOptions::new(TextDecodingMode::RejectInvalid, "ply-text-input")
        .with_decoded_resource("ply-decoded-text-record")
        .with_line_resource("max_line_bytes")
        .with_line_index_resource("ply-line-index-bytes");
    let mut text = scope
        .text_input_record_cursor(scope.diagnostic_source(), options)?
        .map_err(|violation| {
            text_conformance_failure(scope, AsciiTextSurface::Header, violation)
        })?;
    let header = parse_header(scope, bytes, &mut text)?;
    let decoded_utf8_bytes = match header.format {
        PlyFormat::Ascii => bytes.len(),
        PlyFormat::BinaryLittleEndian | PlyFormat::BinaryBigEndian => header.data_start,
    };
    scope.enforce_resource_limit(
        "ply-decoded-utf8-bytes",
        usize_amount(scope, decoded_utf8_bytes, "ply-decoded-utf8-bytes")?,
        scope.limits().max_decoded_utf8_bytes,
    )?;
    let parsed = parse_body(scope, bytes, &header, &mut text)?;
    scene_from_parsed(scope, header, parsed)
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

enum Property<'scope> {
    Scalar {
        name: MeteredString<'scope>,
        ty: ScalarType,
        custom_attribute: Option<usize>,
        source_location: TextDiagnosticLocation<'scope>,
    },
    List {
        name: MeteredString<'scope>,
        count_ty: ScalarType,
        item_ty: ScalarType,
        source_location: TextDiagnosticLocation<'scope>,
    },
}

struct Element<'scope> {
    name: MeteredString<'scope>,
    count: usize,
    properties: MeteredVec<'scope, Property<'scope>>,
    next_custom_attribute: usize,
    source_location: TextDiagnosticLocation<'scope>,
}

struct Header<'scope> {
    format: PlyFormat,
    elements: MeteredVec<'scope, Element<'scope>>,
    comments: MeteredVec<'scope, MeteredString<'scope>>,
    obj_infos: MeteredVec<'scope, MeteredString<'scope>>,
    data_start: usize,
}

struct ParsedPly<'scope> {
    positions: MeteredVec<'scope, [f32; 3]>,
    normals: MeteredVec<'scope, [f32; 3]>,
    texcoords: MeteredVec<'scope, [f32; 2]>,
    colors: MeteredVec<'scope, [f32; 4]>,
    indices: MeteredVec<'scope, u32>,
    face_vertex_counts: MeteredVec<'scope, u32>,
    has_polygon: bool,
    custom_attributes: MeteredVec<'scope, CustomAttributeAccumulator<'scope>>,
}

#[derive(Debug, Clone, Copy)]
struct BodyWorkPlan {
    amounts: [u64; BODY_WORK_ENTRIES],
}

impl BodyWorkPlan {
    const fn record_only() -> Self {
        let mut amounts = [0; BODY_WORK_ENTRIES];
        amounts[BODY_RECORDS] = 1;
        Self { amounts }
    }

    fn for_element(element: &Element<'_>) -> Self {
        let mut amounts = Self::record_only().amounts;
        match element.name.as_str() {
            "vertex" => amounts[BODY_VERTICES] = 1,
            "face" => amounts[BODY_FACES] = 1,
            _ => {}
        }
        Self { amounts }
    }

    fn for_ascii_element(element: &Element<'_>) -> Self {
        let mut plan = Self::for_element(element);
        plan.amounts[BODY_RECORDS] = 0;
        plan
    }

    const fn charges(self) -> [(ImportWorkClass, u64); BODY_WORK_ENTRIES] {
        [
            (ImportWorkClass::Records, self.amounts[BODY_RECORDS]),
            (ImportWorkClass::Vertices, self.amounts[BODY_VERTICES]),
            (ImportWorkClass::Faces, self.amounts[BODY_FACES]),
            (
                ImportWorkClass::Vertices,
                self.amounts[BODY_LIST_LIMIT_VERTICES],
            ),
            (ImportWorkClass::Tokens, self.amounts[BODY_TOKENS]),
            (ImportWorkClass::TokenBytes, self.amounts[BODY_TOKEN_BYTES]),
            (ImportWorkClass::WorkUnits, self.amounts[BODY_WORK_UNITS]),
            (
                ImportWorkClass::TopologyCells,
                self.amounts[BODY_TOPOLOGY_CELLS],
            ),
        ]
    }

    fn add_token(&mut self, token: &str) -> bool {
        let Some(token_bytes) = u64::try_from(token.len()).ok() else {
            self.amounts[BODY_TOKEN_BYTES] = u64::MAX;
            return false;
        };
        self.amounts[BODY_TOKENS] = self.amounts[BODY_TOKENS].saturating_add(1);
        self.amounts[BODY_TOKEN_BYTES] = self.amounts[BODY_TOKEN_BYTES].saturating_add(token_bytes);
        self.amounts[BODY_TOKEN_BYTES] != u64::MAX
    }

    fn add_work_units(&mut self, amount: usize, maximum: u64) -> bool {
        let amount = u64::try_from(amount).unwrap_or(u64::MAX);
        self.amounts[BODY_WORK_UNITS] = self.amounts[BODY_WORK_UNITS].saturating_add(amount);
        self.amounts[BODY_WORK_UNITS] <= maximum
    }

    fn complete_row(self) -> [u64; BODY_WORK_ENTRIES] {
        let mut completed = [0; BODY_WORK_ENTRIES];
        completed[BODY_RECORDS] = self.amounts[BODY_RECORDS];
        completed[BODY_VERTICES] = self.amounts[BODY_VERTICES];
        completed[BODY_FACES] = self.amounts[BODY_FACES];
        completed
    }
}

struct CustomAttributeAccumulator<'scope> {
    name: MeteredString<'scope>,
    semantic: MeteredString<'scope>,
    data: RefCell<CustomAttributeData<'scope>>,
}

enum CustomAttributeData<'scope> {
    F32(MeteredVec<'scope, f32>),
    U32(MeteredVec<'scope, u32>),
    I32(MeteredVec<'scope, i32>),
}

struct CustomAttributeStorage<'scope> {
    semantic: CustomAttributeSemantic,
    data: CustomAttributeData<'scope>,
}

struct PendingMetadata<'scope> {
    key: MeteredString<'scope>,
    value: MeteredString<'scope>,
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

fn parse_header<'scope, 'session, 'source>(
    ctx: &'scope ParseScope<'session>,
    bytes: &[u8],
    text: &mut TextInputRecordCursor<'scope, 'session, 'source>,
) -> ParseResult<Header<'scope>> {
    with_next_ascii_record(ctx, text, AsciiTextSurface::Header, |line, _, _| {
        if line != "ply" {
            return Err(ply_parse_error(
                ctx,
                PlyFailureCode::InvalidHeader,
                Some(0),
                "PLY header must start with 'ply'",
            ));
        }
        Ok(())
    })?
    .ok_or_else(|| {
        ply_parse_error(
            ctx,
            PlyFailureCode::InvalidHeader,
            None,
            "PLY input is empty",
        )
    })?;

    let mut format = None;
    let mut elements = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-elements",
    );
    let mut current_element: Option<Element<'scope>> = None;
    let mut comments = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-comments",
    );
    let mut obj_infos = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-obj-infos",
    );
    let mut saw_vertex_element = false;
    let mut saw_face_element = false;

    let mut header_line_index = 0usize;
    loop {
        poll_collection(ctx, header_line_index)?;
        header_line_index = header_line_index
            .checked_add(1)
            .ok_or_else(|| ctx.resource_limit_exceeded("ply-header-lines", u64::MAX))?;
        let ended = with_next_ascii_record(
            ctx,
            text,
            AsciiTextSurface::Header,
            |line, line_offset, line_location| {
                if line.is_empty() {
                    return Ok(false);
                }
                if line == "end_header" {
                    if let Some(element) = current_element.take() {
                        elements.try_push(element)?;
                    }
                    return Ok(true);
                }
                if let Some(comment) = line.strip_prefix("comment ") {
                    let comment = bounded_string(ctx, comment, "comment")?;
                    comments.try_push(comment)?;
                    return Ok(false);
                }
                if let Some(info) = line.strip_prefix("obj_info ") {
                    let info = bounded_string(ctx, info, "obj_info")?;
                    obj_infos.try_push(info)?;
                    return Ok(false);
                }
                let tokens = checked_tokens(ctx, line)?;
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
                            elements.try_push(element)?;
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
                            name: bounded_string(ctx, name, "element name")?,
                            count,
                            properties: ctx.metered_vec(
                                ImportStorageClass::FormatNormalization,
                                "ply-header-properties",
                            ),
                            next_custom_attribute: 0,
                            source_location: line_location,
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
                            name: bounded_string(ctx, name, "property name")?,
                            count_ty: parse_scalar_type(ctx, count_ty, line_offset)?,
                            item_ty: parse_scalar_type(ctx, item_ty, line_offset)?,
                            source_location: line_location,
                        };
                        element.properties.try_push(property)?;
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
                        let name = bounded_string(ctx, name, "property name")?;
                        let custom_attribute = if element.name.as_str() == "vertex"
                            && !is_builtin_vertex_property(name.as_str())
                        {
                            let slot = element.next_custom_attribute;
                            element.next_custom_attribute =
                                slot.checked_add(1).ok_or_else(|| {
                                    resource_limit_failure(
                                        ctx,
                                        "max_vertex_attribute_streams_per_mesh",
                                        ctx.limits().max_vertex_attribute_streams_per_mesh,
                                    )
                                })?;
                            Some(slot)
                        } else {
                            None
                        };
                        let property = Property::Scalar {
                            name,
                            ty: parse_scalar_type(ctx, ty, line_offset)?,
                            custom_attribute,
                            source_location: line_location,
                        };
                        element.properties.try_push(property)?;
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
                Ok(false)
            },
        )?
        .ok_or_else(|| {
            ply_parse_error(
                ctx,
                PlyFailureCode::InvalidHeader,
                None,
                "PLY header has no end_header",
            )
        })?;
        if ended {
            break;
        }
    }

    let offset = raw_offset_usize(ctx, text.raw_offset(), "ply-header-data-offset")?;

    validate_face_topology_schemas(ctx, elements.as_slice())?;

    let format = format.ok_or_else(|| {
        ply_parse_error(
            ctx,
            PlyFailureCode::InvalidHeader,
            None,
            "PLY header has no format line",
        )
    })?;
    if format != PlyFormat::Ascii {
        validate_binary_record_schema(ctx, elements.as_slice())?;
    }
    validate_record_budget(ctx, elements.as_slice())?;
    if format != PlyFormat::Ascii {
        validate_binary_minimum_payload(ctx, bytes, offset, elements.as_slice())?;
    }

    Ok(Header {
        format,
        elements,
        comments,
        obj_infos,
        data_start: offset,
    })
}

fn validate_face_topology_schemas(ctx: &ParseScope<'_>, elements: &[Element]) -> ParseResult<()> {
    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        if element.name.as_str() != "face" {
            continue;
        }
        let mut topology_properties = 0usize;
        let mut topology_types = None;
        for (property_index, property) in element.properties.as_slice().iter().enumerate() {
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
            if name.as_str() != "vertex_indices" && name.as_str() != "vertex_index" {
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
        for (property_index, property) in element.properties.as_slice().iter().enumerate() {
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

fn parse_body<'scope, 'session, 'source>(
    ctx: &'scope ParseScope<'session>,
    bytes: &[u8],
    header: &Header<'scope>,
    text: &mut TextInputRecordCursor<'scope, 'session, 'source>,
) -> ParseResult<ParsedPly<'scope>> {
    let mut vertex_element = None;
    let mut face_count = 0usize;
    for (element_index, element) in header.elements.as_slice().iter().enumerate() {
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
    let mut positions = ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-positions");
    positions.try_reserve(vertex_count)?;
    let mut normals = ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-normals");
    let channels = vertex_profile.channels;
    if channels.normals {
        normals.try_reserve(vertex_count)?;
    }
    let mut texcoords = ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-texcoords");
    if channels.texcoords {
        texcoords.try_reserve(vertex_count)?;
    }
    let mut colors = ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-colors");
    if channels.colors {
        colors.try_reserve(vertex_count)?;
    }
    let mut face_vertex_counts =
        ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-face-counts");
    face_vertex_counts.try_reserve(face_count)?;
    let custom_attributes =
        custom_accumulators(ctx, vertex_element, vertex_profile.custom_scalar_count)?;
    let mut parsed = ParsedPly {
        positions,
        normals,
        texcoords,
        colors,
        indices: ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-indices"),
        face_vertex_counts,
        has_polygon: false,
        custom_attributes,
    };

    match header.format {
        PlyFormat::Ascii => parse_ascii_body(ctx, header, &mut parsed, text)?,
        PlyFormat::BinaryLittleEndian => parse_binary_body(ctx, bytes, header, false, &mut parsed)?,
        PlyFormat::BinaryBigEndian => parse_binary_body(ctx, bytes, header, true, &mut parsed)?,
    }

    Ok(parsed)
}

fn parse_ascii_body<'scope, 'session, 'source>(
    ctx: &'scope ParseScope<'session>,
    header: &Header<'_>,
    parsed: &mut ParsedPly<'scope>,
    text: &mut TextInputRecordCursor<'scope, 'session, 'source>,
) -> ParseResult<()> {
    for (element_index, element) in header.elements.as_slice().iter().enumerate() {
        poll_collection(ctx, element_index)?;
        warn_for_element_schema(ctx, element)?;
        for record_index in 0..element.count {
            poll_collection(ctx, record_index)?;
            let line_offset = raw_offset_usize(ctx, text.raw_offset(), "ply-text-byte-offset")?;
            ctx.with_parent_work(ImportWorkClass::Records, 1, || {
                with_next_ascii_record(
                    ctx,
                    text,
                    AsciiTextSurface::Body,
                    |line, line_offset, _line_location| {
                        let plan = plan_ascii_row(ctx, line, element, parsed);
                        ctx.with_work_group(plan.charges(), |work| -> ParseResult<()> {
                            let mut cursor = AsciiRow::new(line, line_offset);
                            parse_row(ctx, element, &mut cursor, parsed, plan, work)?;
                            cursor.finish(ctx, element.name.as_str(), work)
                        })
                    },
                )?
                .ok_or_else(|| {
                    format_failure(
                        ctx,
                        PlyFailureCode::TruncatedPayload,
                        "PLY ended inside an element",
                        Some(line_offset),
                        Some("ascii-element-row"),
                    )
                })
            })?;
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
    for (element_index, element) in header.elements.as_slice().iter().enumerate() {
        poll_collection(ctx, element_index)?;
        warn_for_element_schema(ctx, element)?;
        for record_index in 0..element.count {
            poll_collection(ctx, record_index)?;
            let plan = plan_binary_row(ctx, &cursor, element, parsed);
            ctx.with_work_group(plan.charges(), |work| -> ParseResult<()> {
                parse_row(ctx, element, &mut cursor, parsed, plan, work)
            })?;
        }
    }
    Ok(())
}

fn plan_ascii_row(
    ctx: &ParseScope<'_>,
    line: &str,
    element: &Element<'_>,
    parsed: &ParsedPly<'_>,
) -> BodyWorkPlan {
    let mut plan = BodyWorkPlan::for_ascii_element(element);
    let mut tokens = line.split_ascii_whitespace();
    for property in element.properties.as_slice() {
        match property {
            Property::Scalar { .. } => {
                let Some(token) = tokens.next() else {
                    return plan;
                };
                if !plan.add_token(token) {
                    return plan;
                }
            }
            Property::List { name, count_ty, .. } => {
                let Some(count_token) = tokens.next() else {
                    return plan;
                };
                if !plan.add_token(count_token) {
                    return plan;
                }
                let Some(count) = planned_ascii_list_count(count_token, *count_ty) else {
                    return plan;
                };
                if !plan_list_work(ctx, &mut plan, element, name.as_str(), count, parsed) {
                    return plan;
                }
                for _ in 0..count {
                    let Some(token) = tokens.next() else {
                        return plan;
                    };
                    if !plan.add_token(token) {
                        return plan;
                    }
                }
            }
        }
    }
    if let Some(trailing) = tokens.next() {
        let _ = plan.add_token(trailing);
    }
    plan
}

fn planned_ascii_list_count(token: &str, ty: ScalarType) -> Option<usize> {
    if token.len() > MAX_NUMERIC_LITERAL_BYTES {
        return None;
    }
    match ty {
        ScalarType::I8 => usize::try_from(token.parse::<i8>().ok()?).ok(),
        ScalarType::U8 => Some(token.parse::<u8>().ok()? as usize),
        ScalarType::I16 => usize::try_from(token.parse::<i16>().ok()?).ok(),
        ScalarType::U16 => Some(token.parse::<u16>().ok()? as usize),
        ScalarType::I32 => usize::try_from(token.parse::<i32>().ok()?).ok(),
        ScalarType::U32 => usize::try_from(token.parse::<u32>().ok()?).ok(),
        ScalarType::F32 | ScalarType::F64 => None,
    }
}

fn plan_binary_row(
    ctx: &ParseScope<'_>,
    cursor: &BinaryCursor<'_>,
    element: &Element<'_>,
    parsed: &ParsedPly<'_>,
) -> BodyWorkPlan {
    let mut plan = BodyWorkPlan::for_element(element);
    let mut offset = cursor.offset;
    for property in element.properties.as_slice() {
        match property {
            Property::Scalar { ty, .. } => {
                let Some(end) = offset.checked_add(scalar_width(*ty)) else {
                    return plan;
                };
                if end > cursor.bytes.len() {
                    return plan;
                }
                offset = end;
            }
            Property::List {
                name,
                count_ty,
                item_ty,
                ..
            } => {
                let Some((count, after_count)) =
                    planned_binary_list_count(cursor.bytes, offset, *count_ty, cursor.big_endian)
                else {
                    return plan;
                };
                offset = after_count;
                if !plan_list_work(ctx, &mut plan, element, name.as_str(), count, parsed) {
                    return plan;
                }
                let Some(item_bytes) = scalar_width(*item_ty).checked_mul(count) else {
                    return plan;
                };
                let Some(end) = offset.checked_add(item_bytes) else {
                    return plan;
                };
                if end > cursor.bytes.len() {
                    return plan;
                }
                offset = end;
            }
        }
    }
    plan
}

fn planned_binary_list_count(
    bytes: &[u8],
    offset: usize,
    ty: ScalarType,
    big_endian: bool,
) -> Option<(usize, usize)> {
    let end = offset.checked_add(scalar_width(ty))?;
    let bytes = bytes.get(offset..end)?;
    let count = match ty {
        ScalarType::I8 => usize::try_from(i8::from_ne_bytes([*bytes.first()?])).ok()?,
        ScalarType::U8 => *bytes.first()? as usize,
        ScalarType::I16 => {
            let bytes = <[u8; 2]>::try_from(bytes).ok()?;
            usize::try_from(if big_endian {
                i16::from_be_bytes(bytes)
            } else {
                i16::from_le_bytes(bytes)
            })
            .ok()?
        }
        ScalarType::U16 => {
            let bytes = <[u8; 2]>::try_from(bytes).ok()?;
            (if big_endian {
                u16::from_be_bytes(bytes)
            } else {
                u16::from_le_bytes(bytes)
            }) as usize
        }
        ScalarType::I32 => {
            let bytes = <[u8; 4]>::try_from(bytes).ok()?;
            usize::try_from(if big_endian {
                i32::from_be_bytes(bytes)
            } else {
                i32::from_le_bytes(bytes)
            })
            .ok()?
        }
        ScalarType::U32 => {
            let bytes = <[u8; 4]>::try_from(bytes).ok()?;
            usize::try_from(if big_endian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            })
            .ok()?
        }
        ScalarType::F32 | ScalarType::F64 => return None,
    };
    Some((count, end))
}

fn plan_list_work(
    ctx: &ParseScope<'_>,
    plan: &mut BodyWorkPlan,
    element: &Element<'_>,
    property_name: &str,
    count: usize,
    parsed: &ParsedPly<'_>,
) -> bool {
    if count > ctx.limits().max_vertices {
        plan.amounts[BODY_LIST_LIMIT_VERTICES] = u64::try_from(count).unwrap_or(u64::MAX);
        return false;
    }
    let topology = element.name.as_str() == "face"
        && matches!(property_name, "vertex_indices" | "vertex_index");
    if !topology {
        return plan.add_work_units(count, ctx.limits().max_work_units);
    }
    if count < 3 {
        return false;
    }

    let Some(next_indices) = parsed.indices.len().checked_add(count) else {
        plan.amounts[BODY_TOPOLOGY_CELLS] = u64::MAX;
        return false;
    };
    let Some(next_face_counts) = parsed.face_vertex_counts.len().checked_add(1) else {
        plan.amounts[BODY_FACES] = u64::MAX;
        return false;
    };
    let was_polygon = parsed.has_polygon;
    let has_polygon = was_polygon || count != 3;
    let canonical_cells = if has_polygon {
        let Some(cells) = next_indices.checked_add(next_face_counts) else {
            plan.amounts[BODY_TOPOLOGY_CELLS] = u64::MAX;
            return false;
        };
        cells
    } else {
        next_indices
    };
    if canonical_cells > ctx.limits().max_total_topology_cells {
        plan.amounts[BODY_TOPOLOGY_CELLS] = u64::try_from(canonical_cells).unwrap_or(u64::MAX);
        return false;
    }
    let face_count_cells = if !was_polygon && count != 3 {
        next_face_counts
    } else if was_polygon {
        1
    } else {
        0
    };
    let Some(charged_cells) = count.checked_add(face_count_cells) else {
        plan.amounts[BODY_TOPOLOGY_CELLS] = u64::MAX;
        return false;
    };
    plan.amounts[BODY_TOPOLOGY_CELLS] = u64::try_from(charged_cells).unwrap_or(u64::MAX);
    true
}

fn parse_row<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'scope>,
    plan: BodyWorkPlan,
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<()> {
    if element.name.as_str() == "vertex" {
        parse_vertex_row(ctx, element, reader, parsed, work)?;
    } else if element.name.as_str() == "face" {
        parse_face_row(ctx, element, reader, parsed, work)?;
    } else {
        skip_unknown_row(ctx, element, reader, work)?;
    }
    work.complete(plan.complete_row())
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
            Some(element.source_location),
            format_args!("PLY element '{}' was ignored", element.name.as_str()),
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

    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
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
                source_location,
                ..
            } => warn(
                ctx,
                "ply.vertex_list_property_ignored",
                Some(*source_location),
                format_args!("PLY vertex list property '{}' was ignored", name.as_str()),
            )?,
        }
    }

    let normal_components = usize::from(has_nx) + usize::from(has_ny) + usize::from(has_nz);
    if normal_components != 0 && normal_components != 3 {
        warn(
            ctx,
            "ply.partial_normals_ignored",
            Some(element.source_location),
            format_args!("PLY vertex has partial normal properties; normals were ignored"),
        )?;
    }
    if has_s != has_t {
        warn(
            ctx,
            "ply.partial_texcoords_ignored",
            Some(element.source_location),
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
            Some(element.source_location),
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

    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
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
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::List { name, .. }
                if name.as_str() == "vertex_indices" || name.as_str() == "vertex_index" => {}
            Property::List {
                name,
                source_location,
                ..
            } => warn(
                ctx,
                "ply.face_list_property_ignored",
                Some(*source_location),
                format_args!("PLY face list property '{}' was ignored", name.as_str()),
            )?,
            Property::Scalar {
                name,
                source_location,
                ..
            } => warn(
                ctx,
                "ply.face_scalar_property_ignored",
                Some(*source_location),
                format_args!("PLY face scalar property '{}' was ignored", name.as_str()),
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
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
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

    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar {
                name,
                ty,
                custom_attribute,
                ..
            } => {
                let value = reader.read_scalar(ctx, *ty, work)?;
                match name.as_str() {
                    "x" => x = Some(value.to_f32(ctx, "x")?),
                    "y" => y = Some(value.to_f32(ctx, "y")?),
                    "z" => z = Some(value.to_f32(ctx, "z")?),
                    "nx" => nx = Some(value.to_f32(ctx, "nx")?),
                    "ny" => ny = Some(value.to_f32(ctx, "ny")?),
                    "nz" => nz = Some(value.to_f32(ctx, "nz")?),
                    "s" | "u" | "texture_u" => s = Some(value.to_f32(ctx, name.as_str())?),
                    "t" | "v" | "texture_v" => t = Some(value.to_f32(ctx, name.as_str())?),
                    "red" | "r" => red = Some(value.to_color_component(ctx, *ty, name.as_str())?),
                    "green" | "g" => {
                        green = Some(value.to_color_component(ctx, *ty, name.as_str())?)
                    }
                    "blue" | "b" => {
                        blue = Some(value.to_color_component(ctx, *ty, name.as_str())?)
                    }
                    "alpha" | "a" => {
                        alpha = Some(value.to_color_component(ctx, *ty, name.as_str())?)
                    }
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
                        push_custom_scalar(ctx, parsed, attribute_index, name.as_str(), value)?;
                    }
                }
            }
            Property::List {
                count_ty, item_ty, ..
            } => {
                skip_list(ctx, reader, *count_ty, *item_ty, work)?;
            }
        }
    }

    let position = [
        x.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing x"))?,
        y.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing y"))?,
        z.ok_or_else(|| parse_error(ctx, None, "PLY vertex is missing z"))?,
    ];
    if !position.iter().all(|component| component.is_finite()) {
        return Err(ply_parse_error(
            ctx,
            PlyFailureCode::InvalidScalar,
            None,
            "PLY vertex position contains a non-finite value",
        ));
    }
    parsed.positions.try_push_reserved(position)?;

    match (nx, ny, nz) {
        (Some(nx), Some(ny), Some(nz)) => parsed.normals.try_push_reserved([nx, ny, nz])?,
        (None, None, None) => {}
        _ => {}
    }
    match (s, t) {
        (Some(s), Some(t)) => parsed.texcoords.try_push_reserved([s, t])?,
        (None, None) => {}
        _ => {}
    }
    match (red, green, blue) {
        (Some(red), Some(green), Some(blue)) => {
            parsed
                .colors
                .try_push_reserved([red, green, blue, alpha.unwrap_or(1.0)])?
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
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<()> {
    let mut found_vertices = false;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::List {
                name,
                count_ty,
                item_ty,
                ..
            } if name.as_str() == "vertex_indices" || name.as_str() == "vertex_index" => {
                found_vertices = true;
                let count = read_list_count(ctx, reader, *count_ty, work)?;
                if count < 3 {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidTopology,
                        None,
                        "PLY face has fewer than three vertices",
                    ));
                }
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
                if canonical_cells > ctx.limits().max_total_topology_cells {
                    return Err(resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        ctx.limits().max_total_topology_cells,
                    ));
                }
                parsed.indices.try_reserve(count)?;
                let face_count_cells = if !was_polygon && count != 3 {
                    next_face_counts
                } else if was_polygon {
                    1
                } else {
                    0
                };
                let _charged_cells = count.checked_add(face_count_cells).ok_or_else(|| {
                    resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        ctx.limits().max_total_topology_cells,
                    )
                })?;
                parsed.face_vertex_counts.try_push_reserved(count as u32)?;
                for position in 0..count {
                    if position.is_multiple_of(4096) {
                        work.poll()?;
                    }
                    let index = reader
                        .read_scalar(ctx, *item_ty, work)?
                        .to_u32(ctx, name.as_str())?;
                    parsed.indices.try_push_reserved(index)?;
                    work.complete(body_completion(BODY_TOPOLOGY_CELLS, 1))?;
                }
                parsed.has_polygon = has_polygon;
                work.complete(body_completion(
                    BODY_TOPOLOGY_CELLS,
                    usize_amount(ctx, face_count_cells, "ply-face-count-cells")?,
                ))?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => {
                skip_list(ctx, reader, *count_ty, *item_ty, work)?;
            }
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(ctx, *ty, work)?;
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
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<()> {
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(ctx, *ty, work)?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => skip_list(ctx, reader, *count_ty, *item_ty, work)?,
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
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, parsed.indices.len(), "ply-index-validation-work")?,
        |work| -> ParseResult<()> {
            for (position, index) in parsed.indices.as_slice().iter().enumerate() {
                if position.is_multiple_of(4096) {
                    work.poll()?;
                }
                if *index as usize >= parsed.positions.len() {
                    return Err(ply_parse_error(
                        ctx,
                        PlyFailureCode::InvalidTopology,
                        None,
                        "PLY face index references a missing vertex",
                    ));
                }
                work.complete([1])?;
            }
            Ok(())
        },
    )?;

    let keep_face_counts = parsed.has_polygon;
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
    let _bounds = compute_bounds(ctx, parsed.positions.as_slice())?;

    let canonical_string_bytes = canonical_string_bytes(ctx, &header)?;
    let metadata_work = metadata_insertion_work(ctx, &header)?;
    let (pending_metadata, mesh_name, node_name) = ctx.with_work_group(
        [
            (
                ImportWorkClass::CanonicalStringBytes,
                usize_amount(ctx, canonical_string_bytes, "ply-canonical-string-bytes")?,
            ),
            (
                ImportWorkClass::WorkUnits,
                usize_amount(ctx, metadata_work, "ply-metadata-insertion-work")?,
            ),
        ],
        |work| -> ParseResult<_> {
            let mut metadata = ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "ply-complete-batch-pending-metadata",
            );
            insert_string_metadata(
                ctx,
                &mut metadata,
                "ply:format",
                match header.format {
                    PlyFormat::Ascii => "ascii",
                    PlyFormat::BinaryLittleEndian => "binary_little_endian",
                    PlyFormat::BinaryBigEndian => "binary_big_endian",
                },
                work,
            )?;
            insert_string_metadata(
                ctx,
                &mut metadata,
                "ply:source",
                ctx.source().as_str(),
                work,
            )?;
            for (index, comment) in header.comments.as_slice().iter().enumerate() {
                poll_collection(ctx, index)?;
                insert_indexed_string_metadata(
                    ctx,
                    &mut metadata,
                    "ply:comment_",
                    index,
                    comment.as_str(),
                    work,
                )?;
            }
            for (index, info) in header.obj_infos.as_slice().iter().enumerate() {
                poll_collection(ctx, index)?;
                insert_indexed_string_metadata(
                    ctx,
                    &mut metadata,
                    "ply:obj_info_",
                    index,
                    info.as_str(),
                    work,
                )?;
            }
            let mut mesh_name =
                ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-mesh-name");
            mesh_name.try_push_str(MESH_NAME)?;
            let mut node_name =
                ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-node-name");
            node_name.try_push_str(NODE_NAME)?;
            metadata.sort_unstable_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
            work.complete([
                usize_amount(ctx, canonical_string_bytes, "ply-canonical-string-bytes")?,
                0,
            ])?;
            Ok((metadata, mesh_name, node_name))
        },
    )?;
    drop(header);

    let ParsedPly {
        positions,
        normals,
        texcoords,
        colors,
        indices,
        face_vertex_counts,
        custom_attributes,
        ..
    } = parsed;
    let mut custom_attributes = finish_custom_attributes(ctx, custom_attributes)?;
    custom_attributes.sort_unstable_by(|left, right| left.semantic.cmp(&right.semantic));

    let mut attributes = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-attributes",
    );
    let builtin_attribute_count = 1usize
        .checked_add(usize::from(!normals.is_empty()))
        .and_then(|count| count.checked_add(usize::from(!texcoords.is_empty())))
        .and_then(|count| count.checked_add(usize::from(!colors.is_empty())))
        .ok_or_else(|| ctx.allocation_failure("ply-authoring-attribute-count"))?;
    let attribute_count = builtin_attribute_count
        .checked_add(custom_attributes.len())
        .ok_or_else(|| ctx.allocation_failure("ply-authoring-attribute-count"))?;
    attributes.try_reserve(attribute_count)?;
    attributes.try_push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        positions.as_slice().as_flattened(),
        Presence::Absent,
    ))?;
    if !normals.is_empty() {
        attributes.try_push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            normals.as_slice().as_flattened(),
            Presence::Absent,
        ))?;
    }
    if !texcoords.is_empty() {
        attributes.try_push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            texcoords.as_slice().as_flattened(),
            Presence::Absent,
        ))?;
    }
    if !colors.is_empty() {
        attributes.try_push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Vertex,
            4,
            colors.as_slice().as_flattened(),
            Presence::Absent,
        ))?;
    }
    for custom in custom_attributes.as_slice() {
        attributes.try_push(custom.authoring())?;
    }

    let attribute_count = canonical_u32(ctx, attributes.len(), "ply-authoring-attribute-count")?;
    let topology = if indices.is_empty() {
        GeometryTopologyAuthoring::Points
    } else if keep_face_counts {
        GeometryTopologyAuthoring::Polygons(face_vertex_counts.as_slice())
    } else {
        GeometryTopologyAuthoring::TriangleList
    };
    let index_presence = if indices.is_empty() {
        Presence::Absent
    } else {
        Presence::Present(indices.as_slice())
    };
    let logical_mesh =
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1)).with_name(mesh_name.as_str());
    let primitive = GeometryPrimitiveAuthoring::new(
        topology,
        index_presence,
        Presence::Absent,
        AuthoringRange::new(0, attribute_count),
    );
    let mesh_instance =
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0));
    let logical_meshes = [logical_mesh];
    let primitives = [primitive];
    let mesh_instances = [mesh_instance];
    let geometry = GeometryAuthoringBatch::new(
        &logical_meshes,
        &primitives,
        attributes.as_slice(),
        &mesh_instances,
    );

    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )
    .with_name(node_name.as_str())];
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);

    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        &logical_morphs,
        &primitive_deformations,
        &mesh_instance_deformations,
    );

    let mut metadata_keys = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-metadata-keys",
    );
    metadata_keys.try_reserve(pending_metadata.len())?;
    for (index, metadata) in pending_metadata.as_slice().iter().enumerate() {
        poll_collection(ctx, index)?;
        let key = NamespacedName::new(metadata.key.as_str())
            .map_err(|_| canonical_contract_failure(ctx, "ply-complete-batch-metadata-key"))?;
        metadata_keys.try_push_reserved(key)?;
    }
    let mut metadata_entries = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-metadata-authoring",
    );
    metadata_entries.try_reserve(pending_metadata.len())?;
    for (metadata, key) in pending_metadata
        .as_slice()
        .iter()
        .zip(metadata_keys.as_slice())
    {
        metadata_entries.try_push_reserved(MetadataAuthoringEntry::new(
            ProfileOwner::LogicalMesh(0),
            key,
            ExtensionValueRef::String(metadata.value.as_str()),
        ))?;
    }

    let (resource_identity, resource_stability) = profile_resource_identity(ctx)?;
    let format =
        FormatId::new(FORMAT_ID).map_err(|_| ctx.allocation_failure("ply-profile-format-id"))?;
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource_identity,
        resource_stability,
    )];
    let source_identities = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    let provenance_nodes = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let owner_count = 2usize
        .checked_add(nodes.len())
        .and_then(|count| count.checked_add(logical_meshes.len()))
        .and_then(|count| count.checked_add(primitives.len()))
        .and_then(|count| count.checked_add(attributes.len()))
        .and_then(|count| count.checked_add(mesh_instances.len()))
        .ok_or_else(|| ctx.allocation_failure("ply-profile-owner-count"))?;
    let mut owner_provenance = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-owner-provenance",
    );
    owner_provenance.try_reserve(owner_count)?;
    owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
        ProfileOwner::Scene,
        &SOURCE_PROVENANCE,
    ))?;
    owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
        ProfileOwner::SyntheticRoot,
        &SOURCE_PROVENANCE,
    ))?;
    for index in 0..nodes.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(canonical_u32(
                ctx,
                index,
                "ply-profile-node-owner",
            )?)),
            &SOURCE_PROVENANCE,
        ))?;
    }
    for index in 0..logical_meshes.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::LogicalMesh(canonical_u32(ctx, index, "ply-profile-logical-mesh-owner")?),
            &SOURCE_PROVENANCE,
        ))?;
    }
    for index in 0..primitives.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::Primitive(canonical_u32(ctx, index, "ply-profile-primitive-owner")?),
            &SOURCE_PROVENANCE,
        ))?;
    }
    for index in 0..attributes.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::Attribute(canonical_u32(ctx, index, "ply-profile-attribute-owner")?),
            &SOURCE_PROVENANCE,
        ))?;
    }
    for index in 0..mesh_instances.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::MeshInstance(canonical_u32(
                ctx,
                index,
                "ply-profile-mesh-instance-owner",
            )?),
            &SOURCE_PROVENANCE,
        ))?;
    }
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            &resources,
            &source_identities,
            &provenance_nodes,
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::new(metadata_entries.as_slice()),
    );
    let scene_space = SceneSpace::UNKNOWN;
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        node_batch,
        AppearanceAuthoringBatch::empty(),
        geometry,
        deformation,
        AnimationAuthoringBatch::empty(),
        profile,
    );

    ctx.with_work_group(
        [
            (ImportWorkClass::Meshes, 1),
            (ImportWorkClass::Primitives, 1),
            (ImportWorkClass::GraphNodes, 2),
            (ImportWorkClass::GraphEdges, 1),
            (ImportWorkClass::GraphDepth, 2),
        ],
        |work| -> ParseResult<()> {
            work.run([1, 1, 0, 0, 0], || Ok(()))?;
            work.run([0, 0, 2, 0, 2], || Ok(()))?;
            work.run([0, 0, 0, 1, 0], || Ok(()))?;
            Ok(())
        },
    )?;
    ctx.submit_scene(input)
}

trait RowReader {
    fn read_scalar(
        &mut self,
        ctx: &ParseScope<'_>,
        ty: ScalarType,
        work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
    ) -> ParseResult<ScalarValue>;
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

    fn finish(
        &mut self,
        ctx: &ParseScope<'_>,
        element_name: &str,
        work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
    ) -> ParseResult<()> {
        if next_body_token(ctx, self.line, &mut self.byte_index, work)?.is_some() {
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
    fn read_scalar(
        &mut self,
        ctx: &ParseScope<'_>,
        ty: ScalarType,
        work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
    ) -> ParseResult<ScalarValue> {
        let token =
            next_body_token(ctx, self.line, &mut self.byte_index, work)?.ok_or_else(|| {
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
    fn read_scalar(
        &mut self,
        ctx: &ParseScope<'_>,
        ty: ScalarType,
        _work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
    ) -> ParseResult<ScalarValue> {
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
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<usize> {
    let count = reader
        .read_scalar(ctx, count_ty, work)?
        .to_u32(ctx, "list count")? as usize;
    if count > ctx.limits().max_vertices {
        return Err(resource_limit_failure(
            ctx,
            "max_vertices",
            ctx.limits().max_vertices,
        ));
    }
    Ok(count)
}

fn skip_list(
    ctx: &ParseScope<'_>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
    item_ty: ScalarType,
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<()> {
    let count = read_list_count(ctx, reader, count_ty, work)?;
    for index in 0..count {
        if index.is_multiple_of(4096) {
            work.poll()?;
        }
        let _ = reader.read_scalar(ctx, item_ty, work)?;
        work.complete(body_completion(BODY_WORK_UNITS, 1))?;
    }
    Ok(())
}

fn custom_accumulators<'scope>(
    ctx: &'scope ParseScope<'_>,
    element: &Element<'scope>,
    custom_scalar_count: usize,
) -> ParseResult<MeteredVec<'scope, CustomAttributeAccumulator<'scope>>> {
    let mut attributes = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-custom-attributes",
    );
    attributes.try_reserve(custom_scalar_count)?;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
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
        if is_builtin_vertex_property(name.as_str()) {
            continue;
        }
        let data = match ty {
            ScalarType::F32 | ScalarType::F64 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-f32-values",
                );
                values.try_reserve(element.count)?;
                CustomAttributeData::F32(values)
            }
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-u32-values",
                );
                values.try_reserve(element.count)?;
                CustomAttributeData::U32(values)
            }
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-i32-values",
                );
                values.try_reserve(element.count)?;
                CustomAttributeData::I32(values)
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
        ctx.with_work(
            ImportWorkClass::CanonicalStringBytes,
            usize_amount(ctx, canonical_bytes, "ply-custom-attribute-name")?,
            |work| -> ParseResult<()> {
                let mut attr_name = ctx.metered_string(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-attribute-name",
                );
                attr_name.try_push_str("ply:")?;
                attr_name.try_push_str(name.as_str())?;
                let mut semantic_name = ctx.metered_string(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-attribute-semantic",
                );
                semantic_name.try_push_str("ply:")?;
                semantic_name.try_push_str(name.as_str())?;
                attributes.try_push_reserved(CustomAttributeAccumulator {
                    name: attr_name,
                    semantic: semantic_name,
                    data: RefCell::new(data),
                })?;
                work.complete_all()
            },
        )?;
    }
    Ok(attributes)
}

fn finish_custom_attributes<'scope>(
    ctx: &'scope ParseScope<'_>,
    attributes: MeteredVec<'scope, CustomAttributeAccumulator<'scope>>,
) -> ParseResult<MeteredVec<'scope, CustomAttributeStorage<'scope>>> {
    let mut canonical = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-custom-attributes",
    );
    canonical.try_reserve(attributes.len())?;
    attributes.try_for_each_owned(|attribute| {
        let semantic = CustomAttributeSemantic::new(attribute.semantic.as_str()).map_err(|_| {
            canonical_contract_failure(ctx, "ply-complete-batch-custom-attribute-semantic")
        })?;
        if attribute.name.as_str() != semantic.as_str() {
            return Err(canonical_contract_failure(
                ctx,
                "ply-complete-batch-custom-attribute-name",
            ));
        }
        canonical.try_push(CustomAttributeStorage {
            semantic,
            data: attribute.data.into_inner(),
        })
    })?;
    Ok(canonical)
}

impl CustomAttributeStorage<'_> {
    fn authoring(&self) -> GeometryAttributeAuthoring<'_> {
        let semantic = AttributeSemantic::Custom(self.semantic.clone());
        match &self.data {
            CustomAttributeData::F32(values) => GeometryAttributeAuthoring::f32(
                semantic,
                AttributeDomain::Vertex,
                1,
                values.as_slice(),
                Presence::Absent,
            ),
            CustomAttributeData::U32(values) => GeometryAttributeAuthoring::u32(
                semantic,
                AttributeDomain::Vertex,
                1,
                values.as_slice(),
            ),
            CustomAttributeData::I32(values) => GeometryAttributeAuthoring::i32(
                semantic,
                AttributeDomain::Vertex,
                1,
                values.as_slice(),
            ),
        }
    }
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
    let cells = stream_count.checked_mul(vertex_count).ok_or_else(|| {
        resource_limit_failure(
            ctx,
            "max_total_vertex_attribute_cells",
            ctx.limits().max_total_vertex_attribute_cells,
        )
    })?;
    let stream_count = usize_amount(ctx, stream_count, "ply-attribute-stream-count")?;
    let cells = usize_amount(ctx, cells, "ply-attribute-cell-count")?;
    ctx.with_work_group(
        [
            (ImportWorkClass::AttributeStreams, stream_count),
            (ImportWorkClass::AttributeCells, cells),
        ],
        |work| -> ParseResult<()> { work.complete([stream_count, cells]) },
    )
}

fn push_custom_scalar<'scope>(
    ctx: &'scope ParseScope<'_>,
    parsed: &mut ParsedPly<'scope>,
    attribute_index: usize,
    name: &str,
    value: ScalarValue,
) -> ParseResult<()> {
    let Some(attribute) = parsed.custom_attributes.as_slice().get(attribute_index) else {
        let _ = name;
        return Err(format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY custom property references a missing attribute slot",
            None,
            Some("custom-attribute-slot"),
        ));
    };
    let mut data = attribute.data.try_borrow_mut().map_err(|_| {
        format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY custom attribute storage was reentered",
            None,
            Some("custom-attribute-storage"),
        )
    })?;
    match (&mut *data, value) {
        (CustomAttributeData::F32(values), value) => {
            values.try_push_reserved(value.to_f32(ctx, name)?)?
        }
        (CustomAttributeData::U32(values), ScalarValue::U32(value)) => {
            values.try_push_reserved(value)?
        }
        (CustomAttributeData::U32(values), ScalarValue::I32(value)) if value >= 0 => {
            values.try_push_reserved(value as u32)?
        }
        (CustomAttributeData::I32(values), ScalarValue::I32(value)) => {
            values.try_push_reserved(value)?
        }
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
    values: MeteredVec<'scope, &'line str>,
}

impl<'line> CheckedTokens<'line, '_> {
    fn as_slice(&self) -> &[&'line str] {
        self.values.as_slice()
    }
}

fn checked_tokens<'line, 'scope>(
    ctx: &'scope ParseScope<'_>,
    line: &'line str,
) -> ParseResult<CheckedTokens<'line, 'scope>> {
    let mut values = ctx.metered_vec(ImportStorageClass::BackendDecode, "ply-tokens");
    let mut byte_index = 0usize;
    while let Some(token) = next_checked_token(ctx, line, &mut byte_index)? {
        values.try_push(token)?;
    }
    Ok(CheckedTokens { values })
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
    let token_bytes = byte_index.saturating_sub(token_start);
    ctx.with_work_group(
        [
            (ImportWorkClass::Tokens, 1),
            (
                ImportWorkClass::TokenBytes,
                usize_amount(ctx, token_bytes, "ply-token-bytes")?,
            ),
        ],
        |work| -> ParseResult<_> {
            let token = line.get(token_start..*byte_index).ok_or_else(|| {
                format_failure(
                    ctx,
                    PlyFailureCode::InternalInvariant,
                    "PLY token range is not a valid ASCII boundary",
                    None,
                    Some("ascii-token-range"),
                )
            })?;
            work.complete([1, usize_amount(ctx, token.len(), "ply-token-bytes")?])?;
            Ok(Some(token))
        },
    )
}

fn next_body_token<'line>(
    ctx: &ParseScope<'_>,
    line: &'line str,
    byte_index: &mut usize,
    work: &mut ImportWorkScope<'_, '_, BODY_WORK_ENTRIES>,
) -> ParseResult<Option<&'line str>> {
    let bytes = line.as_bytes();
    if *byte_index == 0 {
        work.poll()?;
    }
    while bytes
        .get(*byte_index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *byte_index += 1;
        if (*byte_index).is_multiple_of(SCAN_QUANTUM) {
            work.poll()?;
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
            work.poll()?;
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
    let mut completed = [0; BODY_WORK_ENTRIES];
    completed[BODY_TOKENS] = 1;
    completed[BODY_TOKEN_BYTES] = usize_amount(ctx, token.len(), "ply-token-bytes")?;
    work.complete(completed)?;
    Ok(Some(token))
}

fn bounded_string<'scope>(
    ctx: &'scope ParseScope<'_>,
    value: &str,
    label: &'static str,
) -> ParseResult<MeteredString<'scope>> {
    ctx.with_work(
        ImportWorkClass::StringBytes,
        usize_amount(ctx, value.len(), "ply-string-bytes")?,
        |work| -> ParseResult<_> {
            if value.is_empty() {
                return Err(format_failure(
                    ctx,
                    PlyFailureCode::InvalidSchema,
                    "PLY named header value must not be empty",
                    None,
                    Some(label),
                ));
            }
            let mut output =
                ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-header-string");
            output.try_push_str(value)?;
            work.complete_all()?;
            Ok(output)
        },
    )
}

fn compute_bounds(
    ctx: &ParseScope<'_>,
    positions: &[[f32; 3]],
) -> ParseResult<Option<([f32; 3], [f32; 3])>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        usize_amount(ctx, positions.len(), "ply-bounds-work")?,
        |work| -> ParseResult<_> {
            let mut min = first;
            let mut max = first;
            work.complete([1])?;
            for (index, position) in positions.iter().copied().enumerate().skip(1) {
                if index.is_multiple_of(4096) {
                    work.poll()?;
                }
                for component in 0..3 {
                    min[component] = min[component].min(position[component]);
                    max[component] = max[component].max(position[component]);
                }
                work.complete([1])?;
            }
            Ok(Some((min, max)))
        },
    )
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

fn with_next_ascii_record<'scope, 'session, 'source, T>(
    ctx: &'scope ParseScope<'session>,
    text: &mut TextInputRecordCursor<'scope, 'session, 'source>,
    surface: AsciiTextSurface,
    action: impl for<'line> FnOnce(&'line str, usize, TextDiagnosticLocation<'scope>) -> ParseResult<T>,
) -> ParseResult<Option<T>> {
    match text.with_next_record(|input, _recoveries| {
        let mut lines = input.lines();
        let line = lines.next().ok_or_else(|| {
            ctx.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY text record produced no shared line view",
            ))
        })?;
        if lines.next().is_some() {
            return Err(ctx.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY text record produced multiple shared line views",
            )));
        }
        let line_offset = raw_offset_usize(
            ctx,
            line.raw_content_range().start(),
            "ply-text-byte-offset",
        )?;
        action(line.text(), line_offset, line.start_diagnostic_location())
    })? {
        Ok(value) => Ok(value),
        Err(violation) => Err(text_conformance_failure(ctx, surface, violation)),
    }
}

fn text_conformance_failure(
    ctx: &ParseScope<'_>,
    surface: AsciiTextSurface,
    violation: TextConformanceViolation,
) -> ParseFailure {
    let raw_offset = violation.raw_range().map(DiagnosticByteRange::start);
    match violation.kind() {
        TextConformanceViolationKind::ForbiddenBom => match raw_offset {
            Some(offset) => ctx.format_failure(
                PlyFailure::new(
                    PlyFailureCode::InvalidHeader,
                    "PLY text profile forbids a UTF-8 byte order mark",
                )
                .with_context("header-bom")
                .at_byte(offset),
            ),
            None => internal_text_conformance_failure(ctx, "forbidden-bom-range-missing"),
        },
        TextConformanceViolationKind::InvalidUtf8 | TextConformanceViolationKind::NonAscii => {
            byte_text_conformance_failure(ctx, surface, raw_offset, surface.ascii_context())
        }
        TextConformanceViolationKind::UnsupportedNewline => {
            byte_text_conformance_failure(ctx, surface, raw_offset, surface.newline_context())
        }
        TextConformanceViolationKind::ForbiddenControl => {
            byte_text_conformance_failure(ctx, surface, raw_offset, surface.control_context())
        }
        TextConformanceViolationKind::MissingTextProfile => {
            internal_text_conformance_failure(ctx, "missing-text-profile")
        }
        TextConformanceViolationKind::UnsupportedSourceEncoding => {
            internal_text_conformance_failure(ctx, "unsupported-source-encoding")
        }
        TextConformanceViolationKind::DecodingModeNotDeclared => {
            internal_text_conformance_failure(ctx, "decoding-mode-not-declared")
        }
        _ => internal_text_conformance_failure(ctx, "unknown-violation"),
    }
}

fn byte_text_conformance_failure(
    ctx: &ParseScope<'_>,
    surface: AsciiTextSurface,
    raw_offset: Option<u64>,
    context: &'static str,
) -> ParseFailure {
    match raw_offset {
        Some(offset) => ctx.format_failure(
            PlyFailure::new(
                surface.failure_code(),
                "PLY input violates its ASCII text profile",
            )
            .with_context(context)
            .at_byte(offset),
        ),
        None => internal_text_conformance_failure(ctx, "text-violation-range-missing"),
    }
}

fn internal_text_conformance_failure(ctx: &ParseScope<'_>, context: &'static str) -> ParseFailure {
    ctx.format_failure(
        PlyFailure::new(
            PlyFailureCode::InternalInvariant,
            "PLY shared text conformance contract failed",
        )
        .with_context(context),
    )
}

fn warn(
    ctx: &ParseScope<'_>,
    code: &'static str,
    location: Option<TextDiagnosticLocation<'_>>,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).map_err(|_| {
        ctx.format_failure(PlyFailure::new(
            PlyFailureCode::InternalInvariant,
            "PLY diagnostic code is invalid",
        ))
    })?;
    match location {
        Some(location) => ctx.emit_text_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            location,
            None,
            |details| details.string("format", "ply"),
            message,
        ),
        None => ctx.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            ctx.diagnostic_source(),
            None,
            None,
            |details| details.string("format", "ply"),
            message,
        ),
    }
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

fn raw_offset_usize(
    ctx: &ParseScope<'_>,
    value: u64,
    resource: &'static str,
) -> ParseResult<usize> {
    usize::try_from(value).map_err(|_| ctx.resource_limit_exceeded(resource, u64::MAX))
}

fn canonical_u32(ctx: &ParseScope<'_>, value: usize, resource: &'static str) -> ParseResult<u32> {
    u32::try_from(value).map_err(|_| ctx.allocation_failure(resource))
}

fn canonical_contract_failure(ctx: &ParseScope<'_>, subject: &'static str) -> ParseFailure {
    format_failure(
        ctx,
        PlyFailureCode::InternalInvariant,
        "PLY complete-batch authoring violates its canonical contract",
        None,
        Some(subject),
    )
}

fn profile_resource_identity(
    ctx: &ParseScope<'_>,
) -> ParseResult<(ResourceIdentity, SessionOpaqueReason)> {
    match ResourceIdentity::new(ctx.source().as_str()) {
        Ok(identity) => Ok((identity, SessionOpaqueReason::CallerLabel)),
        Err(_) => ResourceIdentity::new(FALLBACK_RESOURCE_IDENTITY)
            .map(|identity| (identity, SessionOpaqueReason::SessionFallback))
            .map_err(|_| ctx.allocation_failure("ply-profile-resource-identity")),
    }
}

fn decimal_digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |exponent| exponent as usize + 1)
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
        return ctx.with_work(
            class,
            usize_amount(ctx, amount, "ply-limit-amount")?,
            |work| work.complete_all(),
        );
    }
    Ok(())
}

fn metadata_insertion_work(ctx: &ParseScope<'_>, header: &Header<'_>) -> ParseResult<usize> {
    let entry_count = 2usize
        .checked_add(header.comments.len())
        .and_then(|count| count.checked_add(header.obj_infos.len()))
        .ok_or_else(|| {
            ctx.resource_limit_exceeded("max_work_units", ctx.limits().max_work_units)
        })?;
    (1..=entry_count).try_fold(0usize, |total, insertion| {
        total.checked_add(insertion).ok_or_else(|| {
            ctx.resource_limit_exceeded("max_work_units", ctx.limits().max_work_units)
        })
    })
}

fn insert_string_metadata<'scope>(
    ctx: &'scope ParseScope<'_>,
    metadata: &mut MeteredVec<'scope, PendingMetadata<'scope>>,
    key: &str,
    value: &str,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> ParseResult<()> {
    let mut key_storage =
        ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-metadata-key");
    key_storage.try_push_str(key)?;
    let mut value_storage = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "ply-metadata-value",
    );
    value_storage.try_push_str(value)?;
    insert_metadata_parts(ctx, metadata, key_storage, value_storage, work)
}

fn insert_indexed_string_metadata<'scope>(
    ctx: &'scope ParseScope<'_>,
    metadata: &mut MeteredVec<'scope, PendingMetadata<'scope>>,
    prefix: &str,
    index: usize,
    value: &str,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> ParseResult<()> {
    // Canonical NamespacedName local names do not admit dots; preserve the indexed
    // source value while normalizing only this structural separator.
    let mut key = ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-metadata-key");
    key.try_push_str(prefix)?;
    key.try_push_usize(index)?;
    let mut value_storage = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "ply-metadata-value",
    );
    value_storage.try_push_str(value)?;
    insert_metadata_parts(ctx, metadata, key, value_storage, work)
}

fn insert_metadata_parts<'scope>(
    ctx: &ParseScope<'_>,
    metadata: &mut MeteredVec<'scope, PendingMetadata<'scope>>,
    key: MeteredString<'scope>,
    value: MeteredString<'scope>,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> ParseResult<()> {
    if metadata
        .as_slice()
        .iter()
        .any(|candidate| candidate.key.as_str() == key.as_str())
    {
        return Err(format_failure(
            ctx,
            PlyFailureCode::InternalInvariant,
            "PLY metadata key was inserted more than once",
            None,
            Some("metadata-key"),
        ));
    }
    let insertion_work = metadata.len().checked_add(1).ok_or_else(|| {
        ctx.resource_limit_exceeded("max_work_units", ctx.limits().max_work_units)
    })?;
    metadata.try_push(PendingMetadata { key, value })?;
    work.complete([
        0,
        usize_amount(ctx, insertion_work, "ply-metadata-insertion-work")?,
    ])?;
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
        .and_then(|value| value.checked_add(MESH_NAME.len()))
        .and_then(|value| value.checked_add(NODE_NAME.len()))
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                ctx.limits().max_canonical_string_bytes,
            )
        })?;
    for (index, comment) in header.comments.as_slice().iter().enumerate() {
        poll_collection(ctx, index)?;
        let key_len = "ply:comment_"
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
    for (index, info) in header.obj_infos.as_slice().iter().enumerate() {
        poll_collection(ctx, index)?;
        let key_len = "ply:obj_info_"
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
