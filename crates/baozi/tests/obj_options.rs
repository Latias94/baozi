#![cfg(feature = "format-obj")]

use baozi::{
    ConformancePolicy, FormatOptionEnvelope, FormatOptionKey, FormatOptionSchemaId,
    FormatOptionSchemaKey, ImportConfigOverlay, ImportLossKind, ImportOperationControl,
    ImportPreset, ImportRequest, ImportRequestLayer, Importer, LossPolicy, ObjImportOptions,
    ObjTextDecodingMode, OptionCompileLimits, obj_import_option_key,
};
use std::num::NonZeroU32;

const INVALID_NAME: &[u8] = b"o invalid-\xff\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

#[test]
fn facade_only_obj_options_apply_caller_precedence_over_a_preset() {
    let importer = Importer::new().unwrap();
    let key: FormatOptionKey<ObjImportOptions> = obj_import_option_key();
    let permissive = ObjImportOptions::default()
        .with_text_decoding_mode(ObjTextDecodingMode::ReplaceInvalidUtf8);
    let preset = ImportPreset::new(
        ImportConfigOverlay::default()
            .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
            .with_loss_policy(LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery)),
    )
    .try_with_format_option(permissive.to_envelope().unwrap())
    .unwrap();
    let request = ImportRequest::new().try_with_preset(&preset).unwrap();
    let prepared = importer
        .prepare_request(&request, OptionCompileLimits::default())
        .unwrap();
    let permissive_result = importer
        .read_bytes_with_request(
            "named.obj",
            INVALID_NAME,
            &prepared,
            ImportOperationControl::default(),
        )
        .unwrap();
    assert_eq!(permissive_result.loss_evidence().len(), 1);
    assert!(
        prepared
            .overridden_formats()
            .any(|format| format == key.schema().key().owner())
    );

    let caller = ImportRequestLayer::default()
        .try_with_format_option(ObjImportOptions::default().to_envelope().unwrap())
        .unwrap();
    let prepared = importer
        .prepare_request(
            &request.with_caller_layer(caller),
            OptionCompileLimits::default(),
        )
        .unwrap();
    assert!(
        importer
            .read_bytes_with_request(
                "named.obj",
                INVALID_NAME,
                &prepared,
                ImportOperationControl::default(),
            )
            .is_err()
    );
}

#[test]
fn invalid_and_unknown_obj_option_envelopes_fail_before_asset_io() {
    let importer = Importer::new().unwrap();
    let key = obj_import_option_key();
    let invalid = key.envelope_from_canonical_payload([0x80]).unwrap();
    let unknown_schema = FormatOptionSchemaKey::new(
        key.schema().key().owner().clone(),
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
            .prepare_request(
                &ImportRequest::new().with_caller_layer(layer),
                OptionCompileLimits::default(),
            )
            .unwrap_err();
        assert!(matches!(error, baozi::Error::Request(_)));
    }
}
