#![cfg(feature = "format-obj")]

use baozi::{
    ConformancePolicy, FormatOptionEnvelope, FormatOptionSchemaId, FormatOptionSchemaKey,
    ImportConfigOverlay, ImportLossKind, ImportOperationControl, ImportPreset, ImportRequest,
    ImportRequestLayer, Importer, LossPolicy, ObjImportOptions, ObjTextDecodingMode,
    OptionCompileLimits,
};
use std::num::NonZeroU32;

const INVALID_NAME: &[u8] = b"o invalid-\xff\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

#[test]
fn facade_only_obj_options_apply_caller_precedence_over_a_preset() {
    let importer = Importer::new().unwrap();
    let permissive = ObjImportOptions::default()
        .with_text_decoding_mode(ObjTextDecodingMode::ReplaceInvalidUtf8);
    let envelope = permissive.to_envelope().unwrap();
    let format = envelope.target().clone();
    let preset = ImportPreset::new(
        ImportConfigOverlay::default()
            .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
            .with_loss_policy(LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery)),
    )
    .try_with_format_option(envelope)
    .unwrap();
    let request = ImportRequest::new().try_with_preset(&preset).unwrap();
    let prepared = importer
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    let permissive_result = prepared
        .read_bytes(
            baozi::AssetPath::new("named.obj").expect("valid source path"),
            INVALID_NAME,
            ImportOperationControl::default(),
        )
        .map_err(baozi::Error::from)
        .unwrap();
    assert_eq!(permissive_result.loss_evidence().len(), 1);
    assert!(
        prepared
            .overridden_formats()
            .any(|overridden| overridden == &format)
    );

    let caller = ImportRequestLayer::default()
        .try_with_format_option(ObjImportOptions::default().to_envelope().unwrap())
        .unwrap();
    let prepared = importer
        .plan(
            &request.with_caller_layer(caller),
            OptionCompileLimits::default(),
        )
        .unwrap();
    assert!(
        prepared
            .read_bytes(
                baozi::AssetPath::new("named.obj").expect("valid source path"),
                INVALID_NAME,
                ImportOperationControl::default(),
            )
            .map_err(baozi::Error::from)
            .is_err()
    );
}

#[test]
fn invalid_and_unknown_obj_option_envelopes_fail_before_asset_io() {
    let importer = Importer::new().unwrap();
    let defaults = ObjImportOptions::default().to_envelope().unwrap();
    let invalid = FormatOptionEnvelope::new(
        defaults.target().clone(),
        defaults.schema().clone(),
        defaults.version(),
        [0x80],
    )
    .unwrap();
    let unknown_schema = FormatOptionSchemaKey::new(
        defaults.target().clone(),
        FormatOptionSchemaId::new("unknown-obj-options").unwrap(),
    );
    let unknown = FormatOptionEnvelope::new(
        unknown_schema.owner().clone(),
        unknown_schema,
        NonZeroU32::MIN,
        b"",
    )
    .unwrap();
    for envelope in [invalid, unknown] {
        let layer = ImportRequestLayer::default()
            .try_with_format_option(envelope)
            .unwrap();
        let error = importer
            .plan(
                &ImportRequest::new().with_caller_layer(layer),
                OptionCompileLimits::default(),
            )
            .unwrap_err();
        assert!(matches!(error, baozi::Error::Request(_)));
    }
}
