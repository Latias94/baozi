mod common;

use baozi_format_obj::{
    FORMAT_ID, ObjFailureCode, ObjImportOptions, ObjImporter, ObjTextDecodingMode,
    format_descriptor, obj_import_option_key,
};
use baozi_import::authoring::FormatId;
use baozi_import::{
    ConformancePolicy, FormatSelection, ImportCause, ImportConfigOverlay, ImportDimension,
    ImportLossDisposition, ImportLossKind, ImportOperationControl, ImportRequest,
    ImportRequestLayer, LossPolicy, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder,
    ResourceLimits, ScopeAbortCause, TextDecodingMode, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use common::{
    TestResult, expected_failure, expected_success, import_assets_result_with_text_policy,
    import_bytes_result, import_bytes_result_with_text_policy, operation_result, scene_snapshot,
    snapshot_line,
};
use std::sync::Arc;

const GEOMETRY: &[u8] = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

fn replacement_options() -> ObjImportOptions {
    ObjImportOptions::default().with_text_decoding_mode(ObjTextDecodingMode::ReplaceInvalidUtf8)
}

fn recovered_obj(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"o ");
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(suffix);
    bytes.extend_from_slice(b"\n");
    bytes.extend_from_slice(GEOMETRY);
    bytes
}

fn import_recovered_obj(bytes: &[u8]) -> TestResult<baozi_import::ImportReport> {
    import_bytes_result_with_text_policy(
        "recovered.obj",
        bytes,
        ResourceLimits::default(),
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )
    .map(expected_success)
}

fn ranges(report: &baozi_import::ImportReport) -> Vec<(u64, u64)> {
    report
        .loss_evidence()
        .iter()
        .map(|evidence| {
            let range = evidence
                .raw_range()
                .expect("text recovery evidence retains a raw byte range");
            (range.start(), range.end())
        })
        .collect()
}

fn assert_invalid_utf8(failure: &baozi_import::ImportFailure, offset: u64) {
    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected format failure, got {:?}", failure.cause());
    };
    assert_eq!(envelope.code(), ObjFailureCode::InvalidUtf8.as_str());
    assert!(envelope.details().iter().any(|detail| {
        detail.key() == "byte_offset"
            && matches!(
                detail.value(),
                baozi_import::FormatFailureDetailValue::Unsigned(value) if value == offset
            )
    }));
}

#[test]
fn descriptor_advertises_strict_and_declared_utf8_recovery() {
    let descriptor = format_descriptor();
    let profile = descriptor.text_profile().expect("OBJ is a text format");

    assert!(profile.allows_decoding_mode(TextDecodingMode::RejectInvalid));
    assert!(profile.allows_decoding_mode(TextDecodingMode::ReplaceInvalidUtf8));
}

#[test]
fn decoder_uses_only_frozen_effective_limit_authority() {
    let parser_source = include_str!("../src/parser.rs");

    assert!(!parser_source.contains("obj_text_profile"));
    assert!(!parser_source.contains("text_profile"));
    assert!(!parser_source.contains("format_descriptor"));
    assert!(!parser_source.contains(".descriptor()"));
    assert!(!parser_source.contains("hard_caps"));
    assert!(!parser_source.contains("obj-text-profile"));
    assert!(!parser_source.contains("max_raw_bytes"));
    assert!(!parser_source.contains("max_primary_asset_bytes"));
    assert!(!parser_source.contains("max_dependency_asset_bytes"));
    assert!(!parser_source.contains("token_count"));
    assert!(!parser_source.contains("charge_token"));
}

#[test]
fn option_schema_and_canonical_payload_are_stable() -> TestResult<()> {
    let key = obj_import_option_key();
    assert_eq!(key.schema().key().owner().as_str(), FORMAT_ID);
    assert_eq!(key.schema().key().local().as_str(), "import-options");
    assert_eq!(key.schema().version().get(), 1);

    let mut registry = RegistryBuilder::new();
    registry.register(ObjImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    for payload in [
        [0_u8].as_slice(),
        [1_u8, 0].as_slice(),
        [1_u8, 1].as_slice(),
    ] {
        let envelope = key.envelope_from_canonical_payload(payload)?;
        let config = ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?));
        let caller = ImportRequestLayer::new(config).try_with_format_option(envelope)?;
        let request = ImportRequest::new().with_caller_layer(caller);
        snapshot.compile_request(&request, OptionCompileLimits::default())?;
    }
    Ok(())
}

#[test]
fn option_codec_rejects_noncanonical_payloads() -> TestResult<()> {
    let mut registry = RegistryBuilder::new();
    registry.register(ObjImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let key = obj_import_option_key();

    for payload in [
        Vec::new(),
        vec![0b0000_0010],
        vec![0b0000_0001],
        vec![0b0000_0001, 2],
        vec![0, 0],
        vec![1, 0, 0],
    ] {
        let envelope = key.envelope_from_canonical_payload(payload)?;
        let config = ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?));
        let caller = ImportRequestLayer::new(config).try_with_format_option(envelope)?;
        let request = ImportRequest::new().with_caller_layer(caller);
        assert!(
            snapshot
                .compile_request(&request, OptionCompileLimits::default())
                .is_err(),
            "payload must be rejected"
        );
    }
    Ok(())
}

#[test]
fn option_layers_preserve_absent_fields_and_caller_wins() -> TestResult<()> {
    fn import_layers(
        bytes: &[u8],
        preset_payload: &[u8],
        caller_payload: &[u8],
    ) -> TestResult<Result<baozi_import::ImportReport, baozi_import::ImportFailure>> {
        let source = AssetPath::new("layered.obj")?;
        let io = MemoryAssetIo::new();
        io.try_insert(source.clone(), Arc::<[u8]>::from(bytes))?;
        let mut registry = RegistryBuilder::new();
        registry.register(ObjImporter)?;
        let snapshot = registry.freeze(RegistryBuildLimits::default())?;
        let key = obj_import_option_key();
        let preset = ImportRequestLayer::new(ImportConfigOverlay::default())
            .try_with_format_option(key.envelope_from_canonical_payload(preset_payload)?)?;
        let config = ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?))
            .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
            .with_loss_policy(LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery));
        let caller = ImportRequestLayer::new(config)
            .try_with_format_option(key.envelope_from_canonical_payload(caller_payload)?)?;
        let request = ImportRequest::from_layers(preset, caller);
        let request = snapshot.compile_request(&request, OptionCompileLimits::default())?;
        operation_result(import_asset(
            &request,
            &io,
            source,
            ImportOperationControl::default(),
        ))
    }

    let bytes = recovered_obj(&[0xff], b"");
    let preserved = expected_success(import_layers(&bytes, &[1, 1], &[0])?);
    assert_eq!(preserved.loss_evidence().len(), 1);

    let overridden = expected_failure(import_layers(&bytes, &[1, 1], &[1, 0])?);
    assert_invalid_utf8(&overridden, 2);
    assert!(overridden.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn strict_default_rejects_invalid_utf8_without_loss_evidence() -> TestResult<()> {
    let bytes = recovered_obj(&[0xff], b"");
    let failure = expected_failure(import_bytes_result(
        "strict.obj",
        &bytes,
        ResourceLimits::default(),
    )?);

    assert_invalid_utf8(&failure, 2);
    assert!(failure.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn selected_recovery_is_still_rejected_by_strict_conformance() -> TestResult<()> {
    let bytes = recovered_obj(&[0xff], b"");
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "strict-conformance.obj",
        &bytes,
        ResourceLimits::default(),
        Some(replacement_options()),
        ConformancePolicy::Strict,
        true,
    )?);

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::FormatRecoveryRejectedByConformance { .. }
            )
    ));
    assert_eq!(failure.loss_evidence().len(), 1);
    assert_eq!(
        failure.loss_evidence()[0].disposition(),
        ImportLossDisposition::RejectedByConformance
    );
    Ok(())
}

#[test]
fn selected_recovery_is_still_rejected_by_loss_policy() -> TestResult<()> {
    let bytes = recovered_obj(&[0xff], b"");
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "strict-loss.obj",
        &bytes,
        ResourceLimits::default(),
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        false,
    )?);

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { .. }
            )
    ));
    assert_eq!(failure.loss_evidence().len(), 1);
    assert_eq!(
        failure.loss_evidence()[0].disposition(),
        ImportLossDisposition::RejectedByLossPolicy
    );
    Ok(())
}

#[test]
fn all_recovery_gates_apply_ordered_primary_evidence() -> TestResult<()> {
    let bytes = recovered_obj(&[0xff], b"");
    let report = import_recovered_obj(&bytes)?;

    assert_eq!(ranges(&report), vec![(2, 3)]);
    assert_eq!(
        report.loss_evidence()[0].kind(),
        ImportLossKind::FormatRecovery
    );
    assert_eq!(
        report.loss_evidence()[0].disposition(),
        ImportLossDisposition::Applied
    );
    assert_eq!(report.loss_evidence()[0].source().as_str(), "recovered.obj");
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::DecodedTextBytes)
            .expect("decoded text dimension is configured")
            .peak_live_bytes()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::LineIndexBytes)
            .expect("line index dimension is configured")
            .peak_live_bytes()
            > 0
    );
    Ok(())
}

#[test]
fn decoder_uses_unicode_maximal_subparts() -> TestResult<()> {
    type ReplacementVector<'a> = (&'a [u8], &'a [(u64, u64)]);

    let vectors: &[ReplacementVector<'_>] = &[
        (&[0x80, 0x80], &[(2, 3), (3, 4)]),
        (&[0xc0, 0xaf], &[(2, 3), (3, 4)]),
        (&[0xe0, 0x80, 0x80], &[(2, 3), (3, 4), (4, 5)]),
        (&[0xed, 0xa0, 0x80], &[(2, 3), (3, 4), (4, 5)]),
        (&[0xf4, 0x90, 0x80, 0x80], &[(2, 3), (3, 4), (4, 5), (5, 6)]),
        (&[0xe2, 0x82, b'A'], &[(2, 4)]),
    ];

    for (bytes, expected) in vectors {
        let input = recovered_obj(bytes, b"");
        let report = import_recovered_obj(&input)?;
        assert_eq!(ranges(&report), *expected, "vector {bytes:02x?}");
    }
    Ok(())
}

#[test]
fn decoder_groups_truncated_eof_prefixes() -> TestResult<()> {
    for (bytes, expected) in [
        ([0xe2, 0x82].as_slice(), (2, 4)),
        ([0xf0, 0x90, 0x80].as_slice(), (2, 5)),
    ] {
        let mut input = Vec::from(b"o ".as_slice());
        input.extend_from_slice(bytes);
        let failure = expected_failure(import_bytes_result_with_text_policy(
            "truncated.obj",
            &input,
            ResourceLimits::default(),
            Some(replacement_options()),
            ConformancePolicy::AllowDeclaredRecovery,
            true,
        )?);
        assert_eq!(
            failure
                .loss_evidence()
                .iter()
                .map(|evidence| {
                    let range = evidence
                        .raw_range()
                        .expect("text recovery evidence retains a raw byte range");
                    (range.start(), range.end())
                })
                .collect::<Vec<_>>(),
            vec![expected]
        );
    }
    Ok(())
}

#[test]
fn decoder_carries_utf8_state_across_four_kib_boundaries() -> TestResult<()> {
    let mut valid = Vec::from(b"o ".as_slice());
    valid.extend(std::iter::repeat_n(b'a', 4_093));
    valid.extend_from_slice("\u{1f642}".as_bytes());
    valid.push(b'\n');
    valid.extend_from_slice(GEOMETRY);
    let report = import_recovered_obj(&valid)?;
    assert!(report.loss_evidence().is_empty());
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=absent "));
    assert!(
        snapshot
            .preview()
            .lines()
            .any(|line| line.contains("key=obj:object") && line.len() >= 4_000),
        "the long object name must remain available as canonical metadata"
    );

    let mut truncated = Vec::from(b"o ".as_slice());
    truncated.extend(std::iter::repeat_n(b'a', 4_093));
    truncated.extend_from_slice(&[0xe2, 0x82, b'A']);
    truncated.push(b'\n');
    truncated.extend_from_slice(GEOMETRY);
    let report = import_recovered_obj(&truncated)?;
    assert_eq!(ranges(&report), vec![(4_095, 4_097)]);

    let mut truncated_eof = Vec::from(b"o ".as_slice());
    truncated_eof.extend(std::iter::repeat_n(b'a', 4_093));
    truncated_eof.extend_from_slice(&[0xe2, 0x82]);
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "cross-boundary-truncated.obj",
        &truncated_eof,
        ResourceLimits::default(),
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert_eq!(
        failure
            .loss_evidence()
            .iter()
            .map(|evidence| {
                let range = evidence
                    .raw_range()
                    .expect("text recovery evidence retains a raw byte range");
                (range.start(), range.end())
            })
            .collect::<Vec<_>>(),
        vec![(4_095, 4_097)]
    );
    Ok(())
}

#[test]
fn comments_drop_invalid_bytes_without_false_loss() -> TestResult<()> {
    let mut bytes = Vec::from(b"# ignored ".as_slice());
    bytes.extend_from_slice(&[0xff, 0x80]);
    bytes.push(b'\n');
    bytes.extend_from_slice(GEOMETRY);

    let report = import_recovered_obj(&bytes)?;
    assert!(report.loss_evidence().is_empty());
    assert!(report.diagnostics().is_empty());
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::RecoveryCount)
            .expect("recovery count dimension is configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn inline_obj_comments_discard_invalid_bytes_after_semantic_recovery() -> TestResult<()> {
    let mut bytes = Vec::from(b"o recovered".as_slice());
    bytes.push(0xff);
    bytes.extend_from_slice(b" # ignored ");
    bytes.push(0x80);
    bytes.push(b'\n');
    bytes.extend_from_slice(GEOMETRY);
    let semantic_offset = bytes
        .iter()
        .position(|byte| *byte == 0xff)
        .expect("fixture contains a semantic replacement") as u64;
    let comment_offset = bytes
        .iter()
        .position(|byte| *byte == 0x80)
        .expect("fixture contains a comment replacement") as u64;

    let report = import_recovered_obj(&bytes)?;

    assert_eq!(
        ranges(&report),
        vec![(semantic_offset, semantic_offset + 1)]
    );
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.invalid_utf8_replaced"
    );
    assert_eq!(report.diagnostics()[0].source().as_str(), "recovered.obj");
    let diagnostic_range = report.diagnostics()[0]
        .byte_range()
        .expect("semantic recovery diagnostic retains a byte range");
    assert_eq!(
        (diagnostic_range.start(), diagnostic_range.end()),
        (semantic_offset, semantic_offset + 1)
    );
    assert!(!ranges(&report).contains(&(comment_offset, comment_offset + 1)));
    Ok(())
}

#[test]
fn mtl_comments_drop_invalid_bytes_without_false_loss() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"# ignored \xff\x80\nnewmtl red\nKd 1 0 0\n";
    let report = expected_success(import_assets_result_with_text_policy(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
        true,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);

    assert!(report.loss_evidence().is_empty());
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn inline_mtl_comments_discard_invalid_bytes_after_semantic_recovery() -> TestResult<()> {
    let obj = "mtllib material.mtl\nusemtl r\u{fffd}d\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mut mtl = Vec::from(b"newmtl r".as_slice());
    mtl.push(0xff);
    mtl.extend_from_slice(b"d # ignored ");
    mtl.push(0x80);
    mtl.extend_from_slice(b"\nKd 1 0 0\n");
    let semantic_offset = mtl
        .iter()
        .position(|byte| *byte == 0xff)
        .expect("fixture contains a semantic replacement") as u64;
    let comment_offset = mtl
        .iter()
        .position(|byte| *byte == 0x80)
        .expect("fixture contains a comment replacement") as u64;

    let report = expected_success(import_assets_result_with_text_policy(
        "model.obj",
        [
            ("model.obj", obj.as_bytes()),
            ("material.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
        true,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);

    assert_eq!(
        ranges(&report),
        vec![(semantic_offset, semantic_offset + 1)]
    );
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.invalid_utf8_replaced"
    );
    assert_eq!(report.diagnostics()[0].source().as_str(), "material.mtl");
    let diagnostic_range = report.diagnostics()[0]
        .byte_range()
        .expect("semantic recovery diagnostic retains a byte range");
    assert_eq!(
        (diagnostic_range.start(), diagnostic_range.end()),
        (semantic_offset, semantic_offset + 1)
    );
    assert!(!ranges(&report).contains(&(comment_offset, comment_offset + 1)));
    Ok(())
}

#[test]
fn recovery_remains_strict_in_obj_keyword_numeric_smoothing_and_face_syntax() -> TestResult<()> {
    let fixtures: &[(&str, &[u8], u64)] = &[
        ("keyword.obj", b"v\xff 0 0 0\n", 1),
        ("numeric.obj", b"v \xff 0 0\n", 2),
        ("smoothing.obj", b"s \xff\n", 2),
        ("face.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 \xff\n", 30),
    ];

    for (source, prefix, offset) in fixtures {
        let mut bytes = Vec::from(*prefix);
        if *source != "face.obj" {
            bytes.extend_from_slice(GEOMETRY);
        }
        let failure = expected_failure(import_bytes_result_with_text_policy(
            source,
            &bytes,
            ResourceLimits::default(),
            Some(replacement_options()),
            ConformancePolicy::AllowDeclaredRecovery,
            true,
        )?);
        assert_invalid_utf8(&failure, *offset);
        assert!(failure.diagnostics().is_empty());
        assert!(failure.loss_evidence().is_empty());
        for dimension in [
            ImportDimension::RecoveryCount,
            ImportDimension::RecoveryRawBytes,
        ] {
            let recovery = failure
                .ledger()
                .dimension(dimension)
                .expect("recovery evidence remains queryable");
            assert_eq!(recovery.attempted(), 0);
            assert_eq!(recovery.consumed(), 0);
        }
    }
    Ok(())
}

#[test]
fn strict_mtl_invalid_utf8_is_terminal_instead_of_skipped() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl r\xffd\nKd 1 0 0\n";
    let failure = expected_failure(import_assets_result_with_text_policy(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
        true,
        None,
        ConformancePolicy::Strict,
        false,
    )?);

    assert_invalid_utf8(&failure, 8);
    assert!(failure.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn mtl_recovers_only_names_and_map_paths() -> TestResult<()> {
    let obj = "mtllib material.mtl\nusemtl r\u{fffd}d\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl r\xffd\nKd 1 0 0\nmap_Kd textures/\xff.png\n";
    let report = expected_success(import_assets_result_with_text_policy(
        "model.obj",
        [
            ("model.obj", obj.as_bytes()),
            ("material.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
        true,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);

    assert_eq!(report.loss_evidence().len(), 2);
    assert!(
        report
            .loss_evidence()
            .iter()
            .all(|evidence| evidence.source().as_str() == "material.mtl")
    );
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "material[0]").contains(" name=r\u{fffd}d"));
    Ok(())
}

#[test]
fn obj_mtllib_and_mtl_map_paths_recover_at_their_textual_boundaries() -> TestResult<()> {
    let mut obj = Vec::from(b"mtllib mat".as_slice());
    obj.push(0xff);
    obj.extend_from_slice(b".mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    let mut mtl = Vec::from(b"newmtl red\nmap_Kd -clamp on textures/".as_slice());
    mtl.push(0xff);
    mtl.extend_from_slice(b".png\n");
    let recovered_mtl_path = "mat\u{fffd}.mtl";
    let report = expected_success(import_assets_result_with_text_policy(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            (recovered_mtl_path, mtl.as_slice()),
        ],
        ResourceLimits::default(),
        true,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);

    assert_eq!(report.loss_evidence().len(), 2);
    assert_eq!(report.loss_evidence()[0].source().as_str(), "model.obj");
    assert_eq!(
        report.loss_evidence()[1].source().as_str(),
        recovered_mtl_path
    );
    Ok(())
}

#[test]
fn recovery_remains_strict_in_mtl_keyword_and_numeric_syntax() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    for mtl in [
        b"new\xffmtl red\n".as_slice(),
        b"newmtl red\nKd 1 \xff 0\n".as_slice(),
        b"newmtl red\nmap_Kd -s \xff 1 texture.png\n".as_slice(),
    ] {
        let offset = mtl
            .iter()
            .position(|byte| *byte == 0xff)
            .expect("fixture contains invalid UTF-8") as u64;
        let failure = expected_failure(import_assets_result_with_text_policy(
            "model.obj",
            [("model.obj", obj.as_slice()), ("material.mtl", mtl)],
            ResourceLimits::default(),
            true,
            Some(replacement_options()),
            ConformancePolicy::AllowDeclaredRecovery,
            true,
        )?);
        assert_invalid_utf8(&failure, offset);
        assert!(failure.loss_evidence().is_empty());
    }
    Ok(())
}

#[test]
fn text_profile_rejects_undeclared_controls_and_lone_cr() -> TestResult<()> {
    for bytes in [
        b"o bad\0name\nv 0 0 0\n".as_slice(),
        b"o name\rv 0 0 0\n".as_slice(),
    ] {
        let invalid_offset = bytes
            .iter()
            .position(|byte| *byte == 0 || *byte == b'\r')
            .expect("fixture contains a forbidden text byte") as u64;
        let failure = expected_failure(import_bytes_result(
            "text-profile.obj",
            bytes,
            ResourceLimits::default(),
        )?);
        let ImportCause::Format(envelope) = failure.cause() else {
            panic!("expected format failure, got {:?}", failure.cause());
        };
        assert_eq!(envelope.code(), ObjFailureCode::InvalidStatement.as_str());
        assert!(envelope.details().iter().any(|detail| {
            detail.key() == "byte_offset"
                && matches!(
                    detail.value(),
                    baozi_import::FormatFailureDetailValue::Unsigned(value)
                        if value == invalid_offset
                )
        }));
    }
    Ok(())
}

#[test]
fn effective_text_limits_gate_decoder_indexes_tokens_and_canonical_values() -> TestResult<()> {
    let bytes = recovered_obj(&[0xff], b"");

    let mut decoded_limit = ResourceLimits::default();
    decoded_limit.max_decoded_utf8_bytes = 1;
    let decoded_failure = expected_failure(import_bytes_result_with_text_policy(
        "decoded-limit.obj",
        &bytes,
        decoded_limit,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert!(matches!(
        decoded_failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "obj-decoded-utf8-bytes",
                    limit: 1,
                }
            )
    ));

    let mut index_limit = ResourceLimits::default();
    index_limit.max_line_index_bytes = 1;
    let index_failure = expected_failure(import_bytes_result_with_text_policy(
        "index-limit.obj",
        &bytes,
        index_limit,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert!(matches!(
        index_failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::ResourceLimit {
                    resource: "obj-line-index-bytes",
                    limit: 1,
                }
            )
    ));

    let mut token_limit = ResourceLimits::default();
    token_limit.max_tokens = 3;
    let token_failure = expected_failure(import_bytes_result(
        "token-effective-limit.obj",
        GEOMETRY,
        token_limit,
    )?);
    assert!(matches!(
        token_failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(baozi_operation::LedgerCause::LimitExceeded {
                    dimension: ImportDimension::Tokens,
                    ..
                })
            )
    ));

    let mut canonical_limit = ResourceLimits::default();
    canonical_limit.max_canonical_string_bytes = 2;
    let canonical_failure = expected_failure(import_bytes_result(
        "canonical-effective-limit.obj",
        &recovered_obj(b"abc", b""),
        canonical_limit,
    )?);
    assert!(matches!(
        canonical_failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(baozi_operation::LedgerCause::LimitExceeded {
                    dimension: ImportDimension::CanonicalStringBytes,
                    ..
                })
            )
    ));

    let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl material-name-exceeds-budget\n";
    let mut mtl_canonical_limit = ResourceLimits::default();
    mtl_canonical_limit.max_canonical_string_bytes = 8;
    let mtl_failure = expected_failure(import_assets_result_with_text_policy(
        "model.obj",
        [("model.obj", obj.as_slice()), ("m.mtl", mtl.as_slice())],
        mtl_canonical_limit,
        false,
        None,
        ConformancePolicy::Strict,
        false,
    )?);
    assert!(matches!(
        mtl_failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::Ledger(baozi_operation::LedgerCause::LimitExceeded {
                    dimension: ImportDimension::CanonicalStringBytes,
                    ..
                })
            )
    ));
    Ok(())
}

#[test]
fn recovery_and_loss_evidence_obey_each_ledger_limit() -> TestResult<()> {
    fn assert_ledger_failure(failure: &baozi_import::ImportFailure, dimension: ImportDimension) {
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Ledger(baozi_operation::LedgerCause::LimitExceeded {
                        dimension: actual,
                        ..
                    }) if *actual == dimension
                )
        ));
    }

    let two_recoveries = recovered_obj(&[0x80, 0x80], b"");
    let mut recovery_count = ResourceLimits::default();
    recovery_count.max_attempted_recoveries = 1;
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "recovery-count.obj",
        &two_recoveries,
        recovery_count,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert_ledger_failure(&failure, ImportDimension::RecoveryCount);

    let mut evidence_count = ResourceLimits::default();
    evidence_count.max_loss_evidence = 1;
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "evidence-count.obj",
        &two_recoveries,
        evidence_count,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert_ledger_failure(&failure, ImportDimension::LossEvidenceCount);

    let two_raw_bytes = recovered_obj(&[0xe2, 0x82, b'A'], b"");
    let mut recovery_bytes = ResourceLimits::default();
    recovery_bytes.max_attempted_recovery_bytes = 1;
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "recovery-bytes.obj",
        &two_raw_bytes,
        recovery_bytes,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert_ledger_failure(&failure, ImportDimension::RecoveryRawBytes);

    let one_recovery = recovered_obj(&[0xff], b"");
    let mut evidence_bytes = ResourceLimits::default();
    evidence_bytes.max_loss_evidence_bytes = 1;
    let failure = expected_failure(import_bytes_result_with_text_policy(
        "evidence-bytes.obj",
        &one_recovery,
        evidence_bytes,
        Some(replacement_options()),
        ConformancePolicy::AllowDeclaredRecovery,
        true,
    )?);
    assert_ledger_failure(&failure, ImportDimension::LossEvidenceBytes);
    Ok(())
}
