use baozi_core::FormatId;
use baozi_import::{
    ConformancePolicy, DetectionOptions, FormatOptionEnvelope, FormatOptionSchemaId,
    FormatOptionSchemaKey, FormatSelection, ImportConfig, ImportConfigOverlay, ImportLossKind,
    ImportPreset, ImportRequest, ImportRequestBuildError, ImportRequestLayer, LossPolicy,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER, RequestWireError, RequestWireLimits, RequestWireScope,
    ResourceLimits, ResourceLimitsOverlay, SerializedImportRequest,
};
use std::num::NonZeroU32;

#[test]
fn common_config_merges_defaults_preset_and_caller_per_field() {
    let defaults = ResourceLimits::default();
    let mut preset_limits = ResourceLimitsOverlay::default();
    preset_limits.max_tokens = Some(17);
    preset_limits.max_graph_depth = Some(9);
    preset_limits.max_decoded_utf8_bytes = Some(4_096);
    preset_limits.max_line_index_bytes = Some(512);
    preset_limits.max_loss_evidence = Some(7);
    let preset = ImportConfigOverlay::default()
        .with_resource_limits(preset_limits)
        .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
        .with_loss_policy(LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery))
        .with_detection_options(DetectionOptions::new(8).unwrap())
        .with_format_selection(FormatSelection::Preferred(
            FormatId::new("org.example.preferred").unwrap(),
        ));

    let mut caller_limits = ResourceLimitsOverlay::default();
    caller_limits.max_tokens = Some(defaults.max_tokens);
    caller_limits.max_decoded_utf8_bytes = Some(defaults.max_decoded_utf8_bytes);
    let caller = ImportConfigOverlay::default()
        .with_resource_limits(caller_limits)
        .with_conformance_policy(ConformancePolicy::Strict)
        .with_loss_policy(LossPolicy::preserve_or_fail())
        .with_detection_options(DetectionOptions::default())
        .with_format_selection(FormatSelection::Automatic);

    let resolved = ImportConfig::resolve(&preset, &caller);

    assert_eq!(resolved.resource_limits().max_tokens, defaults.max_tokens);
    assert_eq!(resolved.resource_limits().max_graph_depth, 9);
    assert_eq!(
        resolved.resource_limits().max_decoded_utf8_bytes,
        defaults.max_decoded_utf8_bytes
    );
    assert_eq!(resolved.resource_limits().max_line_index_bytes, 512);
    assert_eq!(resolved.resource_limits().max_loss_evidence, 7);
    assert_eq!(resolved.conformance_policy(), ConformancePolicy::Strict);
    assert_eq!(resolved.loss_policy(), LossPolicy::preserve_or_fail());
    assert_eq!(resolved.detection_options(), &DetectionOptions::default());
    assert_eq!(resolved.format_selection(), &FormatSelection::Automatic);
    assert!(
        !resolved
            .loss_policy()
            .allows(ImportLossKind::FormatRecovery)
    );
}

fn envelope(target: &str, local_schema: &str, payload: &[u8]) -> FormatOptionEnvelope {
    let target = FormatId::new(target).unwrap();
    FormatOptionEnvelope::new(
        target.clone(),
        FormatOptionSchemaKey::new(target, FormatOptionSchemaId::new(local_schema).unwrap()),
        NonZeroU32::new(1).unwrap(),
        payload,
    )
    .unwrap()
}

#[test]
fn preset_expansion_is_reusable_fallible_and_bounded() {
    let preset = ImportPreset::new(ImportConfigOverlay::default())
        .try_with_format_option(envelope("org.example.z", "options", &[9]))
        .unwrap()
        .try_with_format_option(envelope("org.example.a", "options", &[1]))
        .unwrap();

    let first = preset.try_expand().unwrap();
    let second = preset.try_expand().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .format_options()
            .iter()
            .map(|option| option.target().as_str())
            .collect::<Vec<_>>(),
        ["org.example.a", "org.example.z"]
    );

    let mut layer = ImportRequestLayer::default();
    for index in 0..MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER {
        layer
            .try_push_format_option(envelope(
                &format!("org.example.format-{index}"),
                "options",
                &[],
            ))
            .unwrap();
    }
    assert!(matches!(
        layer.try_push_format_option(envelope("org.example.overflow", "options", &[])),
        Err(ImportRequestBuildError::FormatOptionEnvelopeCountExceeded {
            maximum: MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
        })
    ));
}

fn empty_request_wire() -> Vec<u8> {
    vec![
        b'B', b'Z', b'I', b'R', 0, 1, // Header.
        0, 2, // Root field count.
        0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, // Empty preset layer.
        0, 2, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, // Empty caller layer.
    ]
}

#[test]
fn empty_request_has_a_canonical_experimental_bzir_v1_vector() {
    let encoded =
        SerializedImportRequest::encode(&ImportRequest::new(), RequestWireLimits::default())
            .unwrap();

    assert_eq!(encoded.as_bytes(), empty_request_wire());
}

#[test]
fn request_wire_roundtrip_is_semantic_and_canonicalizes_envelope_order() {
    let mut preset_limits = ResourceLimitsOverlay::default();
    preset_limits.max_graph_depth = Some(7);
    let preset_config = ImportConfigOverlay::default()
        .with_resource_limits(preset_limits)
        .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
        .with_detection_options(DetectionOptions::new(32).unwrap())
        .with_format_selection(FormatSelection::Preferred(
            FormatId::new("org.example.z").unwrap(),
        ));
    let preset = ImportRequestLayer::new(preset_config)
        .try_with_format_option(envelope("org.example.z", "options", &[9, 8]))
        .unwrap()
        .try_with_format_option(envelope("org.example.a", "options", &[1]))
        .unwrap();
    let caller = ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_loss_policy(LossPolicy::preserve_or_fail())
            .with_format_selection(FormatSelection::Forced(
                FormatId::new("org.example.a").unwrap(),
            )),
    );
    let request = ImportRequest::from_layers(preset, caller);

    let encoded = SerializedImportRequest::encode(&request, RequestWireLimits::default()).unwrap();
    let decoded =
        SerializedImportRequest::decode(encoded.as_bytes(), RequestWireLimits::default()).unwrap();
    let reencoded =
        SerializedImportRequest::encode(&decoded, RequestWireLimits::default()).unwrap();

    assert_eq!(decoded, request);
    assert_eq!(reencoded, encoded);
    assert_eq!(
        decoded
            .preset()
            .format_options()
            .iter()
            .map(|option| option.target().as_str())
            .collect::<Vec<_>>(),
        ["org.example.a", "org.example.z"]
    );
}

#[test]
fn request_wire_roundtrips_each_loss_authority_independently() {
    let kinds = [
        ImportLossKind::OptionalDependencyUnavailable,
        ImportLossKind::FormatRecovery,
        ImportLossKind::SourceSemanticLoss,
        ImportLossKind::ProvenanceEvidenceLoss,
    ];

    for allowed in kinds {
        let policy = LossPolicy::preserve_or_fail().allow(allowed);
        let request = ImportRequest::from_layers(
            ImportRequestLayer::new(ImportConfigOverlay::default().with_loss_policy(policy)),
            ImportRequestLayer::default(),
        );
        let encoded =
            SerializedImportRequest::encode(&request, RequestWireLimits::default()).unwrap();
        let decoded =
            SerializedImportRequest::decode(encoded.as_bytes(), RequestWireLimits::default())
                .unwrap();
        let decoded_policy = decoded.preset().config().loss_policy.unwrap();

        for kind in kinds {
            assert_eq!(decoded_policy.allows(kind), kind == allowed);
        }
    }
}

fn all_resource_limit_fields() -> ResourceLimitsOverlay {
    let mut overlay = ResourceLimitsOverlay::default();
    overlay.max_primary_asset_bytes = Some(1);
    overlay.max_dependency_asset_bytes = Some(2);
    overlay.max_total_asset_bytes = Some(3);
    overlay.max_open_assets = Some(4);
    overlay.max_dependency_references = Some(5);
    overlay.max_include_depth = Some(6);
    overlay.max_data_uri_bytes = Some(7);
    overlay.max_live_bytes = Some(8);
    overlay.max_work_units = Some(9);
    overlay.max_detection_work = Some(10);
    overlay.max_decode_work = Some(11);
    overlay.max_tokens = Some(12);
    overlay.max_path_length = Some(13);
    overlay.max_string_bytes = Some(14);
    overlay.max_canonical_string_bytes = Some(15);
    overlay.max_line_bytes = Some(16);
    overlay.max_token_bytes = Some(17);
    overlay.max_records = Some(18);
    overlay.max_meshes = Some(19);
    overlay.max_primitives = Some(20);
    overlay.max_vertices = Some(21);
    overlay.max_vertex_attribute_streams_per_mesh = Some(22);
    overlay.max_total_vertex_attribute_streams = Some(23);
    overlay.max_total_vertex_attribute_cells = Some(24);
    overlay.max_total_topology_cells = Some(25);
    overlay.max_faces = Some(26);
    overlay.max_influence_cells = Some(27);
    overlay.max_morph_cells = Some(28);
    overlay.max_animation_cells = Some(29);
    overlay.max_canonical_payload_bytes = Some(30);
    overlay.max_solids = Some(31);
    overlay.max_graph_nodes = Some(32);
    overlay.max_graph_edges = Some(33);
    overlay.max_graph_depth = Some(34);
    overlay.max_graph_work = Some(35);
    overlay.max_diagnostics = Some(36);
    overlay.max_attempted_diagnostics = Some(37);
    overlay.max_diagnostic_bytes = Some(38);
    overlay.max_diagnostic_message_bytes = Some(39);
    overlay.max_attempted_diagnostic_bytes = Some(40);
    overlay.max_decoded_utf8_bytes = Some(41);
    overlay.max_line_index_bytes = Some(42);
    overlay.max_loss_evidence = Some(43);
    overlay.max_attempted_recoveries = Some(44);
    overlay.max_loss_evidence_bytes = Some(45);
    overlay.max_attempted_recovery_bytes = Some(46);
    overlay.max_source_scope_records = Some(47);
    overlay.max_source_scope_items = Some(48);
    overlay.max_source_scope_detail_count = Some(49);
    overlay.max_source_scope_detail_bytes = Some(50);
    overlay.max_source_scope_digest_bytes = Some(51);
    overlay
}

#[test]
fn every_resource_limit_field_roundtrips_as_a_canonical_u64() {
    let overlay = all_resource_limit_fields();
    let request = ImportRequest::from_layers(
        ImportRequestLayer::new(
            ImportConfigOverlay::default().with_resource_limits(overlay.clone()),
        ),
        ImportRequestLayer::default(),
    );

    let encoded = SerializedImportRequest::encode(&request, RequestWireLimits::default()).unwrap();
    let resource_fields = (1_u16..=51)
        .map(|field_id| (field_id, u64::from(field_id).to_be_bytes().to_vec()))
        .collect::<Vec<_>>();
    let resource_limits = object(&resource_fields);
    let config = object(&[(1, resource_limits)]);
    let preset = object(&[(1, config)]);
    let caller = object(&[]);
    let root = object(&[(1, preset), (2, caller)]);
    let mut expected = b"BZIR".to_vec();
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&root);

    assert_eq!(encoded.as_bytes(), expected);
    assert_eq!(encoded.try_clone().unwrap(), encoded);
    let decoded =
        SerializedImportRequest::decode(encoded.as_bytes(), RequestWireLimits::default()).unwrap();

    assert_eq!(decoded.preset().config().resource_limits, overlay);
}

#[test]
fn request_wire_rejects_unknown_duplicate_and_out_of_order_fields() {
    let canonical = empty_request_wire();

    let mut unknown = canonical.clone();
    unknown[21] = 3;
    assert!(matches!(
        SerializedImportRequest::decode(&unknown, RequestWireLimits::default()),
        Err(RequestWireError::UnknownField {
            scope: RequestWireScope::Request,
            field_id: 3,
        })
    ));

    let mut duplicate = canonical.clone();
    duplicate[21] = 1;
    assert!(matches!(
        SerializedImportRequest::decode(&duplicate, RequestWireLimits::default()),
        Err(RequestWireError::DuplicateField {
            scope: RequestWireScope::Request,
            field_id: 1,
        })
    ));

    let mut out_of_order = canonical;
    out_of_order[9] = 2;
    out_of_order[21] = 1;
    assert!(matches!(
        SerializedImportRequest::decode(&out_of_order, RequestWireLimits::default()),
        Err(RequestWireError::OutOfOrderField {
            scope: RequestWireScope::Request,
            previous: 2,
            field_id: 1,
        })
    ));
}

#[test]
fn request_wire_rejects_invalid_magic_and_schema_version() {
    let canonical = empty_request_wire();
    let mut invalid_magic = canonical.clone();
    invalid_magic[0] = b'X';
    assert_eq!(
        SerializedImportRequest::decode(&invalid_magic, RequestWireLimits::default()),
        Err(RequestWireError::InvalidMagic)
    );

    let mut unsupported = canonical;
    unsupported[5] = 2;
    assert_eq!(
        SerializedImportRequest::decode(&unsupported, RequestWireLimits::default()),
        Err(RequestWireError::UnsupportedVersion { actual: 2 })
    );
}

#[test]
fn request_wire_rejects_truncation_trailing_bytes_and_size_excess() {
    let canonical = empty_request_wire();
    assert_eq!(
        SerializedImportRequest::decode(
            &canonical[..canonical.len() - 1],
            RequestWireLimits::default(),
        ),
        Err(RequestWireError::Truncated)
    );

    let mut trailing = canonical.clone();
    trailing.push(0);
    assert!(matches!(
        SerializedImportRequest::decode(&trailing, RequestWireLimits::default()),
        Err(RequestWireError::TrailingBytes {
            scope: RequestWireScope::Request,
            remaining: 1,
        })
    ));

    let limits = RequestWireLimits::new(
        u64::try_from(canonical.len() - 1).unwrap(),
        64,
        4 * 1024 * 1024,
    );
    assert!(matches!(
        SerializedImportRequest::decode(&canonical, limits),
        Err(RequestWireError::EncodedSizeExceeded {
            actual: 32,
            maximum: 31,
        })
    ));
    assert!(matches!(
        SerializedImportRequest::encode(&ImportRequest::new(), limits),
        Err(RequestWireError::EncodedSizeExceeded {
            actual: 32,
            maximum: 31,
        })
    ));
}

#[test]
fn request_wire_enforces_collection_and_payload_limits_before_encoding() {
    let request = ImportRequest::from_layers(
        ImportRequestLayer::default()
            .try_with_format_option(envelope("org.example.a", "options", &[1, 2]))
            .unwrap(),
        ImportRequestLayer::default(),
    );

    assert!(matches!(
        SerializedImportRequest::encode(&request, RequestWireLimits::new(1024, 0, 4 * 1024 * 1024),),
        Err(RequestWireError::EnvelopeCountExceeded {
            actual: 1,
            maximum: 0,
        })
    ));
    assert!(matches!(
        SerializedImportRequest::encode(&request, RequestWireLimits::new(1024, 64, 1)),
        Err(RequestWireError::EnvelopePayloadBytesExceeded {
            actual: 2,
            maximum: 1,
        })
    ));

    let encoded = SerializedImportRequest::encode(&request, RequestWireLimits::default()).unwrap();
    assert!(matches!(
        SerializedImportRequest::decode(
            encoded.as_bytes(),
            RequestWireLimits::new(1024, 0, 4 * 1024 * 1024),
        ),
        Err(RequestWireError::EnvelopeCountExceeded {
            actual: 1,
            maximum: 0,
        })
    ));
    assert!(matches!(
        SerializedImportRequest::decode(encoded.as_bytes(), RequestWireLimits::new(1024, 64, 1),),
        Err(RequestWireError::EnvelopePayloadBytesExceeded {
            actual: 2,
            maximum: 1,
        })
    ));
}

fn tlv(field_id: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&field_id.to_be_bytes());
    bytes.extend_from_slice(&u64::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn object(fields: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u16::try_from(fields.len()).unwrap().to_be_bytes());
    for (field_id, payload) in fields {
        bytes.extend_from_slice(&tlv(*field_id, payload));
    }
    bytes
}

fn raw_wire_envelope(target: &str, payload: &[u8]) -> Vec<u8> {
    object(&[
        (1, target.as_bytes().to_vec()),
        (2, target.as_bytes().to_vec()),
        (3, b"options".to_vec()),
        (4, 1_u32.to_be_bytes().to_vec()),
        (5, payload.to_vec()),
    ])
}

fn raw_envelope_collection(envelopes: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::try_from(envelopes.len()).unwrap().to_be_bytes());
    for envelope in envelopes {
        bytes.extend_from_slice(&u64::try_from(envelope.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(envelope);
    }
    bytes
}

#[test]
fn decode_then_encode_canonicalizes_noncanonical_envelope_order() {
    let collection = raw_envelope_collection(&[
        raw_wire_envelope("org.example.z", &[9]),
        raw_wire_envelope("org.example.a", &[1]),
    ]);
    let preset = object(&[(2, collection)]);
    let caller = object(&[]);
    let root = object(&[(1, preset), (2, caller)]);
    let mut bytes = b"BZIR".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&root);

    let canonical =
        SerializedImportRequest::canonicalize(&bytes, RequestWireLimits::default()).unwrap();
    let decoded =
        SerializedImportRequest::decode(canonical.as_bytes(), RequestWireLimits::default())
            .unwrap();

    assert_ne!(canonical.as_bytes(), bytes);
    assert_eq!(
        decoded
            .preset()
            .format_options()
            .iter()
            .map(|option| option.target().as_str())
            .collect::<Vec<_>>(),
        ["org.example.a", "org.example.z"]
    );
}

#[test]
fn usize_resource_limits_use_checked_u64_decoding() {
    let resource_limits = object(&[(34, u64::MAX.to_be_bytes().to_vec())]);
    let config = object(&[(1, resource_limits)]);
    let preset = object(&[(1, config)]);
    let caller = object(&[]);
    let root = object(&[(1, preset), (2, caller)]);
    let mut bytes = b"BZIR".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&root);

    let decoded = SerializedImportRequest::decode(&bytes, RequestWireLimits::default());
    if usize::BITS < u64::BITS {
        assert!(matches!(
            decoded,
            Err(RequestWireError::IntegerOutOfRange {
                scope: RequestWireScope::ResourceLimits,
                field_id: 34,
                value: u64::MAX,
            })
        ));
    } else {
        assert_eq!(
            decoded
                .unwrap()
                .preset()
                .config()
                .resource_limits
                .max_graph_depth,
            Some(usize::MAX)
        );
    }
}

#[test]
fn request_wire_rejects_unknown_loss_policy_bits() {
    let config = object(&[(3, vec![1 << 4])]);
    let preset = object(&[(1, config)]);
    let caller = object(&[]);
    let root = object(&[(1, preset), (2, caller)]);
    let mut bytes = b"BZIR".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&root);

    assert!(matches!(
        SerializedImportRequest::decode(&bytes, RequestWireLimits::default()),
        Err(RequestWireError::InvalidEnumValue {
            scope: RequestWireScope::Config,
            field_id: 3,
            value: 16,
        })
    ));
}
