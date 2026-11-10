mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportDimension, ImportFailure, ImportLedgerSnapshot, ImportTerminalCause,
    ImportTerminalFailureClass, MetadataValueRef,
};
use base64::Engine as _;
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json, import_assets,
    import_assets_result,
};
use serde_json::{Value, json};

fn encodings(document: &Value) -> TestResult<[(&'static str, Vec<u8>); 2]> {
    let json = serde_json::to_vec(document)?;
    let glb = glb_with_json(&json);
    Ok([("null-fields.gltf", json), ("null-fields.glb", glb)])
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

fn assert_invalid_before_dependencies(failure: &ImportFailure) -> TestResult<()> {
    assert_gltf_failure(failure, GltfFailureCode::InvalidDocument, None)?;
    assert!(failure.diagnostics().is_empty());
    assert!(failure.source_scope_record().is_none());
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_released_without_dependencies(failure.ledger());
    assert!(matches!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(terminal)
            if terminal.class() == ImportTerminalFailureClass::Requested
    ));
    Ok(())
}

fn core_document(selected: bool) -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&common::data_uri_gltf())?;
    document["asset"]["copyright"] = json!("Public domain");
    document["scenes"][0] = if selected {
        json!({"nodes": [0]})
    } else {
        json!({})
    };
    document["scenes"][0]["name"] = json!("Main");
    document["nodes"][0]["translation"] = json!([0, 0, 0]);
    document["nodes"][0]["rotation"] = json!([0, 0, 0, 1]);
    document["nodes"][0]["scale"] = json!([1, 1, 1]);
    document["nodes"][0]["camera"] = json!(0);
    document["nodes"][0]["extensions"] = json!({});
    document["nodes"][1]["matrix"] = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    document["nodes"][1]["camera"] = json!(1);
    document["nodes"][1]["weights"] = json!([0]);
    document["buffers"][0]["name"] = json!("Geometry");
    document["bufferViews"][0]["name"] = json!("Positions");
    document["bufferViews"][0]["byteStride"] = json!(12);
    // Reuse the fixture's first index and first position as an unchanged sparse replacement.
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 96, "byteLength": 1}),
        json!({"buffer": 0, "byteLength": 12}),
    ]);
    document["accessors"][0]["name"] = json!("Positions");
    document["accessors"][0]["byteOffset"] = json!(0);
    document["accessors"][0]["sparse"] = json!({
        "count": 1,
        "indices": {"bufferView": 4, "componentType": 5121},
        "values": {"bufferView": 5}
    });
    document["accessors"][1]["min"] = json!([0, 0, 1]);
    document["accessors"][1]["max"] = json!([0, 0, 1]);
    document["meshes"][0]["primitives"][0]["targets"] = json!([{}]);
    document["meshes"][0]["weights"] = json!([0]);
    let image_uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(common::triangle_png())
    );
    document["images"] = json!([{
        "name": "Image", "uri": image_uri, "mimeType": "image/png"
    }]);
    document["textures"] = json!([{"name": "Texture", "source": 0, "sampler": 0}]);
    document["samplers"] = json!([{"name": "Sampler", "magFilter": 9729, "minFilter": 9987}]);
    document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] =
        json!({"index": 0, "texCoord": 0, "extensions": {}});
    document["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
        json!({"index": 0});
    document["materials"][0]["normalTexture"] = json!({"index": 0, "scale": 1});
    document["materials"][0]["occlusionTexture"] = json!({"index": 0, "strength": 1});
    document["materials"][0]["emissiveTexture"] = json!({"index": 0});
    document["materials"][0]["alphaCutoff"] = json!(0.5);
    document["materials"][0]["extensions"] = json!({});
    document["cameras"] = json!([
        {"name": "Perspective", "type": "perspective",
         "perspective": {"yfov": 1, "znear": 0.1, "zfar": 100, "aspectRatio": 1}},
        {"name": "Orthographic", "type": "orthographic",
         "orthographic": {"xmag": 1, "ymag": 1, "znear": 0.1, "zfar": 100}}
    ]);
    Ok(document)
}

#[test]
fn core_null_fields_reject_selected_and_omitted_rows_before_io() -> TestResult<()> {
    let fields = [
        "/asset/generator",
        "/asset/copyright",
        "/scene",
        "/scenes/0/name",
        "/nodes/0/name",
        "/nodes/0/children",
        "/nodes/0/translation",
        "/nodes/0/rotation",
        "/nodes/0/scale",
        "/nodes/0/camera",
        "/nodes/0/extensions",
        "/nodes/1/matrix",
        "/nodes/1/mesh",
        "/nodes/1/skin",
        "/nodes/1/weights",
        "/accessors/0/name",
        "/accessors/0/byteOffset",
        "/accessors/0/bufferView",
        "/accessors/0/sparse",
        "/accessors/0/min",
        "/accessors/0/max",
        "/accessors/1/min",
        "/accessors/1/max",
        "/buffers/0/name",
        "/buffers/0/uri",
        "/bufferViews/0/name",
        "/bufferViews/0/byteOffset",
        "/bufferViews/0/byteStride",
        "/bufferViews/0/target",
        "/meshes/0/name",
        "/meshes/0/weights",
        "/meshes/0/primitives/0/indices",
        "/meshes/0/primitives/0/material",
        "/meshes/0/primitives/0/targets",
        "/images/0/name",
        "/images/0/uri",
        "/images/0/mimeType",
        "/images/0/bufferView",
        "/textures/0/name",
        "/textures/0/sampler",
        "/samplers/0/name",
        "/samplers/0/magFilter",
        "/samplers/0/minFilter",
        "/materials/0/name",
        "/materials/0/alphaCutoff",
        "/materials/0/pbrMetallicRoughness",
        "/materials/0/pbrMetallicRoughness/baseColorTexture",
        "/materials/0/pbrMetallicRoughness/baseColorTexture/texCoord",
        "/materials/0/pbrMetallicRoughness/baseColorTexture/extensions",
        "/materials/0/pbrMetallicRoughness/metallicRoughnessTexture",
        "/materials/0/normalTexture",
        "/materials/0/occlusionTexture",
        "/materials/0/emissiveTexture",
        "/materials/0/extensions",
        "/cameras/0/name",
        "/cameras/0/perspective",
        "/cameras/0/perspective/aspectRatio",
        "/cameras/0/perspective/zfar",
        "/cameras/1/orthographic",
    ];
    for selected in [false, true] {
        let baseline = core_document(selected)?;
        for (source, bytes) in encodings(&baseline)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert_eq!(report.scene().mesh_count(), usize::from(selected));
            assert_eq!(report.scene().camera_count(), if selected { 2 } else { 0 });
            assert!(report.loss_evidence().is_empty());
            assert_released_without_dependencies(report.ledger());
        }
        for pointer in fields {
            let mut document = baseline.clone();
            // Missing sidecars must never mask the typed schema error, even when selected.
            document["buffers"][0]["uri"] = json!("never.bin");
            document["images"][0]["uri"] = json!("never.png");
            let (owner, field) = pointer.rsplit_once('/').unwrap();
            document.pointer_mut(owner).expect("fixture owner exists")[field] = Value::Null;
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(
                    diagnostics.is_empty(),
                    "{pointer}: selected={selected} {source}"
                );
                let failure = expected_error(result)?;
                assert_invalid_before_dependencies(&failure)
                    .map_err(|error| format!("{pointer}: selected={selected} {source}: {error}"))?;
            }
        }
    }
    Ok(())
}

#[test]
fn absent_optional_core_fields_keep_valid_defaults() -> TestResult<()> {
    for selected in [false, true] {
        let mut document = core_document(selected)?;
        for (owner, fields) in [
            ("/asset", &["generator", "copyright"][..]),
            (
                "/nodes/0",
                &[
                    "name",
                    "translation",
                    "rotation",
                    "scale",
                    "camera",
                    "extensions",
                ],
            ),
            ("/nodes/1", &["matrix", "weights", "camera"]),
            ("/accessors/0", &["name", "byteOffset", "sparse"]),
            ("/accessors/1", &["min", "max"]),
            (
                "/bufferViews/0",
                &["name", "byteOffset", "byteStride", "target"],
            ),
            ("/meshes/0/primitives/0", &["indices", "material"]),
            (
                "/materials/0",
                &[
                    "name",
                    "alphaCutoff",
                    "normalTexture",
                    "occlusionTexture",
                    "emissiveTexture",
                    "extensions",
                ],
            ),
            (
                "/materials/0/pbrMetallicRoughness",
                &["baseColorTexture", "metallicRoughnessTexture"],
            ),
            ("/textures/0", &["name", "sampler"]),
            ("/samplers/0", &["name", "magFilter", "minFilter"]),
            ("/cameras/0/perspective", &["aspectRatio", "zfar"]),
        ] {
            let object = document
                .pointer_mut(owner)
                .unwrap()
                .as_object_mut()
                .unwrap();
            for field in fields {
                assert!(object.remove(*field).is_some());
            }
        }
        for (source, bytes) in encodings(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert_eq!(report.scene().mesh_count(), usize::from(selected));
            assert!(report.loss_evidence().is_empty());
            assert_released_without_dependencies(report.ledger());
        }
    }
    Ok(())
}

#[test]
fn source_free_accessor_null_fields_never_become_absent() -> TestResult<()> {
    for selected in [false, true] {
        let mut baseline: Value = serde_json::from_slice(&common::data_uri_gltf())?;
        baseline["scenes"][0] = if selected {
            json!({"nodes": [0]})
        } else {
            json!({})
        };
        baseline["accessors"] = json!([{
            "componentType": 5126, "count": 1, "type": "VEC3",
            "min": [0, 0, 0], "max": [0, 0, 0]
        }]);
        baseline["meshes"][0]["primitives"] = json!([{
            "attributes": {"POSITION": 0}, "mode": 0
        }]);
        baseline["buffers"][0]["uri"] = json!("never.bin");
        for (source, bytes) in encodings(&baseline)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert_eq!(report.scene().mesh_count(), usize::from(selected));
            assert_released_without_dependencies(report.ledger());
        }
        for field in ["byteOffset", "bufferView", "sparse"] {
            let mut document = baseline.clone();
            document["accessors"][0][field] = Value::Null;
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                assert_invalid_before_dependencies(&expected_error(result)?)?;
            }
        }
    }
    Ok(())
}

#[test]
fn extras_null_remains_an_exact_raw_payload_in_json_and_glb() -> TestResult<()> {
    for selected in [false, true] {
        let mut document = core_document(selected)?;
        document["extras"] = Value::Null;
        document["asset"]["extras"] = Value::Null;
        for (source, bytes) in encodings(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert!(report.loss_evidence().is_empty());
            for key in ["org.baozi.gltf:extras", "org.baozi.gltf:asset-extras"] {
                let entry = report
                    .scene()
                    .metadata_entries()
                    .find(|entry| entry.key() == key)
                    .expect("present null extras must be retained");
                let MetadataValueRef::Bytes {
                    media_type,
                    payload,
                } = entry.value()
                else {
                    panic!("extras must retain their raw JSON payload");
                };
                assert_eq!(media_type, "application/json");
                assert_eq!(payload, b"null");
            }
            assert_released_without_dependencies(report.ledger());
        }
    }
    Ok(())
}
