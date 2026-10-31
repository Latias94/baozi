use baozi_core::FormatId;
use baozi_import::{
    ConformancePolicy, DetectionOptions, FormatOptionEnvelope, FormatOptionSchemaId,
    FormatOptionSchemaKey, FormatSelection, ImportConfig, ImportConfigOverlay, ImportLossKind,
    ImportPreset, ImportRequestBuildError, ImportRequestLayer, LossPolicy,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER, ResourceLimits, ResourceLimitsOverlay,
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
