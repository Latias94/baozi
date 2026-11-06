mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, ImportFailure, ImportLedgerSnapshot,
    ImportTerminalCause, LedgerCause, MetadataValueRef, ScopeAbortCause,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, controlled_import, expected_error,
    glb_with_json, gltf_failure, import_assets, import_assets_result,
    import_assets_result_with_control,
};
use std::ops::Range;
use std::sync::atomic::Ordering;

fn encodings(json: &[u8]) -> [(&'static str, Vec<u8>, usize); 2] {
    [
        ("key-edges.gltf", json.to_vec(), 0),
        ("key-edges.glb", glb_with_json(json), 20),
    ]
}

fn assert_released_without_dependencies(ledger: &ImportLedgerSnapshot) {
    assert_eq!(ledger.current_live_bytes(), 0);
    assert!(ledger.current_administrative_bytes() <= ledger.peak_administrative_bytes());
    assert_eq!(ledger.active_reservations(), 0);
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(ledger.dimension(dimension).unwrap().attempted(), 0);
    }
    assert_eq!(
        ledger
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
}

fn assert_unpublished_failure(failure: &ImportFailure) {
    assert!(failure.diagnostics().is_empty());
    assert!(failure.source_scope_record().is_none());
    assert!(failure.scene_validation_error().is_none());
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert!(matches!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(_)
    ));
    assert_released_without_dependencies(failure.ledger());
}

fn assert_raw_range(failure: &ImportFailure, subject: &str, range: Range<usize>) -> TestResult<()> {
    assert_gltf_failure(failure, GltfFailureCode::InvalidDocument, None)?;
    let details = gltf_failure(failure)?.details();
    assert!(details.iter().any(|detail| {
        detail.key() == "subject"
            && matches!(detail.value(), FormatFailureDetailValue::String(value) if value.as_str() == subject)
    }));
    for (key, expected) in [
        ("byte_offset", range.start as u64),
        ("byte_length", range.len() as u64),
    ] {
        assert!(details.iter().any(|detail| {
            detail.key() == key
                && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value) if value == expected)
        }), "missing {key}={expected}: {details:?}");
    }
    assert_unpublished_failure(failure);
    Ok(())
}

#[test]
fn raw_json_transitions_reject_at_the_original_byte_range_in_json_and_glb() -> TestResult<()> {
    struct SyntaxCase {
        vendor: &'static [u8],
        closed: bool,
        subject: &'static str,
        range: Range<usize>,
        reason: &'static str,
    }

    let cases = [
        SyntaxCase {
            vendor: br#"[0,]"#,
            closed: true,
            subject: "json-document",
            range: 3..4,
            reason: "unexpected byte",
        },
        SyntaxCase {
            vendor: br#"{"k" 0}"#,
            closed: true,
            subject: "json-object",
            range: 5..6,
            reason: "colon",
        },
        SyntaxCase {
            vendor: b"-",
            closed: true,
            subject: "json-number",
            range: 1..2,
            reason: "integer digit",
        },
        SyntaxCase {
            vendor: b"1.",
            closed: true,
            subject: "json-number",
            range: 2..3,
            reason: "fraction",
        },
        SyntaxCase {
            vendor: b"1e",
            closed: true,
            subject: "json-number",
            range: 2..3,
            reason: "exponent",
        },
        SyntaxCase {
            vendor: b"1e+",
            closed: true,
            subject: "json-number",
            range: 3..4,
            reason: "exponent",
        },
        SyntaxCase {
            vendor: br#""\uDC00""#,
            closed: true,
            subject: "json-string",
            range: 1..7,
            reason: "low surrogate",
        },
        SyntaxCase {
            vendor: br#""\uD834\u0061""#,
            closed: true,
            subject: "json-string",
            range: 7..13,
            reason: "low surrogate",
        },
        SyntaxCase {
            vendor: br#""tail\"#,
            closed: false,
            subject: "json-string",
            range: 5..6,
            reason: "inside an escape",
        },
        SyntaxCase {
            vendor: br#"[0"#,
            closed: false,
            subject: "json-array",
            range: 2..2,
            reason: "not closed",
        },
        SyntaxCase {
            vendor: br#"{"k":0"#,
            closed: false,
            subject: "json-object",
            range: 6..6,
            reason: "not closed",
        },
        SyntaxCase {
            vendor: b"1e+",
            closed: false,
            subject: "json-number",
            range: 3..3,
            reason: "exponent",
        },
    ];
    const PREFIX: &[u8] = br#"{"asset":{"version":"2.0"},"vendor":"#;
    for case in cases {
        let mut json = PREFIX.to_vec();
        if !case.closed {
            // Put alignment before the value so the GLB wrapper preserves the actual EOF.
            while !(json.len() + case.vendor.len()).is_multiple_of(4) {
                json.push(b' ');
            }
        }
        let value_start = json.len();
        json.extend_from_slice(case.vendor);
        if case.closed {
            json.push(b'}');
        }
        for (source, bytes, base) in encodings(&json) {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidDocument,
                Some(case.reason),
            )?;
            let start = base + value_start;
            assert_raw_range(
                &failure,
                case.subject,
                start + case.range.start..start + case.range.end,
            )?;
        }
    }
    Ok(())
}

#[test]
fn literal_and_surrogate_pair_keys_have_the_same_supplementary_identity() -> TestResult<()> {
    let literal_key = format!("\"{}\"", '\u{1d11e}');
    let escaped_key = r#""\uD834\uDD1E""#;
    for (first, duplicate) in [
        (literal_key.as_str(), escaped_key),
        (escaped_key, literal_key.as_str()),
    ] {
        let json =
            format!(r#"{{"asset":{{"version":"2.0"}},"vendor":{{{first}:1,{duplicate}:2}}}}"#)
                .into_bytes();
        let offset = json
            .windows(duplicate.len())
            .rposition(|bytes| bytes == duplicate.as_bytes())
            .expect("later duplicate key exists");
        for (source, bytes, base) in encodings(&json) {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidDocument,
                Some("duplicate key"),
            )?;
            assert_raw_range(
                &failure,
                "json-object-key",
                base + offset..base + offset + duplicate.len(),
            )?;
        }
    }
    Ok(())
}

#[test]
fn empty_keys_supplementary_scalars_and_distinct_normalization_forms_are_preserved()
-> TestResult<()> {
    let extras_with_literal = format!(r#"{{"":0,"{}":1,"\u00e9":2,"e\u0301":3}}"#, '\u{1d11e}');
    for extras in [
        extras_with_literal.as_str(),
        r#"{"":0,"\uD834\uDD1E":1,"\u00e9":2,"e\u0301":3}"#,
    ] {
        let json = format!(r#"{{
            "asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1,2,3]}}],
            "nodes":[{{"name":"{}"}},{{"name":"\uD834\uDD1E"}},{{"name":"\u00e9"}},{{"name":"e\u0301"}}],
            "extras":{extras}
        }}"#, '\u{1d11e}').into_bytes();
        for (source, bytes, _) in encodings(&json) {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let names = report
                .scene()
                .nodes()
                .map(|node| node.name())
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                [
                    Some("\u{1d11e}"),
                    Some("\u{1d11e}"),
                    Some("\u{e9}"),
                    Some("e\u{301}")
                ]
            );
            let entry = report
                .scene()
                .metadata_entries()
                .find(|entry| entry.key() == "org.baozi.gltf:extras")
                .expect("raw document extras");
            let MetadataValueRef::Bytes {
                media_type,
                payload,
            } = entry.value()
            else {
                panic!("extras must remain raw JSON bytes");
            };
            assert_eq!(media_type, "application/json");
            assert_eq!(payload, extras.as_bytes());
            let unique: serde_json::Value = serde_json::from_slice(payload)?;
            assert_eq!(unique.as_object().unwrap().len(), 4);
            assert_eq!(unique[""], 0);
            assert_eq!(unique["\u{1d11e}"], 1);
            assert_eq!(unique["\u{e9}"], 2);
            assert_eq!(unique["e\u{301}"], 3);
            assert!(report.loss_evidence().is_empty());
            assert_released_without_dependencies(report.ledger());
        }
    }
    Ok(())
}

fn wide_document(width: usize) -> Vec<u8> {
    assert!(matches!(width, 32 | 128));
    let mut json = String::from(r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{}],"vendor":{"#);
    for index in 0..width {
        if index != 0 {
            json.push(',');
        }
        json.push_str(r#""\u006b"#);
        for digit in format!("{index:04}").bytes() {
            json.push_str(&format!(r"\u{digit:04x}"));
        }
        json.push_str(r#"":0"#);
    }
    json.push_str(
        r#"},"nodes":[{"name":"\u0061\uD834\uDD1E"}],"extras":{"label":"\u0061\uD834\uDD1E"}}"#,
    );
    json.into_bytes()
}

fn cancelled_wide_scan(document: &[u8], cancel_on: usize) -> TestResult<ImportFailure> {
    let (control, polls) = controlled_import(Some(cancel_on));
    let (result, diagnostics) = import_assets_result_with_control(
        "wide-keys.gltf",
        [("wide-keys.gltf", document.to_vec())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
    if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
        origin: CancellationOrigin::Signal
    }))));
    assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
    assert_released_without_dependencies(failure.ledger());
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    Ok(failure)
}

#[test]
fn wide_escaped_keys_have_independent_counts_and_cancel_inside_the_scan() -> TestResult<()> {
    let mut previous_work = 0;
    let mut previous_storage = 0;
    let mut total_polls = 0;
    for width in [32, 128] {
        let document = wide_document(width);
        let (control, polls) = controlled_import(None);
        let (result, diagnostics) = import_assets_result_with_control(
            "wide-keys.gltf",
            [("wide-keys.gltf", document.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        assert!(diagnostics.is_empty());
        let report = result?;
        assert_eq!(report.scene().node_count(), 0);
        assert_eq!(report.scene().mesh_count(), 0);
        let extras = report
            .scene()
            .metadata_entries()
            .find(|entry| entry.key() == "org.baozi.gltf:extras")
            .expect("constant selected metadata");
        let MetadataValueRef::Bytes { payload, .. } = extras.value() else {
            panic!("raw extras bytes");
        };
        assert_eq!(payload, br#"{"label":"\u0061\uD834\uDD1E"}"#);
        let value: serde_json::Value = serde_json::from_slice(payload)?;
        assert_eq!(value["label"].as_str(), Some("a\u{1d11e}"));
        assert_eq!(value["label"].as_str().unwrap().len(), 5);
        let ledger = report.ledger();
        assert_eq!(
            ledger
                .dimension(ImportDimension::DecodeWork)
                .unwrap()
                .consumed(),
            document.len() as u64
        );
        // Each sibling contributes a key, colon, number and comma; fixed syntax contributes 42.
        assert_eq!(
            ledger
                .dimension(ImportDimension::Tokens)
                .unwrap()
                .consumed(),
            4 * width as u64 + 42
        );
        assert_eq!(
            ledger
                .dimension(ImportDimension::Records)
                .unwrap()
                .consumed(),
            8
        );
        let work = ledger
            .dimension(ImportDimension::WorkUnits)
            .unwrap()
            .consumed();
        let storage = ledger
            .dimension(ImportDimension::ScratchBytes)
            .unwrap()
            .peak_live_bytes();
        assert!(work > previous_work);
        assert!(storage > previous_storage);
        previous_work = work;
        previous_storage = storage;
        total_polls = polls.load(Ordering::SeqCst);
        assert_released_without_dependencies(ledger);
    }

    let document = wide_document(128);
    let vendor_end = document
        .windows(br#"},"nodes"#.len())
        .position(|bytes| bytes == br#"},"nodes"#)
        .expect("the wide unknown object precedes typed nodes");
    let mut options = GltfTestOptions::memory();
    // Primary byte admission remains ample; the independently counted syntax cannot fit.
    options.limits.max_tokens = 4 * 128 + 41;
    let (result, diagnostics) = import_assets_result(
        "wide-keys.gltf",
        [("wide-keys.gltf", document.clone())],
        options,
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
    if matches!(abort.cause(), ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
        dimension: ImportDimension::Tokens, limit: 553, ..
    }))));
    assert_unpublished_failure(&failure);

    assert!(document.len() > 4096);
    assert!(total_polls > 1);
    let mut low = 1;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let failure = cancelled_wide_scan(&document, middle)?;
        if failure
            .ledger()
            .dimension(ImportDimension::DecodeWork)
            .unwrap()
            .consumed()
            >= 4096
        {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let failure = cancelled_wide_scan(&document, low)?;
    let consumed = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap()
        .consumed();
    assert!(consumed >= 4096 && consumed < vendor_end as u64);
    assert_unpublished_failure(&failure);
    Ok(())
}
