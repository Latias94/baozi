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
    ImportStorageClass, ImportWorkClass, MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS, MeteredString,
    MeteredVec, ParseChildren, ParseContext, ParseErrorView, ParseFailure, ParseParser,
    ParsePieceError, ParseResult, ParseRoot, ParseServices, ParseStorage, ParseWorkLayout,
    SourceScopeEntityKind, TextConformanceViolation, TextConformanceViolationKind,
    TextDecodingMode, TextDiagnosticLocation, TextInputOptions, TextInputRecordCursor,
};
use std::cell::RefCell;
use std::fmt;

const TEXT_SCAN_QUANTUM: usize = 4 * 1024;
const MAX_NUMERIC_LITERAL_BYTES: usize = TEXT_SCAN_QUANTUM;
const BODY_WORK_ENTRIES: usize = 8;
const BODY_RECORDS: usize = 0;
const BODY_VERTICES: usize = 1;
const BODY_FACES: usize = 2;
const BODY_LIST_LIMIT_VERTICES: usize = 3;
const BODY_TOKENS: usize = 4;
const BODY_TOKEN_BYTES: usize = 5;
const BODY_WORK_UNITS: usize = 6;
const BODY_TOPOLOGY_CELLS: usize = 7;
const BODY_WORK_CLASSES: [ImportWorkClass; BODY_WORK_ENTRIES] = [
    ImportWorkClass::Records,
    ImportWorkClass::Vertices,
    ImportWorkClass::Faces,
    ImportWorkClass::Vertices,
    ImportWorkClass::Tokens,
    ImportWorkClass::TokenBytes,
    ImportWorkClass::WorkUnits,
    ImportWorkClass::TopologyCells,
];
const MESH_NAME: &str = "PLY Mesh";
const NODE_NAME: &str = "PLY";
const FALLBACK_RESOURCE_IDENTITY: &str = "session/ply-primary";
const SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

fn body_completion(index: usize, amount: u64) -> [u64; BODY_WORK_ENTRIES] {
    let mut completed = [0; BODY_WORK_ENTRIES];
    completed[index] = amount;
    completed
}

pub(crate) fn read_ply(context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
    let (header, parsed) = context.with_execution(|root| -> ParseResult<_> {
        let limits = root.services()?.limits();
        let bytes = root.primary_bytes();
        let source = root.services()?.diagnostic_source();
        let options = TextInputOptions::new(TextDecodingMode::RejectInvalid, "ply-text-input")
            .with_decoded_resource("ply-decoded-text-record")
            .with_line_resource("max_line_bytes")
            .with_line_index_resource("ply-line-index-bytes");
        let mut text = match root.text_input_record_cursor(source, options)? {
            Ok(text) => text,
            Err(violation) => {
                return Err(text_conformance_failure(
                    &mut root.services()?,
                    AsciiTextSurface::Header,
                    violation,
                ));
            }
        };
        let header = parse_header(&mut root.children()?, bytes, &mut text)?;
        let decoded_utf8_bytes = match header.format {
            PlyFormat::Ascii => bytes.len(),
            PlyFormat::BinaryLittleEndian | PlyFormat::BinaryBigEndian => header.data_start,
        };
        let decoded_utf8_bytes = usize_amount(
            root.error_view(),
            decoded_utf8_bytes,
            "ply-decoded-utf8-bytes",
        )?;
        root.services()?.enforce_resource_limit(
            "ply-decoded-utf8-bytes",
            decoded_utf8_bytes,
            limits.max_decoded_utf8_bytes,
        )?;
        let parsed = parse_body(root, bytes, &header, &mut text)?;
        Ok((header, parsed))
    })?;
    scene_from_parsed(context, header, parsed)
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

enum Property<'session> {
    Scalar {
        name: MeteredString<'session>,
        ty: ScalarType,
        custom_attribute: Option<usize>,
        source_location: TextDiagnosticLocation<'session>,
    },
    List {
        name: MeteredString<'session>,
        count_ty: ScalarType,
        item_ty: ScalarType,
        source_location: TextDiagnosticLocation<'session>,
    },
}

struct Element<'session> {
    name: MeteredString<'session>,
    count: usize,
    properties: MeteredVec<'session, Property<'session>>,
    next_custom_attribute: usize,
    source_location: TextDiagnosticLocation<'session>,
}

struct Header<'session> {
    format: PlyFormat,
    elements: MeteredVec<'session, Element<'session>>,
    comments: MeteredVec<'session, MeteredString<'session>>,
    obj_infos: MeteredVec<'session, MeteredString<'session>>,
    data_start: usize,
}

struct ParsedPly<'session> {
    channels: VertexChannels,
    positions: GeometryStream<'session, 3>,
    normals: GeometryStream<'session, 3>,
    texcoords: GeometryStream<'session, 2>,
    colors: MeteredVec<'session, [f32; 4]>,
    indices: MeteredVec<'session, u32>,
    face_vertex_counts: MeteredVec<'session, u32>,
    has_polygon: bool,
    custom_attributes: MeteredVec<'session, CustomAttributeAccumulator<'session>>,
}

#[derive(Debug, Clone, Copy)]
enum GeometryScalarWidth {
    F32,
    F64,
}

impl GeometryScalarWidth {
    fn with_component(self, ty: ScalarType) -> Self {
        match ty {
            ScalarType::F64 | ScalarType::I32 | ScalarType::U32 => Self::F64,
            ScalarType::F32
            | ScalarType::I8
            | ScalarType::U8
            | ScalarType::I16
            | ScalarType::U16 => self,
        }
    }
}

enum GeometryStream<'session, const LANES: usize> {
    F32(MeteredVec<'session, [f32; LANES]>),
    F64(MeteredVec<'session, [f64; LANES]>),
}

impl<'session, const LANES: usize> GeometryStream<'session, LANES> {
    fn new(
        ctx: &mut ParseServices<'_, '_, 'session>,
        width: GeometryScalarWidth,
        capacity: usize,
        label: &'static str,
    ) -> ParseResult<Self> {
        match width {
            GeometryScalarWidth::F32 => {
                let mut values = ctx.metered_vec(ImportStorageClass::FormatNormalization, label);
                values.try_reserve(ctx, capacity)?;
                Ok(Self::F32(values))
            }
            GeometryScalarWidth::F64 => {
                let mut values = ctx.metered_vec(ImportStorageClass::FormatNormalization, label);
                values.try_reserve(ctx, capacity)?;
                Ok(Self::F64(values))
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn try_push_reserved(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, 'session, BODY_WORK_ENTRIES, true>,
        tuple: [f64; LANES],
        completed: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<()> {
        match self {
            Self::F32(values) => {
                let output = tuple.map(|value| value as f32);
                // The declared F32/small-integer components must round-trip exactly, including -0.
                if tuple
                    .iter()
                    .zip(output)
                    .any(|(source, output)| source.to_bits() != f64::from(output).to_bits())
                {
                    return Err(format_failure(
                        &mut row.services()?,
                        PlyFailureCode::InternalInvariant,
                        "PLY geometry component does not fit its declared storage width exactly",
                        None,
                        Some("geometry-scalar-width"),
                    ));
                }
                row.run(completed, || values.try_push_reserved(output))
            }
            Self::F64(values) => row.run(completed, || values.try_push_reserved(tuple)),
        }
    }

    fn authoring(&self, semantic: AttributeSemantic) -> GeometryAttributeAuthoring<'_> {
        match self {
            Self::F32(values) => GeometryAttributeAuthoring::f32(
                semantic,
                AttributeDomain::Vertex,
                LANES as u8,
                values.as_slice().as_flattened(),
                Presence::Absent,
            ),
            Self::F64(values) => GeometryAttributeAuthoring::f64(
                semantic,
                AttributeDomain::Vertex,
                LANES as u8,
                values.as_slice().as_flattened(),
            ),
        }
    }
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

struct CustomAttributeAccumulator<'session> {
    name: MeteredString<'session>,
    semantic: MeteredString<'session>,
    data: RefCell<CustomAttributeData<'session>>,
}

enum CustomAttributeData<'session> {
    F32(MeteredVec<'session, f32>),
    F64(MeteredVec<'session, f64>),
    U32(MeteredVec<'session, u32>),
    I32(MeteredVec<'session, i32>),
}

struct CustomAttributeStorage<'session> {
    semantic: CustomAttributeSemantic,
    data: CustomAttributeData<'session>,
}

struct PendingMetadata<'session> {
    key: MeteredString<'session>,
    value: MeteredString<'session>,
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
    position_width: GeometryScalarWidth,
    normal_width: GeometryScalarWidth,
    texcoord_width: GeometryScalarWidth,
    custom_scalar_count: usize,
    stream_count: usize,
}

fn parse_header<'op, 'session, 'source>(
    ctx: &mut ParseChildren<'_, 'op, 'session>,
    bytes: &[u8],
    text: &mut TextInputRecordCursor<'session, 'source>,
) -> ParseResult<Header<'session>> {
    let limits = ctx.services()?.limits();
    let first = with_next_ascii_record(ctx, text, AsciiTextSurface::Header, |ctx, line, _, _| {
        if line != "ply" {
            return Err(ply_parse_error(
                &mut ctx.services()?,
                PlyFailureCode::InvalidHeader,
                Some(0),
                "PLY header must start with 'ply'",
            ));
        }
        Ok(())
    })?;
    if first.is_none() {
        return Err(ply_parse_error(
            &mut ctx.services()?,
            PlyFailureCode::InvalidHeader,
            None,
            "PLY input is empty",
        ));
    }

    let mut format = None;
    let mut elements = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-elements",
    );
    let mut current_element: Option<Element<'session>> = None;
    let mut comments = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-comments",
    );
    let mut obj_infos = ctx.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-header-obj-infos",
    );
    let mut saw_vertex_element = false;
    let mut saw_face_element = false;
    let token_group = ctx.bind_group([ImportWorkClass::Tokens, ImportWorkClass::TokenBytes])?;
    let string_group = ctx.bind_group([ImportWorkClass::StringBytes])?;

    let mut header_line_index = 0usize;
    loop {
        poll_collection(&mut ctx.services()?, header_line_index)?;
        header_line_index = header_line_index.checked_add(1).ok_or_else(|| {
            ctx.error_view()
                .resource_limit_exceeded("ply-header-lines", u64::MAX)
        })?;
        let ended = with_next_ascii_record(
            ctx,
            text,
            AsciiTextSurface::Header,
            |ctx, line, line_offset, line_location| {
                if line.is_empty() {
                    return Ok(false);
                }
                if line == "end_header" {
                    if let Some(element) = current_element.take() {
                        elements.try_push(&mut ctx.services()?, element)?;
                    }
                    return Ok(true);
                }
                if let Some(comment) = line.strip_prefix("comment ") {
                    let comment = bounded_string(ctx, &string_group, comment, "comment")?;
                    comments.try_push(&mut ctx.services()?, comment)?;
                    return Ok(false);
                }
                if let Some(info) = line.strip_prefix("obj_info ") {
                    let info = bounded_string(ctx, &string_group, info, "obj_info")?;
                    obj_infos.try_push(&mut ctx.services()?, info)?;
                    return Ok(false);
                }
                let tokens = checked_tokens(ctx, &token_group, line)?;
                match tokens.as_slice() {
                    ["format", value, version] => {
                        if format.is_some() {
                            return Err(ply_parse_error(
                                &mut ctx.services()?,
                                PlyFailureCode::InvalidHeader,
                                Some(line_offset),
                                "PLY header has multiple format lines",
                            ));
                        }
                        if *version != "1.0" {
                            return Err(format_failure(
                                &mut ctx.services()?,
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
                                    &mut ctx.services()?,
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
                            elements.try_push(&mut ctx.services()?, element)?;
                        }
                        let count = parse_count(&mut ctx.services()?, count, line_offset)?;
                        match *name {
                            "vertex" => {
                                if saw_vertex_element {
                                    return Err(ply_parse_error(
                                        &mut ctx.services()?,
                                        PlyFailureCode::InvalidSchema,
                                        Some(line_offset),
                                        "multiple PLY vertex elements are not supported",
                                    ));
                                }
                                saw_vertex_element = true;
                                preflight_limit(
                                    ctx,
                                    count,
                                    limits.max_vertices,
                                    ImportWorkClass::Vertices,
                                )?;
                            }
                            "face" => {
                                if saw_face_element {
                                    return Err(ply_parse_error(
                                        &mut ctx.services()?,
                                        PlyFailureCode::InvalidSchema,
                                        Some(line_offset),
                                        "multiple PLY face elements are not supported",
                                    ));
                                }
                                saw_face_element = true;
                                preflight_limit(
                                    ctx,
                                    count,
                                    limits.max_faces,
                                    ImportWorkClass::Faces,
                                )?;
                            }
                            _ => {}
                        }
                        current_element = Some(Element {
                            name: bounded_string(ctx, &string_group, name, "element name")?,
                            count,
                            properties: ctx.services()?.metered_vec(
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
                                &mut ctx.services()?,
                                PlyFailureCode::InvalidSchema,
                                Some(line_offset),
                                "PLY property appeared before an element",
                            ));
                        };
                        let property = Property::List {
                            name: bounded_string(ctx, &string_group, name, "property name")?,
                            count_ty: parse_scalar_type(
                                &mut ctx.services()?,
                                count_ty,
                                line_offset,
                            )?,
                            item_ty: parse_scalar_type(&mut ctx.services()?, item_ty, line_offset)?,
                            source_location: line_location,
                        };
                        element
                            .properties
                            .try_push(&mut ctx.services()?, property)?;
                    }
                    ["property", ty, name] => {
                        let Some(element) = current_element.as_mut() else {
                            return Err(ply_parse_error(
                                &mut ctx.services()?,
                                PlyFailureCode::InvalidSchema,
                                Some(line_offset),
                                "PLY property appeared before an element",
                            ));
                        };
                        let name = bounded_string(ctx, &string_group, name, "property name")?;
                        let custom_attribute = if element.name.as_str() == "vertex"
                            && !is_builtin_vertex_property(name.as_str())
                        {
                            let slot = element.next_custom_attribute;
                            element.next_custom_attribute = {
                                let ctx = &mut ctx.services()?;
                                slot.checked_add(1).ok_or_else(|| {
                                    resource_limit_failure(
                                        ctx,
                                        "max_vertex_attribute_streams_per_mesh",
                                        limits.max_vertex_attribute_streams_per_mesh,
                                    )
                                })?
                            };
                            Some(slot)
                        } else {
                            None
                        };
                        let property = Property::Scalar {
                            name,
                            ty: parse_scalar_type(&mut ctx.services()?, ty, line_offset)?,
                            custom_attribute,
                            source_location: line_location,
                        };
                        element
                            .properties
                            .try_push(&mut ctx.services()?, property)?;
                    }
                    _ => {
                        return Err(format_failure(
                            &mut ctx.services()?,
                            PlyFailureCode::InvalidHeader,
                            "unsupported PLY header line",
                            Some(line_offset),
                            Some("header-line"),
                        ));
                    }
                }
                Ok(false)
            },
        )?;
        let Some(ended) = ended else {
            return Err(ply_parse_error(
                &mut ctx.services()?,
                PlyFailureCode::InvalidHeader,
                None,
                "PLY header has no end_header",
            ));
        };
        if ended {
            break;
        }
    }

    let offset = raw_offset_usize(
        ctx.error_view(),
        text.raw_offset(),
        "ply-header-data-offset",
    )?;

    let ctx = &mut ctx.services()?;

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

fn validate_face_topology_schemas(
    ctx: &mut ParseServices<'_, '_, '_>,
    elements: &[Element],
) -> ParseResult<()> {
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

fn validate_binary_record_schema(
    ctx: &mut ParseServices<'_, '_, '_>,
    elements: &[Element],
) -> ParseResult<()> {
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

fn validate_record_budget(
    ctx: &mut ParseServices<'_, '_, '_>,
    elements: &[Element],
) -> ParseResult<()> {
    let limits = ctx.limits();
    let limit = usize_amount(ctx.error_view(), limits.max_records, "ply-record-limit")?;
    let mut declared_records = 0_u64;
    for (element_index, element) in elements.iter().enumerate() {
        poll_collection(ctx, element_index)?;
        let count = usize_amount(ctx.error_view(), element.count, "ply-record-count")?;
        let Some(next) = declared_records.checked_add(count) else {
            return Err(ctx.resource_limit_exceeded("max_records", limit));
        };
        declared_records = next;
    }
    ctx.enforce_resource_limit("max_records", declared_records, limit)
}

fn validate_binary_minimum_payload(
    ctx: &mut ParseServices<'_, '_, '_>,
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

fn parse_body<'session, 'source>(
    root: &mut ParseRoot<'_, '_, 'session>,
    bytes: &[u8],
    header: &Header<'session>,
    text: &mut TextInputRecordCursor<'session, 'source>,
) -> ParseResult<ParsedPly<'session>> {
    let mut parsed = {
        let mut children = root.children()?;
        let ctx = &mut children.services()?;
        let limits = ctx.limits();
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
        preflight_limit(&mut children, 1, limits.max_meshes, ImportWorkClass::Meshes)?;
        preflight_limit(
            &mut children,
            1,
            limits.max_primitives,
            ImportWorkClass::Primitives,
        )?;

        let vertex_count = vertex_element.count;
        let vertex_profile = vertex_property_profile(&mut children.services()?, vertex_element)?;
        validate_vertex_attribute_budget(&mut children, vertex_count, vertex_profile.stream_count)?;
        let ctx = &mut children.services()?;
        let channels = vertex_profile.channels;
        let positions = GeometryStream::new(
            ctx,
            vertex_profile.position_width,
            vertex_count,
            "ply-positions",
        )?;
        let normals = GeometryStream::new(
            ctx,
            vertex_profile.normal_width,
            if channels.normals { vertex_count } else { 0 },
            "ply-normals",
        )?;
        let texcoords = GeometryStream::new(
            ctx,
            vertex_profile.texcoord_width,
            if channels.texcoords { vertex_count } else { 0 },
            "ply-texcoords",
        )?;
        let mut colors = ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-colors");
        if channels.colors {
            colors.try_reserve(ctx, vertex_count)?;
        }
        let mut face_vertex_counts =
            ctx.metered_vec(ImportStorageClass::FormatNormalization, "ply-face-counts");
        face_vertex_counts.try_reserve(ctx, face_count)?;
        let custom_attributes = custom_accumulators(
            &mut children,
            vertex_element,
            vertex_profile.custom_scalar_count,
        )?;
        ParsedPly {
            channels,
            positions,
            normals,
            texcoords,
            colors,
            indices: children
                .services()?
                .metered_vec(ImportStorageClass::FormatNormalization, "ply-indices"),
            face_vertex_counts,
            has_polygon: false,
            custom_attributes,
        }
    };

    match header.format {
        PlyFormat::Ascii => parse_ascii_body(root, header, &mut parsed, text)?,
        PlyFormat::BinaryLittleEndian => {
            parse_binary_body(root, bytes, header, false, &mut parsed)?
        }
        PlyFormat::BinaryBigEndian => parse_binary_body(root, bytes, header, true, &mut parsed)?,
    }

    Ok(parsed)
}

fn parse_ascii_body<'session, 'source>(
    root: &mut ParseRoot<'_, '_, 'session>,
    header: &Header<'_>,
    parsed: &mut ParsedPly<'session>,
    text: &mut TextInputRecordCursor<'session, 'source>,
) -> ParseResult<()> {
    let record_group = root.bind_group([ImportWorkClass::Records])?;
    let body_group = root.bind_group(BODY_WORK_CLASSES)?;
    for (element_index, element) in header.elements.as_slice().iter().enumerate() {
        poll_collection(&mut root.services()?, element_index)?;
        warn_for_element_schema(&mut root.services()?, element)?;
        for record_index in 0..element.count {
            poll_collection(&mut root.services()?, record_index)?;
            let line_offset =
                raw_offset_usize(root.error_view(), text.raw_offset(), "ply-text-byte-offset")?;
            root.parent(&record_group, [1], |parent| -> ParseResult<()> {
                let mut children = parent.children()?;
                let record = with_next_ascii_record(
                    &mut children,
                    text,
                    AsciiTextSurface::Body,
                    |children, line, line_offset, _line_location| {
                        let plan =
                            plan_ascii_row(&mut children.services()?, line, element, parsed)?;
                        children.child(&body_group, plan.amounts, |row| -> ParseResult<()> {
                            let mut cursor = AsciiRow::new(line, line_offset);
                            parse_row(row, element, &mut cursor, parsed, plan)?;
                            cursor.finish(row, element.name.as_str())
                        })
                    },
                )?;
                match record {
                    Some(()) => Ok(()),
                    None => Err(format_failure(
                        &mut children.services()?,
                        PlyFailureCode::TruncatedPayload,
                        "PLY ended inside an element",
                        Some(line_offset),
                        Some("ascii-element-row"),
                    )),
                }
            })?;
            #[cfg(test)]
            if element.name.as_str() == "face" {
                crate::control_tests::face_row_completed(parsed.face_vertex_counts.len());
            }
        }
    }
    Ok(())
}

fn parse_binary_body<'session>(
    root: &mut ParseRoot<'_, '_, 'session>,
    bytes: &[u8],
    header: &Header<'_>,
    big_endian: bool,
    parsed: &mut ParsedPly<'session>,
) -> ParseResult<()> {
    let mut cursor = BinaryCursor::new(bytes, header.data_start, big_endian);
    let body_group = root.bind_group(BODY_WORK_CLASSES)?;
    for (element_index, element) in header.elements.as_slice().iter().enumerate() {
        poll_collection(&mut root.services()?, element_index)?;
        warn_for_element_schema(&mut root.services()?, element)?;
        for record_index in 0..element.count {
            poll_collection(&mut root.services()?, record_index)?;
            let plan = plan_binary_row(&mut root.services()?, &cursor, element, parsed)?;
            root.child(&body_group, plan.amounts, |row| -> ParseResult<()> {
                parse_row(row, element, &mut cursor, parsed, plan)
            })?;
            #[cfg(test)]
            if element.name.as_str() == "face" {
                crate::control_tests::face_row_completed(parsed.face_vertex_counts.len());
            }
        }
    }
    Ok(())
}

fn plan_ascii_row(
    ctx: &mut ParseServices<'_, '_, '_>,
    line: &str,
    element: &Element<'_>,
    parsed: &ParsedPly<'_>,
) -> ParseResult<BodyWorkPlan> {
    let mut plan = BodyWorkPlan::for_ascii_element(element);
    let mut byte_index = 0;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar { .. } => {
                let Some(token) = next_ascii_token(ctx, line, &mut byte_index)? else {
                    return Ok(plan);
                };
                if !plan.add_token(token) {
                    return Ok(plan);
                }
            }
            Property::List { name, count_ty, .. } => {
                let Some(count_token) = next_ascii_token(ctx, line, &mut byte_index)? else {
                    return Ok(plan);
                };
                if !plan.add_token(count_token) {
                    return Ok(plan);
                }
                let Some(count) = planned_ascii_list_count(count_token, *count_ty) else {
                    return Ok(plan);
                };
                if !plan_list_work(ctx, &mut plan, element, name.as_str(), count, parsed) {
                    return Ok(plan);
                }
                for index in 0..count {
                    poll_collection(ctx, index)?;
                    let Some(token) = next_ascii_token(ctx, line, &mut byte_index)? else {
                        return Ok(plan);
                    };
                    if !plan.add_token(token) {
                        return Ok(plan);
                    }
                }
            }
        }
    }
    if let Some(trailing) = next_ascii_token(ctx, line, &mut byte_index)? {
        let _ = plan.add_token(trailing);
    }
    Ok(plan)
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
    ctx: &mut ParseServices<'_, '_, '_>,
    cursor: &BinaryCursor<'_>,
    element: &Element<'_>,
    parsed: &ParsedPly<'_>,
) -> ParseResult<BodyWorkPlan> {
    let mut plan = BodyWorkPlan::for_element(element);
    let mut offset = cursor.offset;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        match property {
            Property::Scalar { ty, .. } => {
                let Some(end) = offset.checked_add(scalar_width(*ty)) else {
                    return Ok(plan);
                };
                if end > cursor.bytes.len() {
                    return Ok(plan);
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
                    return Ok(plan);
                };
                offset = after_count;
                if !plan_list_work(ctx, &mut plan, element, name.as_str(), count, parsed) {
                    return Ok(plan);
                }
                let Some(item_bytes) = scalar_width(*item_ty).checked_mul(count) else {
                    return Ok(plan);
                };
                let Some(end) = offset.checked_add(item_bytes) else {
                    return Ok(plan);
                };
                if end > cursor.bytes.len() {
                    return Ok(plan);
                }
                offset = end;
            }
        }
    }
    Ok(plan)
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
    ctx: &ParseServices<'_, '_, '_>,
    plan: &mut BodyWorkPlan,
    element: &Element<'_>,
    property_name: &str,
    count: usize,
    parsed: &ParsedPly<'_>,
) -> bool {
    let limits = ctx.limits();
    if count > limits.max_vertices {
        plan.amounts[BODY_LIST_LIMIT_VERTICES] = u64::try_from(count).unwrap_or(u64::MAX);
        return false;
    }
    let topology = element.name.as_str() == "face"
        && matches!(property_name, "vertex_indices" | "vertex_index");
    if !topology {
        return plan.add_work_units(count, limits.max_work_units);
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
    if canonical_cells > limits.max_total_topology_cells {
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

fn parse_row<'session>(
    row: &mut ParseParser<'_, '_, '_, '_, 'session, BODY_WORK_ENTRIES, true>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'session>,
    plan: BodyWorkPlan,
) -> ParseResult<()> {
    if element.name.as_str() == "vertex" {
        parse_vertex_row(row, element, reader, parsed, plan.complete_row())
    } else if element.name.as_str() == "face" {
        parse_face_row(row, element, reader, parsed, plan.complete_row())
    } else {
        skip_unknown_row(row, element, reader, plan.complete_row())
    }
}

fn warn_for_element_schema(
    ctx: &mut ParseServices<'_, '_, '_>,
    element: &Element,
) -> ParseResult<()> {
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

fn warn_for_vertex_schema(
    ctx: &mut ParseServices<'_, '_, '_>,
    element: &Element,
) -> ParseResult<()> {
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
        record_partial_channel_loss(
            ctx,
            element.source_location,
            "ply.partial_normals_ignored",
            "normal",
        )?;
    }
    if has_s != has_t {
        record_partial_channel_loss(
            ctx,
            element.source_location,
            "ply.partial_texcoords_ignored",
            "texcoord",
        )?;
    }
    let color_components = usize::from(has_red) + usize::from(has_green) + usize::from(has_blue);
    if color_components != 0 && color_components != 3 {
        record_partial_channel_loss(
            ctx,
            element.source_location,
            "ply.partial_colors_ignored",
            "color",
        )?;
    }
    Ok(())
}

fn record_partial_channel_loss(
    ctx: &mut ParseServices<'_, '_, '_>,
    location: TextDiagnosticLocation<'_>,
    code: &'static str,
    channel: &'static str,
) -> ParseResult<()> {
    let mut object_path = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "ply-partial-channel-object-path",
    );
    object_path.try_write_fmt(ctx, format_args!("/elements/vertex/properties/{channel}"))?;
    ctx.record_source_semantic_loss(
        DiagnosticCode::new(code).expect("static PLY diagnostic code must be valid"),
        SourceScopeEntityKind::Mesh,
        object_path.as_str(),
        |details| {
            details.string("channel", channel);
            details.unsigned("byte_offset", location.byte_range().start());
            details.unsigned("byte_length", location.byte_range().end() - location.byte_range().start());
            details.unsigned("line", location.line_column().line());
            details.unsigned("column", location.line_column().column());
        },
        format_args!(
            "PLY vertex has partial {channel} properties; omitting the channel requires explicit source semantic loss authority"
        ),
    )
}

fn vertex_property_profile(
    ctx: &mut ParseServices<'_, '_, '_>,
    element: &Element,
) -> ParseResult<VertexPropertyProfile> {
    let limits = ctx.limits();
    let mut has_nx = false;
    let mut has_ny = false;
    let mut has_nz = false;
    let mut has_s = false;
    let mut has_t = false;
    let mut has_red = false;
    let mut has_green = false;
    let mut has_blue = false;
    let mut position_width = GeometryScalarWidth::F32;
    let mut normal_width = GeometryScalarWidth::F32;
    let mut texcoord_width = GeometryScalarWidth::F32;
    let mut custom_scalar_count = 0usize;

    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(ctx, property_index)?;
        let Property::Scalar {
            name,
            ty,
            custom_attribute,
            ..
        } = property
        else {
            continue;
        };
        match name.as_str() {
            "x" | "y" | "z" => position_width = position_width.with_component(*ty),
            "nx" | "ny" | "nz" => normal_width = normal_width.with_component(*ty),
            "s" | "u" | "texture_u" | "t" | "v" | "texture_v" => {
                texcoord_width = texcoord_width.with_component(*ty)
            }
            _ => {}
        }
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
                    limits.max_vertex_attribute_streams_per_mesh,
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
                limits.max_vertex_attribute_streams_per_mesh,
            )
        })?;
    let stream_count = builtin_streams
        .checked_add(custom_scalar_count)
        .ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_vertex_attribute_streams_per_mesh",
                limits.max_vertex_attribute_streams_per_mesh,
            )
        })?;
    Ok(VertexPropertyProfile {
        channels,
        position_width,
        normal_width,
        texcoord_width,
        custom_scalar_count,
        stream_count,
    })
}

fn warn_for_face_schema(ctx: &mut ParseServices<'_, '_, '_>, element: &Element) -> ParseResult<()> {
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

fn parse_vertex_row<'session>(
    row: &mut ParseParser<'_, '_, '_, '_, 'session, BODY_WORK_ENTRIES, true>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'session>,
    completed: [u64; BODY_WORK_ENTRIES],
) -> ParseResult<()> {
    let vertex_index = parsed.positions.len();
    let retain_colors = parsed.channels.colors;
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
        poll_collection(&mut row.services()?, property_index)?;
        match property {
            Property::Scalar {
                name,
                ty,
                custom_attribute,
                ..
            } => {
                let value = reader.read_scalar(row, *ty, [0; BODY_WORK_ENTRIES])?;
                let ctx = &mut row.services()?;
                match name.as_str() {
                    "x" => x = Some(value.to_f64(ctx, "x")?),
                    "y" => y = Some(value.to_f64(ctx, "y")?),
                    "z" => z = Some(value.to_f64(ctx, "z")?),
                    "nx" => nx = Some(value.to_f64(ctx, "nx")?),
                    "ny" => ny = Some(value.to_f64(ctx, "ny")?),
                    "nz" => nz = Some(value.to_f64(ctx, "nz")?),
                    "s" | "u" | "texture_u" => s = Some(value.to_f64(ctx, name.as_str())?),
                    "t" | "v" | "texture_v" => t = Some(value.to_f64(ctx, name.as_str())?),
                    "red" | "r" => {
                        red = Some(value.to_color_component(
                            ctx,
                            *ty,
                            name.as_str(),
                            vertex_index,
                            retain_colors,
                        )?)
                    }
                    "green" | "g" => {
                        green = Some(value.to_color_component(
                            ctx,
                            *ty,
                            name.as_str(),
                            vertex_index,
                            retain_colors,
                        )?)
                    }
                    "blue" | "b" => {
                        blue = Some(value.to_color_component(
                            ctx,
                            *ty,
                            name.as_str(),
                            vertex_index,
                            retain_colors,
                        )?)
                    }
                    "alpha" | "a" => {
                        alpha = Some(value.to_color_component(
                            ctx,
                            *ty,
                            name.as_str(),
                            vertex_index,
                            retain_colors,
                        )?)
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
                skip_list(row, reader, *count_ty, *item_ty, [0; BODY_WORK_ENTRIES])?;
            }
        }
    }

    let ctx = &mut row.services()?;
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
    let has_normals = nx.is_some() && ny.is_some() && nz.is_some();
    let has_texcoords = s.is_some() && t.is_some();
    let has_colors = red.is_some() && green.is_some() && blue.is_some();
    parsed.positions.try_push_reserved(
        row,
        position,
        if !has_normals && !has_texcoords && !has_colors {
            completed
        } else {
            [0; BODY_WORK_ENTRIES]
        },
    )?;

    match (nx, ny, nz) {
        (Some(nx), Some(ny), Some(nz)) => parsed.normals.try_push_reserved(
            row,
            [nx, ny, nz],
            if !has_texcoords && !has_colors {
                completed
            } else {
                [0; BODY_WORK_ENTRIES]
            },
        )?,
        (None, None, None) => {}
        _ => {}
    }
    match (s, t) {
        (Some(s), Some(t)) => parsed.texcoords.try_push_reserved(
            row,
            [s, t],
            if !has_colors {
                completed
            } else {
                [0; BODY_WORK_ENTRIES]
            },
        )?,
        (None, None) => {}
        _ => {}
    }
    match (red, green, blue) {
        (Some(red), Some(green), Some(blue)) => row.run(completed, || {
            parsed
                .colors
                .try_push_reserved([red, green, blue, alpha.unwrap_or(1.0)])
        })?,
        (None, None, None) => {}
        _ => {}
    }
    Ok(())
}

fn parse_face_row<'session>(
    row: &mut ParseParser<'_, '_, '_, '_, 'session, BODY_WORK_ENTRIES, true>,
    element: &Element,
    reader: &mut dyn RowReader,
    parsed: &mut ParsedPly<'session>,
    completed: [u64; BODY_WORK_ENTRIES],
) -> ParseResult<()> {
    let limits = row.services()?.limits();
    let mut found_vertices = false;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(&mut row.services()?, property_index)?;
        match property {
            Property::List {
                name,
                count_ty,
                item_ty,
                ..
            } if name.as_str() == "vertex_indices" || name.as_str() == "vertex_index" => {
                found_vertices = true;
                let count = read_list_count(row, reader, *count_ty, [0; BODY_WORK_ENTRIES])?;
                let ctx = &mut row.services()?;
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
                        limits.max_total_topology_cells,
                    )
                })?;
                let next_face_counts = parsed
                    .face_vertex_counts
                    .len()
                    .checked_add(1)
                    .ok_or_else(|| resource_limit_failure(ctx, "max_faces", limits.max_faces))?;
                let was_polygon = parsed.has_polygon;
                let has_polygon = was_polygon || count != 3;
                let canonical_cells = if has_polygon {
                    next_indices.checked_add(next_face_counts).ok_or_else(|| {
                        resource_limit_failure(
                            ctx,
                            "max_total_topology_cells",
                            limits.max_total_topology_cells,
                        )
                    })?
                } else {
                    next_indices
                };
                if canonical_cells > limits.max_total_topology_cells {
                    return Err(resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        limits.max_total_topology_cells,
                    ));
                }
                parsed.indices.try_reserve(ctx, count)?;
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
                        limits.max_total_topology_cells,
                    )
                })?;
                parsed.face_vertex_counts.try_push_reserved(count as u32)?;
                for position in 0..count {
                    poll_collection(&mut row.services()?, position)?;
                    let index = reader
                        .read_scalar(row, *item_ty, [0; BODY_WORK_ENTRIES])?
                        .to_u32(&mut row.services()?, name.as_str())?;
                    row.run(body_completion(BODY_TOPOLOGY_CELLS, 1), || {
                        parsed.indices.try_push_reserved(index)
                    })?;
                    #[cfg(test)]
                    crate::control_tests::face_index_completed(parsed.indices.len(), index);
                }
                let errors = row.error_view();
                row.run_returned(|| {
                    parsed.has_polygon = has_polygon;
                    let face_count_cells =
                        usize_amount(errors, face_count_cells, "ply-face-count-cells")?;
                    Ok(((), body_completion(BODY_TOPOLOGY_CELLS, face_count_cells)))
                })?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => {
                skip_list(row, reader, *count_ty, *item_ty, [0; BODY_WORK_ENTRIES])?;
            }
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(row, *ty, [0; BODY_WORK_ENTRIES])?;
            }
        }
    }
    row.run_format(completed, || {
        if found_vertices {
            Ok(())
        } else {
            Err(PlyFailure::new(
                PlyFailureCode::InvalidSchema,
                "PLY face element has no vertex_indices list property",
            ))
        }
    })
}

fn skip_unknown_row(
    row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
    element: &Element,
    reader: &mut dyn RowReader,
    completed: [u64; BODY_WORK_ENTRIES],
) -> ParseResult<()> {
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        poll_collection(&mut row.services()?, property_index)?;
        let completed = if property_index + 1 == element.properties.len() {
            completed
        } else {
            [0; BODY_WORK_ENTRIES]
        };
        match property {
            Property::Scalar { ty, .. } => {
                let _ = reader.read_scalar(row, *ty, completed)?;
            }
            Property::List {
                count_ty, item_ty, ..
            } => skip_list(row, reader, *count_ty, *item_ty, completed)?,
        }
    }
    Ok(())
}

fn scene_from_parsed<'session>(
    context: &mut ParseContext<'session, '_>,
    header: Header<'session>,
    parsed: ParsedPly<'session>,
) -> ParseResult<()> {
    let ParsedPly {
        positions,
        normals,
        texcoords,
        colors,
        indices,
        face_vertex_counts,
        has_polygon: keep_face_counts,
        custom_attributes,
        ..
    } = parsed;
    // Batches borrow outer payloads only after all authoring storage has left its region.
    let (pending_metadata, mesh_name, node_name, custom_attributes) =
        context.with_execution(|root| -> ParseResult<_> {
            let mut children = root.children()?;
            let limits = children.services()?.limits();
            preflight_limit(
                &mut children,
                2,
                limits.max_graph_nodes,
                ImportWorkClass::GraphNodes,
            )?;
            preflight_limit(
                &mut children,
                1,
                limits.max_graph_edges,
                ImportWorkClass::GraphEdges,
            )?;
            preflight_limit(
                &mut children,
                2,
                limits.max_graph_depth,
                ImportWorkClass::GraphDepth,
            )?;
            if positions.is_empty() {
                return Err(ply_parse_error(
                    &mut children.services()?,
                    PlyFailureCode::InvalidTopology,
                    None,
                    "PLY produced no vertices",
                ));
            }
            preflight_limit(
                &mut children,
                positions.len(),
                limits.max_vertices,
                ImportWorkClass::Vertices,
            )?;
            let face_count = face_vertex_counts.len();
            preflight_limit(
                &mut children,
                face_count,
                limits.max_faces,
                ImportWorkClass::Faces,
            )?;
            let index_group = children.bind_group([ImportWorkClass::WorkUnits])?;
            let index_work = usize_amount(
                children.error_view(),
                indices.len(),
                "ply-index-validation-work",
            )?;
            children.child(&index_group, [index_work], |parser| -> ParseResult<()> {
                for (position, index) in indices.as_slice().iter().enumerate() {
                    poll_collection(&mut parser.services()?, position)?;
                    parser.run_format([1], || {
                        if *index as usize >= positions.len() {
                            Err(PlyFailure::new(
                                PlyFailureCode::InvalidTopology,
                                "PLY face index references a missing vertex",
                            ))
                        } else {
                            Ok(())
                        }
                    })?;
                }
                Ok(())
            })?;

            let ctx = &mut children.services()?;
            let topology_cells = indices
                .len()
                .checked_add(if keep_face_counts {
                    face_vertex_counts.len()
                } else {
                    0
                })
                .ok_or_else(|| {
                    resource_limit_failure(
                        ctx,
                        "max_total_topology_cells",
                        limits.max_total_topology_cells,
                    )
                })?;
            preflight_limit(
                &mut children,
                topology_cells,
                limits.max_total_topology_cells,
                ImportWorkClass::TopologyCells,
            )?;
            let ctx = &mut children.services()?;
            let canonical_string_bytes = canonical_string_bytes(ctx, &header)?;
            let metadata_count = metadata_entry_count(ctx, &header)?;
            let amounts = [
                usize_amount(
                    ctx.error_view(),
                    canonical_string_bytes,
                    "ply-canonical-string-bytes",
                )?,
                usize_amount(ctx.error_view(), metadata_count, "ply-metadata-append-work")?,
            ];
            let metadata_group = children.bind_group([
                ImportWorkClass::CanonicalStringBytes,
                ImportWorkClass::WorkUnits,
            ])?;
            let (mut pending_metadata, mesh_name, node_name) =
                children.child(&metadata_group, amounts, |parser| -> ParseResult<_> {
                    let ctx = &mut parser.services()?;
                    let mut metadata = ctx.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "ply-complete-batch-pending-metadata",
                    );
                    metadata.try_reserve(ctx, metadata_count)?;
                    // Fixed keys and disjoint indexed prefixes make every generated key unique.
                    append_string_metadata(
                        parser,
                        &mut metadata,
                        "ply:format",
                        match header.format {
                            PlyFormat::Ascii => "ascii",
                            PlyFormat::BinaryLittleEndian => "binary_little_endian",
                            PlyFormat::BinaryBigEndian => "binary_big_endian",
                        },
                    )?;
                    let source = parser.services()?.source();
                    append_string_metadata(parser, &mut metadata, "ply:source", source.as_str())?;
                    for (index, comment) in header.comments.as_slice().iter().enumerate() {
                        poll_collection(&mut parser.services()?, index)?;
                        append_indexed_string_metadata(
                            parser,
                            &mut metadata,
                            "ply:comment_",
                            index,
                            comment.as_str(),
                        )?;
                    }
                    for (index, info) in header.obj_infos.as_slice().iter().enumerate() {
                        poll_collection(&mut parser.services()?, index)?;
                        append_indexed_string_metadata(
                            parser,
                            &mut metadata,
                            "ply:obj_info_",
                            index,
                            info.as_str(),
                        )?;
                    }
                    let ctx = &mut parser.services()?;
                    let mut mesh_name = ctx
                        .metered_string(ImportStorageClass::FormatNormalization, "ply-mesh-name");
                    mesh_name.try_push_str(ctx, MESH_NAME)?;
                    let mut node_name = ctx
                        .metered_string(ImportStorageClass::FormatNormalization, "ply-node-name");
                    parser.append_string([amounts[0], 0], &mut node_name, NODE_NAME)?;
                    Ok((metadata, mesh_name, node_name))
                })?;
            drop(header);
            order_metadata(&mut children, &mut pending_metadata)?;

            let mut custom_attributes =
                finish_custom_attributes(&mut children.services()?, custom_attributes)?;
            order_custom_attributes(&mut children, &mut custom_attributes)?;

            Ok((pending_metadata, mesh_name, node_name, custom_attributes))
        })?;

    let (attributes, attribute_count, metadata_keys) =
        context.with_execution(|root| -> ParseResult<_> {
            let ctx = &mut root.services()?;
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
            attributes.try_reserve(ctx, attribute_count)?;
            attributes.try_push(ctx, positions.authoring(AttributeSemantic::Position))?;
            if !normals.is_empty() {
                attributes.try_push(ctx, normals.authoring(AttributeSemantic::Normal))?;
            }
            if !texcoords.is_empty() {
                attributes.try_push(ctx, texcoords.authoring(AttributeSemantic::Texcoord(0)))?;
            }
            if !colors.is_empty() {
                attributes.try_push(
                    ctx,
                    GeometryAttributeAuthoring::f32(
                        AttributeSemantic::Color(0),
                        AttributeDomain::Vertex,
                        4,
                        colors.as_slice().as_flattened(),
                        Presence::Absent,
                    ),
                )?;
            }
            for (index, custom) in custom_attributes.as_slice().iter().enumerate() {
                poll_collection(ctx, index)?;
                attributes.try_push_reserved(custom.authoring())?;
            }

            let attribute_count =
                canonical_u32(ctx, attributes.len(), "ply-authoring-attribute-count")?;
            let mut metadata_keys = ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "ply-complete-batch-metadata-keys",
            );
            metadata_keys.try_reserve(ctx, pending_metadata.len())?;
            for (index, metadata) in pending_metadata.as_slice().iter().enumerate() {
                poll_collection(ctx, index)?;
                let key = NamespacedName::new(metadata.key.as_str()).map_err(|_| {
                    canonical_contract_failure(ctx, "ply-complete-batch-metadata-key")
                })?;
                metadata_keys.try_push_reserved(key)?;
            }
            Ok((attributes, attribute_count, metadata_keys))
        })?;

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
        baozi_import::authoring::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, attribute_count),
    );
    let mesh_instance =
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0));
    let logical_meshes = [logical_mesh];
    let primitives = [primitive];
    let mesh_instances = [mesh_instance];

    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )
    .with_name(node_name.as_str())];

    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];

    let (
        metadata_entries,
        resource_identity,
        resource_stability,
        format,
        owner_provenance,
        geometry,
        node_batch,
        deformation,
    ) = context.with_execution(|root| -> ParseResult<_> {
        let ctx = &mut root.services()?;
        let mut metadata_entries = ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "ply-complete-batch-metadata-authoring",
        );
        metadata_entries.try_reserve(ctx, pending_metadata.len())?;
        for (index, (metadata, key)) in pending_metadata
            .as_slice()
            .iter()
            .zip(metadata_keys.as_slice())
            .enumerate()
        {
            poll_collection(ctx, index)?;
            metadata_entries.try_push_reserved(MetadataAuthoringEntry::new(
                ProfileOwner::LogicalMesh(0),
                key,
                ExtensionValueRef::String(metadata.value.as_str()),
            ))?;
        }

        let (resource_identity, resource_stability) = profile_resource_identity(ctx)?;
        let format = FormatId::new(FORMAT_ID)
            .map_err(|_| ctx.allocation_failure("ply-profile-format-id"))?;
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
        owner_provenance.try_reserve(ctx, owner_count)?;
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
                ProfileOwner::LogicalMesh(canonical_u32(
                    ctx,
                    index,
                    "ply-profile-logical-mesh-owner",
                )?),
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
            poll_collection(ctx, index)?;
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
        let scene_group = root.bind_group([
            ImportWorkClass::Meshes,
            ImportWorkClass::Primitives,
            ImportWorkClass::GraphNodes,
            ImportWorkClass::GraphEdges,
            ImportWorkClass::GraphDepth,
        ])?;
        let (geometry, node_batch, deformation) =
            root.child(&scene_group, [1, 1, 2, 1, 2], |parser| -> ParseResult<_> {
                parser.run([1, 1, 2, 1, 2], || {
                    let geometry = GeometryAuthoringBatch::new(
                        &logical_meshes,
                        &primitives,
                        attributes.as_slice(),
                        &mesh_instances,
                    );
                    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
                    let deformation = DeformationAuthoringBatch::new(
                        &[],
                        &[],
                        &logical_morphs,
                        &primitive_deformations,
                        &mesh_instance_deformations,
                    );
                    Ok::<_, ParseFailure>((geometry, node_batch, deformation))
                })
            })?;
        Ok((
            metadata_entries,
            resource_identity,
            resource_stability,
            format,
            owner_provenance,
            geometry,
            node_batch,
            deformation,
        ))
    })?;

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

    context.submit_complete(input)
}

trait RowReader {
    fn read_scalar(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        completed: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<ScalarValue>;

    fn read_list_count(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        empty_completion: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<usize>;
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
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        element_name: &str,
    ) -> ParseResult<()> {
        if next_body_token(row, self.line, &mut self.byte_index)?.is_some() {
            let _ = element_name;
            return Err(format_failure(
                &mut row.services()?,
                PlyFailureCode::InvalidRecord,
                "PLY row has trailing tokens",
                Some(self.line_offset),
                Some("ascii-row"),
            ));
        }
        Ok(())
    }

    fn scalar_token(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
    ) -> ParseResult<&'line str> {
        let Some(token) = next_body_token(row, self.line, &mut self.byte_index)? else {
            return Err(ply_parse_error(
                &mut row.services()?,
                PlyFailureCode::TruncatedPayload,
                Some(self.line_offset),
                "PLY ASCII row ended before all properties were read",
            ));
        };
        if token.len() <= MAX_NUMERIC_LITERAL_BYTES {
            row.poll()?;
        }
        Ok(token)
    }
}

impl RowReader for AsciiRow<'_> {
    fn read_scalar(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        completed: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<ScalarValue> {
        let token = self.scalar_token(row)?;
        row.run_format(completed, || {
            parse_ascii_scalar(token, ty, self.line_offset)
        })
    }

    fn read_list_count(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        empty_completion: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<usize> {
        let token = self.scalar_token(row)?;
        row.run_returned_with_storage(|_storage| {
            let value =
                parse_ascii_scalar(token, ty, self.line_offset).map_err(ParsePieceError::Format)?;
            let count = value.unsigned_index().map_err(ParsePieceError::Format)? as usize;
            Ok((
                count,
                if count == 0 {
                    empty_completion
                } else {
                    [0; BODY_WORK_ENTRIES]
                },
            ))
        })
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

    fn take(&mut self, len: usize) -> Result<&'a [u8], PlyFailure> {
        let end = self.offset.checked_add(len).ok_or_else(|| {
            PlyFailure::new(
                PlyFailureCode::TruncatedPayload,
                "PLY binary payload offset overflowed",
            )
            .at_byte(u64::try_from(self.offset).unwrap_or(u64::MAX))
            .with_context("binary-offset")
        })?;
        let slice = self.bytes.get(self.offset..end).ok_or_else(|| {
            PlyFailure::new(
                PlyFailureCode::TruncatedPayload,
                "PLY binary payload ended inside a property",
            )
            .at_byte(u64::try_from(self.offset).unwrap_or(u64::MAX))
        })?;
        self.offset = end;
        Ok(slice)
    }
    fn scalar(&mut self, ty: ScalarType) -> Result<ScalarValue, PlyFailure> {
        let value = match ty {
            ScalarType::I8 => ScalarValue::I32(i8::from_ne_bytes([self.take(1)?[0]]) as i32),
            ScalarType::U8 => ScalarValue::U32(self.take(1)?[0] as u32),
            ScalarType::I16 => {
                let bytes = array2(self.take(2)?);
                ScalarValue::I32(if self.big_endian {
                    i16::from_be_bytes(bytes)
                } else {
                    i16::from_le_bytes(bytes)
                } as i32)
            }
            ScalarType::U16 => {
                let bytes = array2(self.take(2)?);
                ScalarValue::U32(if self.big_endian {
                    u16::from_be_bytes(bytes)
                } else {
                    u16::from_le_bytes(bytes)
                } as u32)
            }
            ScalarType::I32 => {
                let bytes = array4(self.take(4)?);
                ScalarValue::I32(if self.big_endian {
                    i32::from_be_bytes(bytes)
                } else {
                    i32::from_le_bytes(bytes)
                })
            }
            ScalarType::U32 => {
                let bytes = array4(self.take(4)?);
                ScalarValue::U32(if self.big_endian {
                    u32::from_be_bytes(bytes)
                } else {
                    u32::from_le_bytes(bytes)
                })
            }
            ScalarType::F32 => {
                let bytes = array4(self.take(4)?);
                ScalarValue::F64(if self.big_endian {
                    f32::from_be_bytes(bytes)
                } else {
                    f32::from_le_bytes(bytes)
                } as f64)
            }
            ScalarType::F64 => {
                let bytes = array8(self.take(8)?);
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

impl RowReader for BinaryCursor<'_> {
    fn read_scalar(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        completed: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<ScalarValue> {
        row.run_format(completed, || self.scalar(ty))
    }

    fn read_list_count(
        &mut self,
        row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
        ty: ScalarType,
        empty_completion: [u64; BODY_WORK_ENTRIES],
    ) -> ParseResult<usize> {
        row.run_returned_with_storage(|_storage| {
            let value = self.scalar(ty).map_err(ParsePieceError::Format)?;
            let count = value.unsigned_index().map_err(ParsePieceError::Format)? as usize;
            Ok((
                count,
                if count == 0 {
                    empty_completion
                } else {
                    [0; BODY_WORK_ENTRIES]
                },
            ))
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum ScalarValue {
    I32(i32),
    U32(u32),
    F64(f64),
}

impl ScalarValue {
    fn to_f64(self, ctx: &mut ParseServices<'_, '_, '_>, name: &str) -> ParseResult<f64> {
        let value = match self {
            Self::I32(value) => f64::from(value),
            Self::U32(value) => f64::from(value),
            Self::F64(value) => value,
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

    fn to_f32(self, ctx: &mut ParseServices<'_, '_, '_>, name: &str) -> ParseResult<f32> {
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

    fn to_u32(self, ctx: &mut ParseServices<'_, '_, '_>, name: &str) -> ParseResult<u32> {
        let _ = name;
        self.unsigned_index()
            .map_err(|failure| ctx.format_failure(failure))
    }

    fn unsigned_index(self) -> Result<u32, PlyFailure> {
        match self {
            Self::U32(value) => Ok(value),
            Self::I32(value) if value >= 0 => Ok(value as u32),
            Self::I32(_) | Self::F64(_) => Err(PlyFailure::new(
                PlyFailureCode::InvalidScalar,
                "PLY property is not a valid unsigned index",
            )
            .with_context("unsigned-index")),
        }
    }

    fn to_color_component(
        self,
        ctx: &mut ParseServices<'_, '_, '_>,
        ty: ScalarType,
        name: &str,
        vertex_index: usize,
        retained: bool,
    ) -> ParseResult<f32> {
        let value = self.to_f64(ctx, name)?;
        let normalized = match ty {
            ScalarType::U8 => value / 255.0,
            ScalarType::U16 => value / 65535.0,
            ScalarType::U32 => value / f64::from(u32::MAX),
            ScalarType::F32 | ScalarType::F64 if (0.0..=1.0).contains(&value) => value,
            ScalarType::F32 | ScalarType::F64 => {
                return Err(format_failure(
                    ctx,
                    PlyFailureCode::InvalidScalar,
                    "PLY floating color must be within [0, 1]",
                    None,
                    Some("color-component"),
                ));
            }
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => {
                return Err(format_failure(
                    ctx,
                    PlyFailureCode::InvalidScalar,
                    "PLY signed color scalars have no admitted interpretation",
                    None,
                    Some("color-component"),
                ));
            }
        };
        let output = normalized as f32;
        if retained && ty == ScalarType::F64 && f64::from(output).to_bits() != value.to_bits() {
            let mut object_path = ctx.metered_string(
                ImportStorageClass::FormatNormalization,
                "ply-color-precision-object-path",
            );
            object_path.try_write_fmt(
                ctx,
                format_args!("/elements/vertex/records/{vertex_index}/properties/{name}"),
            )?;
            ctx.record_source_semantic_loss(
                DiagnosticCode::new("ply.color_precision_loss")
                    .expect("static PLY diagnostic code must be valid"),
                SourceScopeEntityKind::Mesh,
                object_path.as_str(),
                |details| {
                    details.unsigned("vertex", vertex_index as u64);
                    details.string("property", name);
                    details.unsigned("source_f64_bits", value.to_bits());
                    details.unsigned("output_f32_bits", u64::from(output.to_bits()));
                },
                format_args!(
                    "PLY double color requires explicit source semantic loss authority to round to F32"
                ),
            )?;
        }
        Ok(output)
    }
}

fn read_list_count(
    row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
    empty_completion: [u64; BODY_WORK_ENTRIES],
) -> ParseResult<usize> {
    let limits = row.services()?.limits();
    let count = reader.read_list_count(row, count_ty, empty_completion)?;
    if count > limits.max_vertices {
        return Err(resource_limit_failure(
            &row.services()?,
            "max_vertices",
            limits.max_vertices,
        ));
    }
    Ok(count)
}

fn skip_list(
    row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
    reader: &mut dyn RowReader,
    count_ty: ScalarType,
    item_ty: ScalarType,
    completed: [u64; BODY_WORK_ENTRIES],
) -> ParseResult<()> {
    let count = read_list_count(row, reader, count_ty, completed)?;
    for index in 0..count {
        poll_collection(&mut row.services()?, index)?;
        let mut item_completion = if index + 1 == count {
            completed
        } else {
            [0; BODY_WORK_ENTRIES]
        };
        item_completion[BODY_WORK_UNITS] = 1;
        let _ = reader.read_scalar(row, item_ty, item_completion)?;
    }
    Ok(())
}

fn custom_accumulators<'session>(
    children: &mut ParseChildren<'_, '_, 'session>,
    element: &Element<'session>,
    custom_scalar_count: usize,
) -> ParseResult<MeteredVec<'session, CustomAttributeAccumulator<'session>>> {
    let ctx = &mut children.services()?;
    let limits = ctx.limits();
    let mut attributes = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-custom-attributes",
    );
    attributes.try_reserve(ctx, custom_scalar_count)?;
    let name_group = children.bind_group([ImportWorkClass::CanonicalStringBytes])?;
    for (property_index, property) in element.properties.as_slice().iter().enumerate() {
        let ctx = &mut children.services()?;
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
            ScalarType::F32 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-f32-values",
                );
                values.try_reserve(ctx, element.count)?;
                CustomAttributeData::F32(values)
            }
            ScalarType::F64 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-f64-values",
                );
                values.try_reserve(ctx, element.count)?;
                CustomAttributeData::F64(values)
            }
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-u32-values",
                );
                values.try_reserve(ctx, element.count)?;
                CustomAttributeData::U32(values)
            }
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 => {
                let mut values = ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "ply-custom-i32-values",
                );
                values.try_reserve(ctx, element.count)?;
                CustomAttributeData::I32(values)
            }
        };
        let attr_name_bytes = "ply:".len().checked_add(name.len()).ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                limits.max_canonical_string_bytes,
            )
        })?;
        let canonical_bytes = attr_name_bytes.checked_mul(2).ok_or_else(|| {
            resource_limit_failure(
                ctx,
                "max_canonical_string_bytes",
                limits.max_canonical_string_bytes,
            )
        })?;
        let amounts = [usize_amount(
            ctx.error_view(),
            canonical_bytes,
            "ply-custom-attribute-name",
        )?];
        children.child(&name_group, amounts, |child| -> ParseResult<()> {
            let ctx = &mut child.services()?;
            let mut attr_name = ctx.metered_string(
                ImportStorageClass::FormatNormalization,
                "ply-custom-attribute-name",
            );
            attr_name.try_push_str(ctx, "ply:")?;
            attr_name.try_push_str(ctx, name.as_str())?;
            let mut semantic_name = ctx.metered_string(
                ImportStorageClass::FormatNormalization,
                "ply-custom-attribute-semantic",
            );
            semantic_name.try_push_str(ctx, "ply:")?;
            semantic_name.try_push_str(ctx, name.as_str())?;
            let attribute = CustomAttributeAccumulator {
                name: attr_name,
                semantic: semantic_name,
                data: RefCell::new(data),
            };
            child.run(amounts, || attributes.try_push_reserved(attribute))
        })?;
    }
    Ok(attributes)
}

fn finish_custom_attributes<'session>(
    ctx: &mut ParseServices<'_, '_, 'session>,
    attributes: MeteredVec<'session, CustomAttributeAccumulator<'session>>,
) -> ParseResult<MeteredVec<'session, CustomAttributeStorage<'session>>> {
    let mut canonical = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "ply-complete-batch-custom-attributes",
    );
    canonical.try_reserve(ctx, attributes.len())?;
    let mut attribute_index = 0;
    attributes.try_for_each_owned(|attribute| {
        poll_collection(ctx, attribute_index)?;
        let semantic = CustomAttributeSemantic::new(attribute.semantic.as_str()).map_err(|_| {
            canonical_contract_failure(ctx, "ply-complete-batch-custom-attribute-semantic")
        })?;
        if attribute.name.as_str() != semantic.as_str() {
            return Err(canonical_contract_failure(
                ctx,
                "ply-complete-batch-custom-attribute-name",
            ));
        }
        canonical.try_push_reserved(CustomAttributeStorage {
            semantic,
            data: attribute.data.into_inner(),
        })?;
        attribute_index += 1;
        Ok(())
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
            CustomAttributeData::F64(values) => GeometryAttributeAuthoring::f64(
                semantic,
                AttributeDomain::Vertex,
                1,
                values.as_slice(),
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
    children: &mut ParseChildren<'_, '_, '_>,
    vertex_count: usize,
    stream_count: usize,
) -> ParseResult<()> {
    let ctx = &mut children.services()?;
    let limits = ctx.limits();
    ctx.enforce_resource_limit(
        "max_vertex_attribute_streams_per_mesh",
        usize_amount(ctx.error_view(), stream_count, "ply-attribute-stream-count")?,
        usize_amount(
            ctx.error_view(),
            limits.max_vertex_attribute_streams_per_mesh,
            "ply-attribute-stream-limit",
        )?,
    )?;
    let cells = stream_count.checked_mul(vertex_count).ok_or_else(|| {
        resource_limit_failure(
            ctx,
            "max_total_vertex_attribute_cells",
            limits.max_total_vertex_attribute_cells,
        )
    })?;
    let amounts = [
        usize_amount(ctx.error_view(), stream_count, "ply-attribute-stream-count")?,
        usize_amount(ctx.error_view(), cells, "ply-attribute-cell-count")?,
    ];
    let errors = ctx.error_view();
    let cell_limit = u64::try_from(limits.max_total_vertex_attribute_cells).unwrap_or(u64::MAX);
    let group = children.bind_group([
        ImportWorkClass::AttributeStreams,
        ImportWorkClass::AttributeCells,
    ])?;
    children.child(&group, amounts, |child| {
        child.run(amounts, || {
            stream_count
                .checked_mul(vertex_count)
                .ok_or_else(|| {
                    errors.resource_limit_exceeded("max_total_vertex_attribute_cells", cell_limit)
                })
                .map(|_| ())
        })
    })
}

fn push_custom_scalar<'session>(
    ctx: &mut ParseServices<'_, '_, 'session>,
    parsed: &mut ParsedPly<'session>,
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
        (CustomAttributeData::F64(values), value) => {
            values.try_push_reserved(value.to_f64(ctx, name)?)?
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
    token: &str,
    ty: ScalarType,
    line_offset: usize,
) -> Result<ScalarValue, PlyFailure> {
    match ty {
        ScalarType::I8 => parse_ascii_number::<i8>(token, line_offset, "signed-integer")
            .map(|value| ScalarValue::I32(value as i32)),
        ScalarType::I16 => parse_ascii_number::<i16>(token, line_offset, "signed-integer")
            .map(|value| ScalarValue::I32(value as i32)),
        ScalarType::I32 => {
            parse_ascii_number::<i32>(token, line_offset, "signed-integer").map(ScalarValue::I32)
        }
        ScalarType::U8 => parse_ascii_number::<u8>(token, line_offset, "unsigned-integer")
            .map(|value| ScalarValue::U32(value as u32)),
        ScalarType::U16 => parse_ascii_number::<u16>(token, line_offset, "unsigned-integer")
            .map(|value| ScalarValue::U32(value as u32)),
        ScalarType::U32 => {
            parse_ascii_number::<u32>(token, line_offset, "unsigned-integer").map(ScalarValue::U32)
        }
        ScalarType::F32 => parse_ascii_number::<f32>(token, line_offset, "float")
            .map(|value| ScalarValue::F64(value as f64)),
        ScalarType::F64 => {
            parse_ascii_number::<f64>(token, line_offset, "float").map(ScalarValue::F64)
        }
    }
}

fn parse_ascii_number<T>(
    token: &str,
    line_offset: usize,
    context: &'static str,
) -> Result<T, PlyFailure>
where
    T: std::str::FromStr,
{
    if token.len() > MAX_NUMERIC_LITERAL_BYTES {
        return Err(
            PlyFailure::new(PlyFailureCode::InvalidScalar, "invalid PLY scalar value")
                .at_byte(u64::try_from(line_offset).unwrap_or(u64::MAX))
                .with_context(context),
        );
    }
    token.parse::<T>().map_err(|_| {
        PlyFailure::new(PlyFailureCode::InvalidScalar, "invalid PLY scalar value")
            .at_byte(u64::try_from(line_offset).unwrap_or(u64::MAX))
            .with_context(context)
    })
}

fn parse_scalar_type(
    ctx: &mut ParseServices<'_, '_, '_>,
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

fn parse_count(
    ctx: &mut ParseServices<'_, '_, '_>,
    token: &str,
    line_offset: usize,
) -> ParseResult<usize> {
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

struct CheckedTokens<'line, 'session> {
    values: MeteredVec<'session, &'line str>,
}

impl<'line> CheckedTokens<'line, '_> {
    fn as_slice(&self) -> &[&'line str] {
        self.values.as_slice()
    }
}

fn checked_tokens<'line, 'op, 'session>(
    ctx: &mut ParseChildren<'_, 'op, 'session>,
    group: &ParseWorkLayout<'op, 2>,
    line: &'line str,
) -> ParseResult<CheckedTokens<'line, 'session>> {
    let mut values = ctx
        .services()?
        .metered_vec(ImportStorageClass::BackendDecode, "ply-tokens");
    let mut byte_index = 0usize;
    while let Some(token) = next_checked_token(ctx, group, line, &mut byte_index)? {
        values.try_push(&mut ctx.services()?, token)?;
    }
    Ok(CheckedTokens { values })
}

fn next_checked_token<'line, 'op, 'session>(
    ctx: &mut ParseChildren<'_, 'op, 'session>,
    group: &ParseWorkLayout<'op, 2>,
    line: &'line str,
    byte_index: &mut usize,
) -> ParseResult<Option<&'line str>> {
    let range = {
        let mut services = ctx.services()?;
        let mut storage = services.storage()?;
        scan_ascii_token_range(&mut storage, line, byte_index)?
    };
    let Some(range) = range else {
        return Ok(None);
    };
    let amounts = [
        1,
        usize_amount(ctx.error_view(), range.len(), "ply-token-bytes")?,
    ];
    ctx.child(group, amounts, |token| {
        token.run_format(amounts, || ascii_token(line, range).map(Some))
    })
}

fn next_body_token<'line>(
    row: &mut ParseParser<'_, '_, '_, '_, '_, BODY_WORK_ENTRIES, true>,
    line: &'line str,
    byte_index: &mut usize,
) -> ParseResult<Option<&'line str>> {
    let errors = row.error_view();
    row.run_returned_with_storage(|mut storage| {
        let range = scan_ascii_token_range(&mut storage, line, byte_index)
            .map_err(ParsePieceError::Parse)?;
        let Some(range) = range else {
            return Ok((None, [0; BODY_WORK_ENTRIES]));
        };
        let token = ascii_token(line, range).map_err(ParsePieceError::Format)?;
        let mut completed = [0; BODY_WORK_ENTRIES];
        completed[BODY_TOKENS] = 1;
        completed[BODY_TOKEN_BYTES] =
            usize_amount(errors, token.len(), "ply-token-bytes").map_err(ParsePieceError::Parse)?;
        Ok((Some(token), completed))
    })
}

fn next_ascii_token<'line>(
    ctx: &mut ParseServices<'_, '_, '_>,
    line: &'line str,
    byte_index: &mut usize,
) -> ParseResult<Option<&'line str>> {
    let range = scan_ascii_token_range(&mut ctx.storage()?, line, byte_index)?;
    range
        .map(|range| ascii_token(line, range).map_err(|failure| ctx.format_failure(failure)))
        .transpose()
}

fn scan_ascii_token_range(
    storage: &mut ParseStorage<'_, '_, '_>,
    line: &str,
    byte_index: &mut usize,
) -> ParseResult<Option<std::ops::Range<usize>>> {
    // Planning and body decoding poll at the same byte frontiers.
    let bytes = line.as_bytes();
    if *byte_index == 0 {
        storage.poll()?;
    }
    while bytes
        .get(*byte_index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *byte_index += 1;
        if (*byte_index).is_multiple_of(TEXT_SCAN_QUANTUM) {
            storage.poll()?;
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
        if (*byte_index).is_multiple_of(TEXT_SCAN_QUANTUM) {
            storage.poll()?;
        }
    }
    Ok(Some(token_start..*byte_index))
}

fn ascii_token(line: &str, range: std::ops::Range<usize>) -> Result<&str, PlyFailure> {
    line.get(range).ok_or_else(|| {
        PlyFailure::new(
            PlyFailureCode::InternalInvariant,
            "PLY token range is not a valid ASCII boundary",
        )
        .with_context("ascii-token-range")
    })
}

fn bounded_string<'op, 'session>(
    ctx: &mut ParseChildren<'_, 'op, 'session>,
    group: &ParseWorkLayout<'op, 1>,
    value: &str,
    label: &'static str,
) -> ParseResult<MeteredString<'session>> {
    let amounts = [usize_amount(
        ctx.error_view(),
        value.len(),
        "ply-string-bytes",
    )?];
    ctx.child(group, amounts, |child| -> ParseResult<_> {
        if value.is_empty() {
            return Err(format_failure(
                &mut child.services()?,
                PlyFailureCode::InvalidSchema,
                "PLY named header value must not be empty",
                None,
                Some(label),
            ));
        }
        let mut output = child
            .services()?
            .metered_string(ImportStorageClass::FormatNormalization, "ply-header-string");
        child.append_string(amounts, &mut output, value)?;
        Ok(output)
    })
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

fn with_next_ascii_record<'region, 'op, 'session, 'source, T>(
    ctx: &mut ParseChildren<'region, 'op, 'session>,
    text: &mut TextInputRecordCursor<'session, 'source>,
    surface: AsciiTextSurface,
    action: impl for<'line> FnOnce(
        &mut ParseChildren<'region, 'op, 'session>,
        &'line str,
        usize,
        TextDiagnosticLocation<'session>,
    ) -> ParseResult<T>,
) -> ParseResult<Option<T>> {
    match text.with_next_record(ctx, |ctx, input, _recoveries| {
        let mut lines = input.lines();
        let Some(line) = lines.next() else {
            return Err(ctx.services()?.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY text record produced no shared line view",
            )));
        };
        if lines.next().is_some() {
            return Err(ctx.services()?.format_failure(PlyFailure::new(
                PlyFailureCode::InternalInvariant,
                "PLY text record produced multiple shared line views",
            )));
        }
        let line_offset = raw_offset_usize(
            ctx.error_view(),
            line.raw_content_range().start(),
            "ply-text-byte-offset",
        )?;
        action(
            ctx,
            line.text(),
            line_offset,
            line.start_diagnostic_location(),
        )
    })? {
        Ok(value) => Ok(value),
        Err(violation) => Err(text_conformance_failure(
            &mut ctx.services()?,
            surface,
            violation,
        )),
    }
}

fn text_conformance_failure(
    ctx: &mut ParseServices<'_, '_, '_>,
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
    ctx: &mut ParseServices<'_, '_, '_>,
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

fn internal_text_conformance_failure(
    ctx: &mut ParseServices<'_, '_, '_>,
    context: &'static str,
) -> ParseFailure {
    ctx.format_failure(
        PlyFailure::new(
            PlyFailureCode::InternalInvariant,
            "PLY shared text conformance contract failed",
        )
        .with_context(context),
    )
}

fn warn(
    ctx: &mut ParseServices<'_, '_, '_>,
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
    ctx: &mut ParseServices<'_, '_, '_>,
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
    ctx: &mut ParseServices<'_, '_, '_>,
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
    ctx: &mut ParseServices<'_, '_, '_>,
    byte_offset: Option<usize>,
    message: &'static str,
) -> ParseFailure {
    ply_parse_error(ctx, PlyFailureCode::InvalidRecord, byte_offset, message)
}

fn usize_amount(
    errors: ParseErrorView<'_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| errors.resource_limit_exceeded(resource, u64::MAX))
}

fn raw_offset_usize(
    errors: ParseErrorView<'_>,
    value: u64,
    resource: &'static str,
) -> ParseResult<usize> {
    usize::try_from(value).map_err(|_| errors.resource_limit_exceeded(resource, u64::MAX))
}

fn canonical_u32(
    ctx: &ParseServices<'_, '_, '_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u32> {
    u32::try_from(value).map_err(|_| ctx.allocation_failure(resource))
}

fn canonical_contract_failure(
    ctx: &mut ParseServices<'_, '_, '_>,
    subject: &'static str,
) -> ParseFailure {
    format_failure(
        ctx,
        PlyFailureCode::InternalInvariant,
        "PLY complete-batch authoring violates its canonical contract",
        None,
        Some(subject),
    )
}

fn profile_resource_identity(
    ctx: &ParseServices<'_, '_, '_>,
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
    ctx: &ParseServices<'_, '_, '_>,
    resource: &'static str,
    limit: usize,
) -> ParseFailure {
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    ctx.resource_limit_exceeded(resource, limit)
}

fn preflight_limit(
    children: &mut ParseChildren<'_, '_, '_>,
    amount: usize,
    maximum: usize,
    class: ImportWorkClass,
) -> ParseResult<()> {
    if amount > maximum {
        let group = children.bind_group([class])?;
        let amount = usize_amount(children.error_view(), amount, "ply-limit-amount")?;
        let maximum = u64::try_from(maximum).unwrap_or(u64::MAX);
        return children.child(&group, [amount], |child| -> ParseResult<()> {
            Err(child
                .error_view()
                .resource_limit_exceeded("ply-limit-amount", maximum))
        });
    }
    Ok(())
}

fn metadata_entry_count(
    ctx: &ParseServices<'_, '_, '_>,
    header: &Header<'_>,
) -> ParseResult<usize> {
    let limits = ctx.limits();
    2usize
        .checked_add(header.comments.len())
        .and_then(|count| count.checked_add(header.obj_infos.len()))
        .ok_or_else(|| ctx.resource_limit_exceeded("max_work_units", limits.max_work_units))
}

fn append_string_metadata<'session>(
    parser: &mut ParseParser<'_, '_, '_, '_, 'session, 2, true>,
    metadata: &mut MeteredVec<'session, PendingMetadata<'session>>,
    key: &str,
    value: &str,
) -> ParseResult<()> {
    let ctx = &mut parser.services()?;
    let mut key_storage =
        ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-metadata-key");
    key_storage.try_push_str(ctx, key)?;
    let mut value_storage = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "ply-metadata-value",
    );
    value_storage.try_push_str(ctx, value)?;
    append_metadata_parts(parser, metadata, key_storage, value_storage)
}

fn append_indexed_string_metadata<'session>(
    parser: &mut ParseParser<'_, '_, '_, '_, 'session, 2, true>,
    metadata: &mut MeteredVec<'session, PendingMetadata<'session>>,
    prefix: &str,
    index: usize,
    value: &str,
) -> ParseResult<()> {
    let ctx = &mut parser.services()?;
    // Canonical NamespacedName local names do not admit dots; preserve the indexed
    // source value while normalizing only this structural separator.
    let mut key = ctx.metered_string(ImportStorageClass::FormatNormalization, "ply-metadata-key");
    key.try_push_str(ctx, prefix)?;
    key.try_push_usize(ctx, index)?;
    let mut value_storage = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "ply-metadata-value",
    );
    value_storage.try_push_str(ctx, value)?;
    append_metadata_parts(parser, metadata, key, value_storage)
}

fn append_metadata_parts<'session>(
    parser: &mut ParseParser<'_, '_, '_, '_, 'session, 2, true>,
    metadata: &mut MeteredVec<'session, PendingMetadata<'session>>,
    key: MeteredString<'session>,
    value: MeteredString<'session>,
) -> ParseResult<()> {
    parser.run([0, 1], || {
        metadata.try_push_reserved(PendingMetadata { key, value })
    })
}

fn order_metadata(
    children: &mut ParseChildren<'_, '_, '_>,
    metadata: &mut MeteredVec<'_, PendingMetadata<'_>>,
) -> ParseResult<()> {
    let group = children.bind_group([ImportWorkClass::WorkUnits])?;
    children.child(&group, [0], |parser| {
        let mut comparison_index = 0;
        metadata.try_sort_unstable_by(|left, right| -> ParseResult<_> {
            poll_collection(&mut parser.services()?, comparison_index)?;
            parser.replenish(1)?;
            // Generated keys have fixed prefixes and at most usize::MAX's decimal digits.
            let ordering = parser.run([1], || {
                Ok::<_, ParseFailure>(left.key.as_str().cmp(right.key.as_str()))
            })?;
            comparison_index = (comparison_index + 1) % MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS;
            Ok(ordering)
        })
    })
}

fn order_custom_attributes(
    children: &mut ParseChildren<'_, '_, '_>,
    attributes: &mut MeteredVec<'_, CustomAttributeStorage<'_>>,
) -> ParseResult<()> {
    let group = children.bind_group([ImportWorkClass::WorkUnits])?;
    children.child(&group, [0], |parser| {
        let mut comparison_index = 0;
        attributes.try_sort_unstable_by(|left, right| -> ParseResult<_> {
            poll_collection(&mut parser.services()?, comparison_index)?;
            parser.replenish(1)?;
            // Validated custom semantics contain at most 64 ASCII bytes.
            let ordering = parser.run([1], || {
                Ok::<_, ParseFailure>(left.semantic.cmp(&right.semantic))
            })?;
            comparison_index = (comparison_index + 1) % MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS;
            Ok(ordering)
        })
    })
}

fn canonical_string_bytes(
    ctx: &mut ParseServices<'_, '_, '_>,
    header: &Header<'_>,
) -> ParseResult<usize> {
    let limits = ctx.limits();
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
                limits.max_canonical_string_bytes,
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
                    limits.max_canonical_string_bytes,
                )
            })?;
        total = total
            .checked_add(key_len)
            .and_then(|value| value.checked_add(comment.len()))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    limits.max_canonical_string_bytes,
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
                    limits.max_canonical_string_bytes,
                )
            })?;
        total = total
            .checked_add(key_len)
            .and_then(|value| value.checked_add(info.len()))
            .ok_or_else(|| {
                resource_limit_failure(
                    ctx,
                    "max_canonical_string_bytes",
                    limits.max_canonical_string_bytes,
                )
            })?;
    }
    Ok(total)
}

fn poll_collection(ctx: &mut ParseServices<'_, '_, '_>, index: usize) -> ParseResult<()> {
    if index.is_multiple_of(MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS) {
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
