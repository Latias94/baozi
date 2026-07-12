use crate::config::{
    ConformancePolicy, DetectionOptions, DetectionOptionsError, FormatSelection,
    ImportConfigOverlay, ImportRequest, ImportRequestBuildError, ImportRequestLayer, LossPolicy,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
};
use crate::limits::ResourceLimitsOverlay;
use crate::options::{
    FormatOptionEnvelope, FormatOptionEnvelopeError, FormatOptionSchemaId,
    FormatOptionSchemaIdError, FormatOptionSchemaKey,
};
use baozi_core::{FormatId, FormatIdError};
use std::fmt;
use std::num::NonZeroU32;

const MAGIC: &[u8; 4] = b"BZIR";
const SCHEMA_VERSION: u16 = 1;
const TLV_HEADER_BYTES: u64 = 10;

/// Independent admission limits for the stable import-request wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestWireLimits {
    max_encoded_bytes: u64,
    max_envelopes: usize,
    max_total_envelope_payload_bytes: u64,
}

impl RequestWireLimits {
    pub const fn new(
        max_encoded_bytes: u64,
        max_envelopes: usize,
        max_total_envelope_payload_bytes: u64,
    ) -> Self {
        Self {
            max_encoded_bytes,
            max_envelopes,
            max_total_envelope_payload_bytes,
        }
    }

    pub const fn max_encoded_bytes(self) -> u64 {
        self.max_encoded_bytes
    }

    pub const fn max_envelopes(self) -> usize {
        self.max_envelopes
    }

    pub const fn max_total_envelope_payload_bytes(self) -> u64 {
        self.max_total_envelope_payload_bytes
    }
}

impl Default for RequestWireLimits {
    fn default() -> Self {
        Self::new(8 * 1024 * 1024, 64, 4 * 1024 * 1024)
    }
}

/// Nested object whose wire contract rejected an input field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestWireScope {
    Request,
    PresetLayer,
    CallerLayer,
    Config,
    ResourceLimits,
    FormatOptions,
    FormatOptionEnvelope,
}

/// Stable failure returned by BZIR v1 encoding and decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestWireError {
    AllocationFailed,
    EncodedSizeExceeded {
        actual: u64,
        maximum: u64,
    },
    EnvelopeCountExceeded {
        actual: usize,
        maximum: usize,
    },
    EnvelopePayloadBytesExceeded {
        actual: u64,
        maximum: u64,
    },
    InvalidMagic,
    UnsupportedVersion {
        actual: u16,
    },
    Truncated,
    TrailingBytes {
        scope: RequestWireScope,
        remaining: usize,
    },
    UnknownField {
        scope: RequestWireScope,
        field_id: u16,
    },
    DuplicateField {
        scope: RequestWireScope,
        field_id: u16,
    },
    OutOfOrderField {
        scope: RequestWireScope,
        previous: u16,
        field_id: u16,
    },
    MissingField {
        scope: RequestWireScope,
        field_id: u16,
    },
    InvalidFieldLength {
        scope: RequestWireScope,
        field_id: u16,
        actual: u64,
        expected: u64,
    },
    InvalidEnumValue {
        scope: RequestWireScope,
        field_id: u16,
        value: u64,
    },
    IntegerOutOfRange {
        scope: RequestWireScope,
        field_id: u16,
        value: u64,
    },
    IntegerEncodingOverflow {
        scope: RequestWireScope,
        field_id: u16,
    },
    InvalidUtf8 {
        scope: RequestWireScope,
        field_id: u16,
    },
    InvalidFormatId {
        field_id: u16,
        cause: FormatIdError,
    },
    InvalidDetectionOptions {
        cause: DetectionOptionsError,
    },
    InvalidSchemaId {
        field_id: u16,
        cause: FormatOptionSchemaIdError,
    },
    InvalidEnvelopeVersion,
    InvalidEnvelope {
        cause: FormatOptionEnvelopeError,
    },
    InvalidRequest {
        cause: ImportRequestBuildError,
    },
}

const _: () = assert!(std::mem::size_of::<RequestWireError>() < 128);
const _: () = assert!(!std::mem::needs_drop::<RequestWireError>());

impl RequestWireError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AllocationFailed => "allocation-failed",
            Self::EncodedSizeExceeded { .. } => "encoded-size-exceeded",
            Self::EnvelopeCountExceeded { .. } => "envelope-count-exceeded",
            Self::EnvelopePayloadBytesExceeded { .. } => "envelope-payload-bytes-exceeded",
            Self::InvalidMagic => "invalid-magic",
            Self::UnsupportedVersion { .. } => "unsupported-version",
            Self::Truncated => "truncated",
            Self::TrailingBytes { .. } => "trailing-bytes",
            Self::UnknownField { .. } => "unknown-field",
            Self::DuplicateField { .. } => "duplicate-field",
            Self::OutOfOrderField { .. } => "out-of-order-field",
            Self::MissingField { .. } => "missing-field",
            Self::InvalidFieldLength { .. } => "invalid-field-length",
            Self::InvalidEnumValue { .. } => "invalid-enum-value",
            Self::IntegerOutOfRange { .. } => "integer-out-of-range",
            Self::IntegerEncodingOverflow { .. } => "integer-encoding-overflow",
            Self::InvalidUtf8 { .. } => "invalid-utf8",
            Self::InvalidFormatId { .. } => "invalid-format-id",
            Self::InvalidDetectionOptions { .. } => "invalid-detection-options",
            Self::InvalidSchemaId { .. } => "invalid-schema-id",
            Self::InvalidEnvelopeVersion => "invalid-envelope-version",
            Self::InvalidEnvelope { .. } => "invalid-envelope",
            Self::InvalidRequest { .. } => "invalid-request",
        }
    }
}

impl fmt::Display for RequestWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("request wire allocation failed"),
            Self::EncodedSizeExceeded { actual, maximum } => write!(
                formatter,
                "encoded request is {actual} bytes; the maximum is {maximum}"
            ),
            Self::EnvelopeCountExceeded { actual, maximum } => write!(
                formatter,
                "request has {actual} format option envelopes; the maximum is {maximum}"
            ),
            Self::EnvelopePayloadBytesExceeded { actual, maximum } => write!(
                formatter,
                "request has {actual} format option payload bytes; the maximum is {maximum}"
            ),
            Self::InvalidMagic => formatter.write_str("request wire magic is not BZIR"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported request wire version {actual}")
            }
            Self::Truncated => formatter.write_str("request wire input is truncated"),
            Self::TrailingBytes { scope, remaining } => {
                write!(formatter, "{scope:?} has {remaining} trailing bytes")
            }
            Self::UnknownField { scope, field_id } => {
                write!(formatter, "unknown {scope:?} field {field_id}")
            }
            Self::DuplicateField { scope, field_id } => {
                write!(formatter, "duplicate {scope:?} field {field_id}")
            }
            Self::OutOfOrderField {
                scope,
                previous,
                field_id,
            } => write!(
                formatter,
                "{scope:?} field {field_id} follows field {previous} out of order"
            ),
            Self::MissingField { scope, field_id } => {
                write!(formatter, "missing {scope:?} field {field_id}")
            }
            Self::InvalidFieldLength {
                scope,
                field_id,
                actual,
                expected,
            } => write!(
                formatter,
                "{scope:?} field {field_id} is {actual} bytes; expected {expected}"
            ),
            Self::InvalidEnumValue {
                scope,
                field_id,
                value,
            } => write!(
                formatter,
                "invalid {scope:?} field {field_id} value {value}"
            ),
            Self::IntegerOutOfRange {
                scope,
                field_id,
                value,
            } => write!(
                formatter,
                "{scope:?} field {field_id} value {value} is out of range"
            ),
            Self::IntegerEncodingOverflow { scope, field_id } => {
                write!(
                    formatter,
                    "{scope:?} field {field_id} cannot be encoded as u64"
                )
            }
            Self::InvalidUtf8 { scope, field_id } => {
                write!(formatter, "{scope:?} field {field_id} is not UTF-8")
            }
            Self::InvalidFormatId { field_id, cause } => {
                write!(formatter, "invalid format id in field {field_id}: {cause}")
            }
            Self::InvalidDetectionOptions { cause } => {
                write!(formatter, "invalid detection options: {cause}")
            }
            Self::InvalidSchemaId { field_id, cause } => {
                write!(formatter, "invalid schema id in field {field_id}: {cause}")
            }
            Self::InvalidEnvelopeVersion => {
                formatter.write_str("format option envelope version must be nonzero")
            }
            Self::InvalidEnvelope { cause } => write!(formatter, "invalid envelope: {cause}"),
            Self::InvalidRequest { cause } => write!(formatter, "invalid request: {cause}"),
        }
    }
}

impl std::error::Error for RequestWireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidFormatId { cause, .. } => Some(cause),
            Self::InvalidDetectionOptions { cause } => Some(cause),
            Self::InvalidSchemaId { cause, .. } => Some(cause),
            Self::InvalidEnvelope { cause } => Some(cause),
            Self::InvalidRequest { cause } => Some(cause),
            _ => None,
        }
    }
}

/// Opaque canonical BZIR v1 representation of one import request.
#[derive(PartialEq, Eq)]
pub struct SerializedImportRequest {
    bytes: Vec<u8>,
}

impl fmt::Debug for SerializedImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SerializedImportRequest")
            .field("encoded_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl SerializedImportRequest {
    pub fn encode(
        request: &ImportRequest,
        limits: RequestWireLimits,
    ) -> Result<Self, RequestWireError> {
        check_request_limits(request, limits)?;
        let expected_size =
            encoded_request_size(request).ok_or(RequestWireError::EncodedSizeExceeded {
                actual: u64::MAX,
                maximum: limits.max_encoded_bytes,
            })?;
        check_encoded_size_u64(expected_size, limits)?;
        let preset = encode_layer(request.preset())?;
        let caller = encode_layer(request.caller())?;
        let mut root = ObjectEncoder::new()?;
        root.field(1, &preset)?;
        root.field(2, &caller)?;
        let root = root.finish();

        let mut bytes = Vec::new();
        append(&mut bytes, MAGIC)?;
        append(&mut bytes, &SCHEMA_VERSION.to_be_bytes())?;
        append(&mut bytes, &root)?;
        debug_assert_eq!(u64::try_from(bytes.len()), Ok(expected_size));
        Ok(Self { bytes })
    }

    pub fn decode(
        bytes: &[u8],
        limits: RequestWireLimits,
    ) -> Result<ImportRequest, RequestWireError> {
        check_encoded_size(bytes.len(), limits)?;
        decode_request(bytes, limits)
    }

    pub fn canonicalize(bytes: &[u8], limits: RequestWireLimits) -> Result<Self, RequestWireError> {
        let request = Self::decode(bytes, limits)?;
        Self::encode(&request, limits)
    }

    pub fn parse(bytes: &[u8], limits: RequestWireLimits) -> Result<Self, RequestWireError> {
        Self::canonicalize(bytes, limits)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn try_clone(&self) -> Result<Self, RequestWireError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.bytes.len())
            .map_err(|_| RequestWireError::AllocationFailed)?;
        bytes.extend_from_slice(&self.bytes);
        Ok(Self { bytes })
    }
}

fn check_encoded_size(length: usize, limits: RequestWireLimits) -> Result<(), RequestWireError> {
    let actual = u64::try_from(length).map_err(|_| RequestWireError::EncodedSizeExceeded {
        actual: u64::MAX,
        maximum: limits.max_encoded_bytes,
    })?;
    check_encoded_size_u64(actual, limits)
}

fn check_encoded_size_u64(actual: u64, limits: RequestWireLimits) -> Result<(), RequestWireError> {
    if actual > limits.max_encoded_bytes {
        return Err(RequestWireError::EncodedSizeExceeded {
            actual,
            maximum: limits.max_encoded_bytes,
        });
    }
    Ok(())
}

fn encoded_request_size(request: &ImportRequest) -> Option<u64> {
    let preset = encoded_layer_size(request.preset())?;
    let caller = encoded_layer_size(request.caller())?;
    let mut root = 2_u64;
    add_tlv_size(&mut root, preset)?;
    add_tlv_size(&mut root, caller)?;
    root.checked_add(6)
}

fn encoded_layer_size(layer: &ImportRequestLayer) -> Option<u64> {
    let mut size = 2_u64;
    if !layer.config().is_empty() {
        add_tlv_size(&mut size, encoded_config_size(layer.config())?)?;
    }
    if !layer.format_options().is_empty() {
        add_tlv_size(&mut size, encoded_envelopes_size(layer.format_options())?)?;
    }
    Some(size)
}

fn encoded_config_size(config: &ImportConfigOverlay) -> Option<u64> {
    let mut size = 2_u64;
    if !config.resource_limits.is_empty() {
        add_tlv_size(
            &mut size,
            encoded_resource_limits_size(&config.resource_limits)?,
        )?;
    }
    if config.conformance_policy.is_some() {
        add_tlv_size(&mut size, 1)?;
    }
    if config.loss_policy.is_some() {
        add_tlv_size(&mut size, 1)?;
    }
    if config.detection_options.is_some() {
        add_tlv_size(&mut size, 8)?;
    }
    if let Some(selection) = &config.format_selection {
        let selection_size = match selection {
            FormatSelection::Automatic => 1,
            FormatSelection::Preferred(format) | FormatSelection::Forced(format) => {
                u64::try_from(format.as_str().len()).ok()?.checked_add(1)?
            }
        };
        add_tlv_size(&mut size, selection_size)?;
    }
    Some(size)
}

fn encoded_resource_limits_size(limits: &ResourceLimitsOverlay) -> Option<u64> {
    let fields = [
        limits.max_primary_asset_bytes.is_some(),
        limits.max_dependency_asset_bytes.is_some(),
        limits.max_total_asset_bytes.is_some(),
        limits.max_open_assets.is_some(),
        limits.max_dependency_references.is_some(),
        limits.max_include_depth.is_some(),
        limits.max_data_uri_bytes.is_some(),
        limits.max_live_bytes.is_some(),
        limits.max_work_units.is_some(),
        limits.max_detection_work.is_some(),
        limits.max_decode_work.is_some(),
        limits.max_tokens.is_some(),
        limits.max_path_length.is_some(),
        limits.max_string_bytes.is_some(),
        limits.max_canonical_string_bytes.is_some(),
        limits.max_line_bytes.is_some(),
        limits.max_token_bytes.is_some(),
        limits.max_records.is_some(),
        limits.max_meshes.is_some(),
        limits.max_primitives.is_some(),
        limits.max_vertices.is_some(),
        limits.max_vertex_attribute_streams_per_mesh.is_some(),
        limits.max_total_vertex_attribute_streams.is_some(),
        limits.max_total_vertex_attribute_cells.is_some(),
        limits.max_total_topology_cells.is_some(),
        limits.max_faces.is_some(),
        limits.max_influence_cells.is_some(),
        limits.max_morph_cells.is_some(),
        limits.max_animation_cells.is_some(),
        limits.max_canonical_payload_bytes.is_some(),
        limits.max_solids.is_some(),
        limits.max_graph_nodes.is_some(),
        limits.max_graph_edges.is_some(),
        limits.max_graph_depth.is_some(),
        limits.max_graph_work.is_some(),
        limits.max_diagnostics.is_some(),
        limits.max_attempted_diagnostics.is_some(),
        limits.max_diagnostic_bytes.is_some(),
        limits.max_diagnostic_message_bytes.is_some(),
        limits.max_attempted_diagnostic_bytes.is_some(),
        limits.max_decoded_utf8_bytes.is_some(),
        limits.max_line_index_bytes.is_some(),
        limits.max_loss_evidence.is_some(),
        limits.max_attempted_recoveries.is_some(),
        limits.max_loss_evidence_bytes.is_some(),
        limits.max_attempted_recovery_bytes.is_some(),
    ];
    let count = u64::try_from(fields.into_iter().filter(|is_set| *is_set).count()).ok()?;
    count
        .checked_mul(TLV_HEADER_BYTES.checked_add(8)?)?
        .checked_add(2)
}

fn encoded_envelopes_size(envelopes: &[FormatOptionEnvelope]) -> Option<u64> {
    let mut size = 4_u64;
    for envelope in envelopes {
        size = size.checked_add(8)?;
        size = size.checked_add(encoded_envelope_size(envelope)?)?;
    }
    Some(size)
}

fn encoded_envelope_size(envelope: &FormatOptionEnvelope) -> Option<u64> {
    let mut size = 2_u64;
    for payload_size in [
        u64::try_from(envelope.target().as_str().len()).ok()?,
        u64::try_from(envelope.schema().owner().as_str().len()).ok()?,
        u64::try_from(envelope.schema().local().as_str().len()).ok()?,
        4,
        u64::try_from(envelope.payload_len()).ok()?,
    ] {
        add_tlv_size(&mut size, payload_size)?;
    }
    Some(size)
}

fn add_tlv_size(size: &mut u64, payload_size: u64) -> Option<()> {
    *size = size
        .checked_add(TLV_HEADER_BYTES)?
        .checked_add(payload_size)?;
    Some(())
}

fn check_request_limits(
    request: &ImportRequest,
    limits: RequestWireLimits,
) -> Result<(), RequestWireError> {
    let envelope_count = request
        .preset()
        .format_options()
        .len()
        .checked_add(request.caller().format_options().len())
        .ok_or(RequestWireError::EnvelopeCountExceeded {
            actual: usize::MAX,
            maximum: limits.max_envelopes,
        })?;
    if envelope_count > limits.max_envelopes {
        return Err(RequestWireError::EnvelopeCountExceeded {
            actual: envelope_count,
            maximum: limits.max_envelopes,
        });
    }
    let payload_bytes = request
        .preset()
        .format_options()
        .iter()
        .chain(request.caller().format_options())
        .try_fold(0_u64, |total, envelope| {
            let length = u64::try_from(envelope.payload_len()).ok()?;
            total.checked_add(length)
        })
        .ok_or(RequestWireError::EnvelopePayloadBytesExceeded {
            actual: u64::MAX,
            maximum: limits.max_total_envelope_payload_bytes,
        })?;
    if payload_bytes > limits.max_total_envelope_payload_bytes {
        return Err(RequestWireError::EnvelopePayloadBytesExceeded {
            actual: payload_bytes,
            maximum: limits.max_total_envelope_payload_bytes,
        });
    }
    Ok(())
}

fn encode_layer(layer: &ImportRequestLayer) -> Result<Vec<u8>, RequestWireError> {
    let mut object = ObjectEncoder::new()?;
    if !layer.config().is_empty() {
        object.field(1, &encode_config(layer.config())?)?;
    }
    if !layer.format_options().is_empty() {
        object.field(2, &encode_envelopes(layer.format_options())?)?;
    }
    Ok(object.finish())
}

fn encode_config(config: &ImportConfigOverlay) -> Result<Vec<u8>, RequestWireError> {
    let mut object = ObjectEncoder::new()?;
    if !config.resource_limits.is_empty() {
        object.field(1, &encode_resource_limits(&config.resource_limits)?)?;
    }
    if let Some(policy) = config.conformance_policy {
        let value = match policy {
            ConformancePolicy::Strict => 0,
            ConformancePolicy::AllowDeclaredRecovery => 1,
        };
        object.field(2, &[value])?;
    }
    if let Some(policy) = config.loss_policy {
        object.field(3, &[policy.bits()])?;
    }
    if let Some(options) = config.detection_options {
        object.field(4, &options.max_probe_bytes().to_be_bytes())?;
    }
    if let Some(selection) = &config.format_selection {
        object.field(5, &encode_format_selection(selection)?)?;
    }
    Ok(object.finish())
}

fn encode_format_selection(selection: &FormatSelection) -> Result<Vec<u8>, RequestWireError> {
    let format = match selection {
        FormatSelection::Automatic => None,
        FormatSelection::Preferred(format) | FormatSelection::Forced(format) => Some(format),
    };
    let length = format
        .map(|format| format.as_str().len())
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(RequestWireError::AllocationFailed)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(length)
        .map_err(|_| RequestWireError::AllocationFailed)?;
    encoded.push(match selection {
        FormatSelection::Automatic => 0,
        FormatSelection::Preferred(_) => 1,
        FormatSelection::Forced(_) => 2,
    });
    if let Some(format) = format {
        encoded.extend_from_slice(format.as_str().as_bytes());
    }
    Ok(encoded)
}

fn encode_resource_limits(limits: &ResourceLimitsOverlay) -> Result<Vec<u8>, RequestWireError> {
    let mut object = ObjectEncoder::new()?;

    macro_rules! u64_field {
        ($id:literal, $field:ident) => {
            if let Some(value) = limits.$field {
                object.field($id, &value.to_be_bytes())?;
            }
        };
    }
    macro_rules! usize_field {
        ($id:literal, $field:ident) => {
            if let Some(value) = limits.$field {
                let value = u64::try_from(value).map_err(|_| {
                    RequestWireError::IntegerEncodingOverflow {
                        scope: RequestWireScope::ResourceLimits,
                        field_id: $id,
                    }
                })?;
                object.field($id, &value.to_be_bytes())?;
            }
        };
    }

    u64_field!(1, max_primary_asset_bytes);
    u64_field!(2, max_dependency_asset_bytes);
    u64_field!(3, max_total_asset_bytes);
    u64_field!(4, max_open_assets);
    u64_field!(5, max_dependency_references);
    usize_field!(6, max_include_depth);
    u64_field!(7, max_data_uri_bytes);
    u64_field!(8, max_live_bytes);
    u64_field!(9, max_work_units);
    u64_field!(10, max_detection_work);
    u64_field!(11, max_decode_work);
    u64_field!(12, max_tokens);
    usize_field!(13, max_path_length);
    usize_field!(14, max_string_bytes);
    usize_field!(15, max_canonical_string_bytes);
    usize_field!(16, max_line_bytes);
    usize_field!(17, max_token_bytes);
    usize_field!(18, max_records);
    usize_field!(19, max_meshes);
    usize_field!(20, max_primitives);
    usize_field!(21, max_vertices);
    usize_field!(22, max_vertex_attribute_streams_per_mesh);
    usize_field!(23, max_total_vertex_attribute_streams);
    usize_field!(24, max_total_vertex_attribute_cells);
    usize_field!(25, max_total_topology_cells);
    usize_field!(26, max_faces);
    usize_field!(27, max_influence_cells);
    usize_field!(28, max_morph_cells);
    usize_field!(29, max_animation_cells);
    u64_field!(30, max_canonical_payload_bytes);
    usize_field!(31, max_solids);
    usize_field!(32, max_graph_nodes);
    usize_field!(33, max_graph_edges);
    usize_field!(34, max_graph_depth);
    usize_field!(35, max_graph_work);
    u64_field!(36, max_diagnostics);
    u64_field!(37, max_attempted_diagnostics);
    u64_field!(38, max_diagnostic_bytes);
    usize_field!(39, max_diagnostic_message_bytes);
    u64_field!(40, max_attempted_diagnostic_bytes);
    u64_field!(41, max_decoded_utf8_bytes);
    u64_field!(42, max_line_index_bytes);
    u64_field!(43, max_loss_evidence);
    u64_field!(44, max_attempted_recoveries);
    u64_field!(45, max_loss_evidence_bytes);
    u64_field!(46, max_attempted_recovery_bytes);

    Ok(object.finish())
}

fn encode_envelopes(envelopes: &[FormatOptionEnvelope]) -> Result<Vec<u8>, RequestWireError> {
    let mut ordered = Vec::new();
    ordered
        .try_reserve_exact(envelopes.len())
        .map_err(|_| RequestWireError::AllocationFailed)?;
    ordered.extend(envelopes);
    ordered.sort_unstable_by(|left, right| {
        left.target()
            .cmp(right.target())
            .then_with(|| left.schema().owner().cmp(right.schema().owner()))
            .then_with(|| left.schema().local().cmp(right.schema().local()))
            .then_with(|| left.version().cmp(&right.version()))
    });

    let mut bytes = Vec::new();
    let count = u32::try_from(ordered.len()).map_err(|_| {
        let maximum = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
        RequestWireError::EnvelopeCountExceeded {
            actual: ordered.len(),
            maximum,
        }
    })?;
    append(&mut bytes, &count.to_be_bytes())?;
    for envelope in ordered {
        let encoded = encode_envelope(envelope)?;
        let length =
            u64::try_from(encoded.len()).map_err(|_| RequestWireError::AllocationFailed)?;
        append(&mut bytes, &length.to_be_bytes())?;
        append(&mut bytes, &encoded)?;
    }
    Ok(bytes)
}

fn encode_envelope(envelope: &FormatOptionEnvelope) -> Result<Vec<u8>, RequestWireError> {
    let mut object = ObjectEncoder::new()?;
    object.field(1, envelope.target().as_str().as_bytes())?;
    object.field(2, envelope.schema().owner().as_str().as_bytes())?;
    object.field(3, envelope.schema().local().as_str().as_bytes())?;
    object.field(4, &envelope.version().get().to_be_bytes())?;
    object.field(5, envelope.canonical_payload_bytes())?;
    Ok(object.finish())
}

fn decode_request(
    bytes: &[u8],
    limits: RequestWireLimits,
) -> Result<ImportRequest, RequestWireError> {
    let mut header = Cursor::new(bytes);
    if header.take(MAGIC.len())? != MAGIC {
        return Err(RequestWireError::InvalidMagic);
    }
    let version = header.read_u16()?;
    if version != SCHEMA_VERSION {
        return Err(RequestWireError::UnsupportedVersion { actual: version });
    }

    let mut root = ObjectReader::new(header.remaining_bytes(), RequestWireScope::Request)?;
    let mut preset = None;
    let mut caller = None;
    let mut budget = DecodeBudget::default();
    while let Some((field_id, value)) = root.next()? {
        match field_id {
            1 => {
                preset = Some(decode_layer(
                    value,
                    RequestWireScope::PresetLayer,
                    limits,
                    &mut budget,
                )?);
            }
            2 => {
                caller = Some(decode_layer(
                    value,
                    RequestWireScope::CallerLayer,
                    limits,
                    &mut budget,
                )?);
            }
            _ => {
                return Err(RequestWireError::UnknownField {
                    scope: RequestWireScope::Request,
                    field_id,
                });
            }
        }
    }
    root.finish()?;
    let preset = preset.ok_or(RequestWireError::MissingField {
        scope: RequestWireScope::Request,
        field_id: 1,
    })?;
    let caller = caller.ok_or(RequestWireError::MissingField {
        scope: RequestWireScope::Request,
        field_id: 2,
    })?;
    Ok(ImportRequest::from_layers(preset, caller))
}

fn decode_layer(
    bytes: &[u8],
    scope: RequestWireScope,
    limits: RequestWireLimits,
    budget: &mut DecodeBudget,
) -> Result<ImportRequestLayer, RequestWireError> {
    let mut object = ObjectReader::new(bytes, scope)?;
    let mut config = None;
    let mut format_options = None;
    while let Some((field_id, value)) = object.next()? {
        match field_id {
            1 => config = Some(decode_config(value)?),
            2 => format_options = Some(value),
            _ => return Err(RequestWireError::UnknownField { scope, field_id }),
        }
    }
    object.finish()?;

    let mut layer = ImportRequestLayer::new(config.unwrap_or_default());
    if let Some(encoded) = format_options {
        decode_envelopes(encoded, limits, budget, &mut layer)?;
    }
    Ok(layer)
}

fn decode_config(bytes: &[u8]) -> Result<ImportConfigOverlay, RequestWireError> {
    let scope = RequestWireScope::Config;
    let mut object = ObjectReader::new(bytes, scope)?;
    let mut overlay = ImportConfigOverlay::default();
    while let Some((field_id, value)) = object.next()? {
        match field_id {
            1 => overlay.resource_limits = decode_resource_limits(value)?,
            2 => {
                overlay.conformance_policy = Some(match decode_byte(value, scope, field_id)? {
                    0 => ConformancePolicy::Strict,
                    1 => ConformancePolicy::AllowDeclaredRecovery,
                    value => {
                        return Err(RequestWireError::InvalidEnumValue {
                            scope,
                            field_id,
                            value: u64::from(value),
                        });
                    }
                });
            }
            3 => {
                let value = decode_byte(value, scope, field_id)?;
                overlay.loss_policy = Some(LossPolicy::from_bits(value).ok_or(
                    RequestWireError::InvalidEnumValue {
                        scope,
                        field_id,
                        value: u64::from(value),
                    },
                )?);
            }
            4 => {
                let max_probe_bytes = decode_u64(value, scope, field_id)?;
                overlay.detection_options = Some(
                    DetectionOptions::new(max_probe_bytes)
                        .map_err(|cause| RequestWireError::InvalidDetectionOptions { cause })?,
                );
            }
            5 => overlay.format_selection = Some(decode_format_selection(value)?),
            _ => return Err(RequestWireError::UnknownField { scope, field_id }),
        }
    }
    object.finish()?;
    Ok(overlay)
}

fn decode_format_selection(bytes: &[u8]) -> Result<FormatSelection, RequestWireError> {
    let scope = RequestWireScope::Config;
    let field_id = 5;
    let Some((&variant, format)) = bytes.split_first() else {
        return Err(RequestWireError::InvalidFieldLength {
            scope,
            field_id,
            actual: 0,
            expected: 1,
        });
    };
    match variant {
        0 => {
            expect_length(bytes, 1, scope, field_id)?;
            Ok(FormatSelection::Automatic)
        }
        1 => decode_format_id(format, scope, field_id).map(FormatSelection::Preferred),
        2 => decode_format_id(format, scope, field_id).map(FormatSelection::Forced),
        value => Err(RequestWireError::InvalidEnumValue {
            scope,
            field_id,
            value: u64::from(value),
        }),
    }
}

fn decode_resource_limits(bytes: &[u8]) -> Result<ResourceLimitsOverlay, RequestWireError> {
    let scope = RequestWireScope::ResourceLimits;
    let mut object = ObjectReader::new(bytes, scope)?;
    let mut limits = ResourceLimitsOverlay::default();
    while let Some((field_id, value)) = object.next()? {
        match field_id {
            1 => limits.max_primary_asset_bytes = Some(decode_u64(value, scope, field_id)?),
            2 => limits.max_dependency_asset_bytes = Some(decode_u64(value, scope, field_id)?),
            3 => limits.max_total_asset_bytes = Some(decode_u64(value, scope, field_id)?),
            4 => limits.max_open_assets = Some(decode_u64(value, scope, field_id)?),
            5 => limits.max_dependency_references = Some(decode_u64(value, scope, field_id)?),
            6 => limits.max_include_depth = Some(decode_usize(value, scope, field_id)?),
            7 => limits.max_data_uri_bytes = Some(decode_u64(value, scope, field_id)?),
            8 => limits.max_live_bytes = Some(decode_u64(value, scope, field_id)?),
            9 => limits.max_work_units = Some(decode_u64(value, scope, field_id)?),
            10 => limits.max_detection_work = Some(decode_u64(value, scope, field_id)?),
            11 => limits.max_decode_work = Some(decode_u64(value, scope, field_id)?),
            12 => limits.max_tokens = Some(decode_u64(value, scope, field_id)?),
            13 => limits.max_path_length = Some(decode_usize(value, scope, field_id)?),
            14 => limits.max_string_bytes = Some(decode_usize(value, scope, field_id)?),
            15 => limits.max_canonical_string_bytes = Some(decode_usize(value, scope, field_id)?),
            16 => limits.max_line_bytes = Some(decode_usize(value, scope, field_id)?),
            17 => limits.max_token_bytes = Some(decode_usize(value, scope, field_id)?),
            18 => limits.max_records = Some(decode_usize(value, scope, field_id)?),
            19 => limits.max_meshes = Some(decode_usize(value, scope, field_id)?),
            20 => limits.max_primitives = Some(decode_usize(value, scope, field_id)?),
            21 => limits.max_vertices = Some(decode_usize(value, scope, field_id)?),
            22 => {
                limits.max_vertex_attribute_streams_per_mesh =
                    Some(decode_usize(value, scope, field_id)?);
            }
            23 => {
                limits.max_total_vertex_attribute_streams =
                    Some(decode_usize(value, scope, field_id)?);
            }
            24 => {
                limits.max_total_vertex_attribute_cells =
                    Some(decode_usize(value, scope, field_id)?);
            }
            25 => {
                limits.max_total_topology_cells = Some(decode_usize(value, scope, field_id)?);
            }
            26 => limits.max_faces = Some(decode_usize(value, scope, field_id)?),
            27 => limits.max_influence_cells = Some(decode_usize(value, scope, field_id)?),
            28 => limits.max_morph_cells = Some(decode_usize(value, scope, field_id)?),
            29 => limits.max_animation_cells = Some(decode_usize(value, scope, field_id)?),
            30 => limits.max_canonical_payload_bytes = Some(decode_u64(value, scope, field_id)?),
            31 => limits.max_solids = Some(decode_usize(value, scope, field_id)?),
            32 => limits.max_graph_nodes = Some(decode_usize(value, scope, field_id)?),
            33 => limits.max_graph_edges = Some(decode_usize(value, scope, field_id)?),
            34 => limits.max_graph_depth = Some(decode_usize(value, scope, field_id)?),
            35 => limits.max_graph_work = Some(decode_usize(value, scope, field_id)?),
            36 => limits.max_diagnostics = Some(decode_u64(value, scope, field_id)?),
            37 => limits.max_attempted_diagnostics = Some(decode_u64(value, scope, field_id)?),
            38 => limits.max_diagnostic_bytes = Some(decode_u64(value, scope, field_id)?),
            39 => {
                limits.max_diagnostic_message_bytes = Some(decode_usize(value, scope, field_id)?);
            }
            40 => {
                limits.max_attempted_diagnostic_bytes = Some(decode_u64(value, scope, field_id)?);
            }
            41 => limits.max_decoded_utf8_bytes = Some(decode_u64(value, scope, field_id)?),
            42 => limits.max_line_index_bytes = Some(decode_u64(value, scope, field_id)?),
            43 => limits.max_loss_evidence = Some(decode_u64(value, scope, field_id)?),
            44 => limits.max_attempted_recoveries = Some(decode_u64(value, scope, field_id)?),
            45 => limits.max_loss_evidence_bytes = Some(decode_u64(value, scope, field_id)?),
            46 => limits.max_attempted_recovery_bytes = Some(decode_u64(value, scope, field_id)?),
            _ => return Err(RequestWireError::UnknownField { scope, field_id }),
        }
    }
    object.finish()?;
    Ok(limits)
}

fn decode_envelopes(
    bytes: &[u8],
    limits: RequestWireLimits,
    budget: &mut DecodeBudget,
    layer: &mut ImportRequestLayer,
) -> Result<(), RequestWireError> {
    let mut cursor = Cursor::new(bytes);
    let count_u32 = cursor.read_u32()?;
    let count = usize::try_from(count_u32).map_err(|_| RequestWireError::IntegerOutOfRange {
        scope: RequestWireScope::FormatOptions,
        field_id: 0,
        value: u64::from(count_u32),
    })?;
    if count > MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER {
        return Err(RequestWireError::EnvelopeCountExceeded {
            actual: count,
            maximum: MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
        });
    }
    budget.admit_envelopes(count, limits)?;
    for _ in 0..count {
        let length = cursor.read_u64()?;
        let encoded = cursor.take_length(length, RequestWireScope::FormatOptions, 0)?;
        let envelope = decode_envelope(encoded, limits, budget)?;
        layer
            .try_push_format_option(envelope)
            .map_err(|cause| RequestWireError::InvalidRequest { cause })?;
    }
    if cursor.remaining() != 0 {
        return Err(RequestWireError::TrailingBytes {
            scope: RequestWireScope::FormatOptions,
            remaining: cursor.remaining(),
        });
    }
    Ok(())
}

fn decode_envelope(
    bytes: &[u8],
    limits: RequestWireLimits,
    budget: &mut DecodeBudget,
) -> Result<FormatOptionEnvelope, RequestWireError> {
    let scope = RequestWireScope::FormatOptionEnvelope;
    let mut object = ObjectReader::new(bytes, scope)?;
    let mut target = None;
    let mut owner = None;
    let mut local = None;
    let mut version = None;
    let mut payload = None;
    while let Some((field_id, value)) = object.next()? {
        match field_id {
            1 => target = Some(decode_format_id(value, scope, field_id)?),
            2 => owner = Some(decode_format_id(value, scope, field_id)?),
            3 => local = Some(decode_schema_id(value, scope, field_id)?),
            4 => {
                let raw = decode_u32(value, scope, field_id)?;
                version =
                    Some(NonZeroU32::new(raw).ok_or(RequestWireError::InvalidEnvelopeVersion)?);
            }
            5 => payload = Some(value),
            _ => return Err(RequestWireError::UnknownField { scope, field_id }),
        }
    }
    object.finish()?;

    let target = required(target, scope, 1)?;
    let owner = required(owner, scope, 2)?;
    let local = required(local, scope, 3)?;
    let version = required(version, scope, 4)?;
    let payload = required(payload, scope, 5)?;
    budget.admit_payload(payload.len(), limits)?;
    FormatOptionEnvelope::new(
        target,
        FormatOptionSchemaKey::new(owner, local),
        version,
        payload,
    )
    .map_err(|cause| RequestWireError::InvalidEnvelope { cause })
}

fn required<T>(
    value: Option<T>,
    scope: RequestWireScope,
    field_id: u16,
) -> Result<T, RequestWireError> {
    value.ok_or(RequestWireError::MissingField { scope, field_id })
}

fn decode_format_id(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<FormatId, RequestWireError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| RequestWireError::InvalidUtf8 { scope, field_id })?;
    FormatId::new(value).map_err(|cause| RequestWireError::InvalidFormatId { field_id, cause })
}

fn decode_schema_id(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<FormatOptionSchemaId, RequestWireError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| RequestWireError::InvalidUtf8 { scope, field_id })?;
    FormatOptionSchemaId::new(value)
        .map_err(|cause| RequestWireError::InvalidSchemaId { field_id, cause })
}

fn decode_byte(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<u8, RequestWireError> {
    expect_length(bytes, 1, scope, field_id)?;
    Ok(bytes[0])
}

fn decode_u32(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<u32, RequestWireError> {
    expect_length(bytes, 4, scope, field_id)?;
    Ok(u32::from_be_bytes(
        bytes.try_into().expect("length was checked"),
    ))
}

fn decode_u64(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<u64, RequestWireError> {
    expect_length(bytes, 8, scope, field_id)?;
    Ok(u64::from_be_bytes(
        bytes.try_into().expect("length was checked"),
    ))
}

fn decode_usize(
    bytes: &[u8],
    scope: RequestWireScope,
    field_id: u16,
) -> Result<usize, RequestWireError> {
    let value = decode_u64(bytes, scope, field_id)?;
    usize::try_from(value).map_err(|_| RequestWireError::IntegerOutOfRange {
        scope,
        field_id,
        value,
    })
}

fn expect_length(
    bytes: &[u8],
    expected: usize,
    scope: RequestWireScope,
    field_id: u16,
) -> Result<(), RequestWireError> {
    if bytes.len() == expected {
        return Ok(());
    }
    Err(RequestWireError::InvalidFieldLength {
        scope,
        field_id,
        actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        expected: u64::try_from(expected).unwrap_or(u64::MAX),
    })
}

#[derive(Default)]
struct DecodeBudget {
    envelopes: usize,
    payload_bytes: u64,
}

impl DecodeBudget {
    fn admit_envelopes(
        &mut self,
        count: usize,
        limits: RequestWireLimits,
    ) -> Result<(), RequestWireError> {
        let next =
            self.envelopes
                .checked_add(count)
                .ok_or(RequestWireError::EnvelopeCountExceeded {
                    actual: usize::MAX,
                    maximum: limits.max_envelopes,
                })?;
        if next > limits.max_envelopes {
            return Err(RequestWireError::EnvelopeCountExceeded {
                actual: next,
                maximum: limits.max_envelopes,
            });
        }
        self.envelopes = next;
        Ok(())
    }

    fn admit_payload(
        &mut self,
        length: usize,
        limits: RequestWireLimits,
    ) -> Result<(), RequestWireError> {
        let length =
            u64::try_from(length).map_err(|_| RequestWireError::EnvelopePayloadBytesExceeded {
                actual: u64::MAX,
                maximum: limits.max_total_envelope_payload_bytes,
            })?;
        let next = self.payload_bytes.checked_add(length).ok_or(
            RequestWireError::EnvelopePayloadBytesExceeded {
                actual: u64::MAX,
                maximum: limits.max_total_envelope_payload_bytes,
            },
        )?;
        if next > limits.max_total_envelope_payload_bytes {
            return Err(RequestWireError::EnvelopePayloadBytesExceeded {
                actual: next,
                maximum: limits.max_total_envelope_payload_bytes,
            });
        }
        self.payload_bytes = next;
        Ok(())
    }
}

struct ObjectEncoder {
    bytes: Vec<u8>,
    field_count: u16,
}

impl ObjectEncoder {
    fn new() -> Result<Self, RequestWireError> {
        let mut bytes = Vec::new();
        append(&mut bytes, &0_u16.to_be_bytes())?;
        Ok(Self {
            bytes,
            field_count: 0,
        })
    }

    fn field(&mut self, field_id: u16, payload: &[u8]) -> Result<(), RequestWireError> {
        self.field_count = self
            .field_count
            .checked_add(1)
            .ok_or(RequestWireError::AllocationFailed)?;
        append(&mut self.bytes, &field_id.to_be_bytes())?;
        let length =
            u64::try_from(payload.len()).map_err(|_| RequestWireError::AllocationFailed)?;
        append(&mut self.bytes, &length.to_be_bytes())?;
        append(&mut self.bytes, payload)
    }

    fn finish(mut self) -> Vec<u8> {
        self.bytes[..2].copy_from_slice(&self.field_count.to_be_bytes());
        self.bytes
    }
}

fn append(target: &mut Vec<u8>, bytes: &[u8]) -> Result<(), RequestWireError> {
    target
        .try_reserve_exact(bytes.len())
        .map_err(|_| RequestWireError::AllocationFailed)?;
    target.extend_from_slice(bytes);
    Ok(())
}

struct ObjectReader<'a> {
    cursor: Cursor<'a>,
    remaining_fields: u16,
    previous: Option<u16>,
    scope: RequestWireScope,
}

impl<'a> ObjectReader<'a> {
    fn new(bytes: &'a [u8], scope: RequestWireScope) -> Result<Self, RequestWireError> {
        let mut cursor = Cursor::new(bytes);
        let remaining_fields = cursor.read_u16()?;
        Ok(Self {
            cursor,
            remaining_fields,
            previous: None,
            scope,
        })
    }

    fn next(&mut self) -> Result<Option<(u16, &'a [u8])>, RequestWireError> {
        if self.remaining_fields == 0 {
            return Ok(None);
        }
        let field_id = self.cursor.read_u16()?;
        if let Some(previous) = self.previous {
            if field_id == previous {
                return Err(RequestWireError::DuplicateField {
                    scope: self.scope,
                    field_id,
                });
            }
            if field_id < previous {
                return Err(RequestWireError::OutOfOrderField {
                    scope: self.scope,
                    previous,
                    field_id,
                });
            }
        }
        let length = self.cursor.read_u64()?;
        let value = self.cursor.take_length(length, self.scope, field_id)?;
        self.previous = Some(field_id);
        self.remaining_fields -= 1;
        Ok(Some((field_id, value)))
    }

    fn finish(self) -> Result<(), RequestWireError> {
        if self.cursor.remaining() != 0 {
            return Err(RequestWireError::TrailingBytes {
                scope: self.scope,
                remaining: self.cursor.remaining(),
            });
        }
        Ok(())
    }
}

struct Cursor<'a> {
    remaining: &'a [u8],
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    const fn remaining(&self) -> usize {
        self.remaining.len()
    }

    const fn remaining_bytes(&self) -> &'a [u8] {
        self.remaining
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], RequestWireError> {
        if length > self.remaining.len() {
            return Err(RequestWireError::Truncated);
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn take_length(
        &mut self,
        length: u64,
        scope: RequestWireScope,
        field_id: u16,
    ) -> Result<&'a [u8], RequestWireError> {
        let length = usize::try_from(length).map_err(|_| RequestWireError::IntegerOutOfRange {
            scope,
            field_id,
            value: length,
        })?;
        self.take(length)
    }

    fn read_u16(&mut self) -> Result<u16, RequestWireError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("length is fixed"),
        ))
    }

    fn read_u32(&mut self) -> Result<u32, RequestWireError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("length is fixed"),
        ))
    }

    fn read_u64(&mut self) -> Result<u64, RequestWireError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("length is fixed"),
        ))
    }
}

const _: () = assert!(TLV_HEADER_BYTES == 2 + 8);
