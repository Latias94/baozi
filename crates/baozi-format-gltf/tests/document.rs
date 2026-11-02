mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{ImportCause, ImportDimension, MetadataValueRef, ScopeAbortCause};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, assert_resource_limit, controlled_import,
    expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control,
};
use std::sync::atomic::Ordering;

#[test]
fn indexed_unknown_values_keep_exact_extras_and_escaped_names_keep_presence() -> TestResult<()> {
    let extras = br#"{ "unknown\u002dkey" : [[null,{},[],"\uD834\uDD1E"]], "number":-0.00e+0 }"#;
    let mut document = br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1,2]}],"nodes":[{}, {"name":""}, {"n\u0061me":"\u0061\uD834\uDD1E"}],"vendor":{"ignored":[[[{"escaped\u002dkey":"\u0000"}]]]},"extras":"#.to_vec();
    document.extend_from_slice(extras);
    document.push(b'}');
    for (source, bytes) in [
        ("indexed.gltf", document.clone()),
        ("indexed.glb", glb_with_json(&document)),
    ] {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let names: Vec<_> = report.scene().nodes().map(|node| node.name()).collect();
        assert_eq!(names, [None, Some(""), Some("a\u{1d11e}")]);
        let entry = report
            .scene()
            .metadata_entries()
            .find(|entry| entry.key() == "org.baozi.gltf:extras")
            .expect("document extras are retained");
        let MetadataValueRef::Bytes {
            media_type,
            payload,
        } = entry.value()
        else {
            panic!("raw extras must remain bytes")
        };
        assert_eq!(media_type, "application/json");
        assert_eq!(payload, extras);
        assert!(report.loss_evidence().is_empty());
        assert_eq!(report.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn shared_image_texture_and_sampler_names_keep_their_own_metadata_owners() -> TestResult<()> {
    use baozi_import::MetadataOwnerRef;
    use serde_json::json;

    let mut document: serde_json::Value = serde_json::from_slice(&common::data_uri_gltf())?;
    document["images"] = json!([{ "uri": "data:image/png;base64,AA==", "name": "SharedImage" }]);
    document["samplers"] = json!([{ "name": "" }, { "name": "OwnSampler" }]);
    document["textures"] = json!([
        { "source": 0, "sampler": 0, "name": "OwnTexture" },
        { "source": 0, "sampler": 1, "name": "" },
        { "source": 0 }
    ]);
    document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": 0 });
    document["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
        json!({ "index": 1 });
    document["materials"][0]["emissiveTexture"] = json!({ "index": 2 });
    let (report, diagnostics) = import_assets(
        "source-names.gltf",
        [("source-names.gltf", serde_json::to_vec(&document)?)],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let scene = report.scene();
    assert_eq!(
        (
            scene.image_count(),
            scene.texture_count(),
            scene.sampler_count()
        ),
        (1, 3, 3)
    );
    let images: Vec<_> = scene.images().collect();
    let textures: Vec<_> = scene.textures().collect();
    let samplers: Vec<_> = scene.samplers().collect();
    for (owner, expected) in [
        (images[0].metadata_owner(), Some("SharedImage")),
        (textures[0].metadata_owner(), Some("OwnTexture")),
        (textures[1].metadata_owner(), Some("")),
        (textures[2].metadata_owner(), None),
        (samplers[0].metadata_owner(), Some("")),
        (samplers[1].metadata_owner(), Some("OwnSampler")),
        (samplers[2].metadata_owner(), None),
    ] {
        let entry = scene.metadata_entries().find(|entry| {
            entry.key() == "org.baozi.gltf:source-name"
                && entry.owner() == MetadataOwnerRef::Entity(owner)
        });
        let actual = entry.map(|entry| {
            let MetadataValueRef::String(value) = entry.value() else {
                panic!("source name must remain a string")
            };
            value
        });
        assert_eq!(actual, expected);
    }
    assert_eq!(report.ledger().current_live_bytes(), 0);
    Ok(())
}

fn metered_document() -> Vec<u8> {
    let nodes = std::iter::repeat_n(r#"{"name":"\u0061\u0062\u0063\uD834\uDD1E"}"#, 64)
        .collect::<Vec<_>>()
        .join(",");
    let materials = std::iter::repeat_n("{}", 192).collect::<Vec<_>>().join(",");
    let mut nested = String::from(r#"{"escaped\u002dkey":"\u0061"}"#);
    for _ in 0..64 {
        nested = format!("[{nested}]");
    }
    format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{nodes}],"materials":[{materials}],"vendor":{nested}}}"#).into_bytes()
}

#[test]
fn document_and_escaped_string_storage_obey_the_measured_live_boundary() -> TestResult<()> {
    const SOURCE: &str = "metered-document.gltf";
    let document = metered_document();
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, document.clone())],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let peak = report.ledger().peak_live_bytes();
    assert!(peak > document.len() as u64);
    drop(report);

    let mut exact = GltfTestOptions::memory();
    exact.limits.max_live_bytes = peak;
    let (report, diagnostics) = import_assets(SOURCE, [(SOURCE, document.clone())], exact)?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.ledger().peak_live_bytes(), peak);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    drop(report);

    let mut below = GltfTestOptions::memory();
    below.limits.max_live_bytes = peak - 1;
    let (result, _) = import_assets_result(SOURCE, [(SOURCE, document)], below)?;
    let failure = expected_error(result)?;
    assert_resource_limit(&failure, "max_live_bytes");
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}

#[test]
fn cancellation_releases_indexed_unknown_values_typed_rows_and_decoded_names() -> TestResult<()> {
    const SOURCE: &str = "cancel-document.gltf";
    let document = metered_document();
    let (control, polls) = controlled_import(None);
    let (result, diagnostics) = import_assets_result_with_control(
        SOURCE,
        [(SOURCE, document.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    let report = result?;
    let total_polls = polls.load(Ordering::SeqCst);
    assert!(total_polls > 4);
    drop(report);
    for cancel_on in [total_polls / 4, total_polls / 2, 3 * (total_polls / 4)] {
        let (control, polls) = controlled_import(Some(cancel_on));
        let (result, diagnostics) = import_assets_result_with_control(
            SOURCE,
            [(SOURCE, document.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert!(
            matches!(failure.cause(),ImportCause::Scope(abort) if matches!(abort.cause(),ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal })))
        );
        assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn omitted_core_rows_still_validate_references_hooks_and_unused_samplers() -> TestResult<()> {
    let invalid_rows = [
        r#""nodes":[{"mesh":0}]"#,
        r#""accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}]"#,
        r#""cameras":[{"type":"perspective","orthographic":{"xmag":1,"ymag":1,"znear":0,"zfar":10}}]"#,
        r#""textures":[{}]"#,
        r#""buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4,"byteStride":256}]"#,
        r#""buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]"#,
        r#""buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"SCALAR"}],"animations":[{"channels":[],"samplers":[{"input":0,"output":1}]}]"#,
    ];
    for (case, rows) in invalid_rows.into_iter().enumerate() {
        let json = format!(r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{}}],{rows}}}"#)
            .into_bytes();
        for (source, bytes) in [
            ("omitted-invalid.gltf", json.clone()),
            ("omitted-invalid.glb", glb_with_json(&json)),
        ] {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            let code = if case == 5 {
                GltfFailureCode::InvalidAccessor
            } else {
                GltfFailureCode::InvalidDocument
            };
            assert_gltf_failure(&failure, code, None)?;
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .expect("dependency dimension")
                    .attempted(),
                0
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}
