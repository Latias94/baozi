mod common;

use baozi_import::{MetadataOwnerRef, MetadataValueRef, SourceObjectLocatorRef, ValidatedScene};
use common::{
    GltfTestOptions, TestResult, expected_error, glb_with_json, import_assets, import_assets_result,
};

#[derive(Debug, PartialEq, Eq)]
struct SourceJson {
    locator: String,
    key: String,
    bytes: Vec<u8>,
}

fn source_json(scene: &ValidatedScene) -> Vec<SourceJson> {
    scene
        .metadata_entries()
        .filter_map(|entry| {
            let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                return None;
            };
            assert_eq!(source.generation(), scene.generation());
            assert_eq!(source.format(), "org.baozi.gltf");
            let SourceObjectLocatorRef::Namespaced { value, .. } = source.locator() else {
                panic!("glTF source locators must retain their declared schema");
            };
            let MetadataValueRef::Bytes {
                media_type,
                payload,
            } = entry.value()
            else {
                panic!("source extras must be a byte value");
            };
            assert_eq!(media_type, "application/json");
            Some(SourceJson {
                locator: value.to_owned(),
                key: entry.key().to_owned(),
                bytes: payload.to_vec(),
            })
        })
        .collect()
}

#[test]
fn json_values_preserve_raw_spelling_presence_and_glb_offsets() -> TestResult<()> {
    for raw in [
        "null",
        "{}",
        "[]",
        "true",
        "false",
        "-0",
        "1.2300e+04",
        "184467440737095516160",
        r#""\u0061\uD83D\uDE00""#,
        r#"{ "标签" : [null, {}, [], -0.00, "\u0061"], "extensions":{"VENDOR_data":{}} }"#,
    ] {
        let node = format!(r#"{{"name":"节点","extr\u0061s":{raw}}}"#);
        let json = format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{node}]}}"#);
        for is_glb in [false, true] {
            let bytes = if is_glb {
                glb_with_json(json.as_bytes())
            } else {
                json.as_bytes().to_vec()
            };
            let (report, diagnostics) = import_assets(
                "mapping.gltf",
                [("mapping.gltf", bytes)],
                GltfTestOptions::memory(),
            )?;
            assert!(diagnostics.is_empty());
            assert!(report.loss_evidence().is_empty());
            let output = source_json(report.scene());
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].key, "org.baozi.gltf:extras");
            let offset = json.find(&node).unwrap() + if is_glb { 20 } else { 0 };
            assert_eq!(
                output[0].locator,
                format!(
                    "kind=node;parent=-;index=0;offset={offset};length={}",
                    node.len()
                )
            );
            drop(report);
            assert_eq!(output[0].bytes, raw.as_bytes());
        }
    }
    let (report, _) = import_assets(
        "mapping.gltf",
        [(
            "mapping.gltf",
            br#"{"asset":{"version":"2.0"},"nodes":[{}]}"#.to_vec(),
        )],
        GltfTestOptions::memory(),
    )?;
    assert_eq!(report.scene().metadata_count(), 0);
    Ok(())
}

#[test]
fn asset_document_scene_and_nested_camera_keep_distinct_source_sites() -> TestResult<()> {
    let json = br#"{
      "asset":{"version":"2.0","extras":null},"extras":{},
      "scene":0,"scenes":[{"nodes":[0],"extras":[]}],
      "nodes":[{"camera":0,"extras":false}],
      "cameras":[{"type":"perspective","extras":true,"perspective":{"yfov":1,"znear":0.1,"extras":{"nested":1}}}]
    }"#;
    let (report, _) = import_assets(
        "mapping.gltf",
        [("mapping.gltf", json.to_vec())],
        GltfTestOptions::memory(),
    )?;
    let values = source_json(report.scene());
    assert_eq!(values.len(), 6);
    for (kind, key, raw) in [
        ("document", "org.baozi.gltf:extras", "{}"),
        ("document", "org.baozi.gltf:asset-extras", "null"),
        ("scene", "org.baozi.gltf:extras", "[]"),
        ("node", "org.baozi.gltf:extras", "false"),
        ("camera", "org.baozi.gltf:extras", "true"),
        (
            "camera",
            "org.baozi.gltf:perspective-extras",
            r#"{"nested":1}"#,
        ),
    ] {
        assert!(
            values
                .iter()
                .any(|v| v.locator.starts_with(&format!("kind={kind};"))
                    && v.key == key
                    && v.bytes == raw.as_bytes()),
            "{kind}/{key}"
        );
    }
    Ok(())
}

fn import_triangle(document: Vec<u8>) -> TestResult<baozi_import::ImportReport> {
    let (report, diagnostics) = import_assets(
        "mapping.gltf",
        [
            ("mapping.gltf", document),
            ("triangle.bin", common::triangle_bin()),
            ("textures/base.png", common::triangle_png()),
        ],
        common::sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    assert!(report.loss_evidence().is_empty());
    Ok(report)
}

#[test]
fn accessor_view_buffer_and_nested_primitives_have_one_source_owner_each() -> TestResult<()> {
    let mut json: serde_json::Value = serde_json::from_slice(&common::triangle_gltf())?;
    for (path, value) in [
        ("/buffers/0", 10),
        ("/bufferViews/0", 20),
        ("/accessors/0", 30),
    ] {
        json.pointer_mut(path).unwrap()["extras"] = value.into();
    }
    json["meshes"][0]["primitives"][0]["extras"] = 40.into();
    let mut other = json["meshes"][0].clone();
    other["primitives"][0]["extras"] = 50.into();
    json["meshes"].as_array_mut().unwrap().push(other);
    json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"mesh":1}));
    let index = json["nodes"].as_array().unwrap().len() - 1;
    json["scenes"][0]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(index.into());
    let report = import_triangle(serde_json::to_vec(&json)?)?;
    let values = source_json(report.scene());
    assert_eq!(values.len(), 5);
    for (prefix, raw) in [
        ("kind=buffer;parent=-;index=0;", "10"),
        ("kind=buffer-view;parent=-;index=0;", "20"),
        ("kind=accessor;parent=-;index=0;", "30"),
        ("kind=primitive;parent=0;index=0;", "40"),
        ("kind=primitive;parent=1;index=0;", "50"),
    ] {
        assert!(
            values
                .iter()
                .any(|v| v.locator.starts_with(prefix) && v.bytes == raw.as_bytes())
        );
    }
    Ok(())
}

#[test]
fn texture_transform_sites_do_not_collide_with_sibling_bindings() -> TestResult<()> {
    let mut json: serde_json::Value = serde_json::from_slice(&common::triangle_gltf())?;
    json["extensionsUsed"] = serde_json::json!(["KHR_texture_transform"]);
    let material = &mut json["materials"][0];
    for (path, marker) in [
        ("/pbrMetallicRoughness/baseColorTexture", 1),
        ("/pbrMetallicRoughness/metallicRoughnessTexture", 2),
        ("/normalTexture", 3),
        ("/occlusionTexture", 4),
        ("/emissiveTexture", 5),
    ] {
        let binding = serde_json::json!({"index":0, "extras":marker,
            "extensions":{"KHR_texture_transform":{"extras":[marker]}}});
        if let Some(name) = path.strip_prefix("/pbrMetallicRoughness/") {
            material["pbrMetallicRoughness"][name] = binding;
        } else {
            material[&path[1..]] = binding;
        }
    }
    let report = import_triangle(serde_json::to_vec(&json)?)?;
    let values = source_json(report.scene());
    assert_eq!(values.len(), 10);
    for (site, marker) in [
        ("base-color", 1),
        ("metallic-roughness", 2),
        ("normal", 3),
        ("occlusion", 4),
        ("emissive", 5),
    ] {
        assert!(
            values
                .iter()
                .any(|v| v.key == format!("org.baozi.gltf:{site}-texture-extras")
                    && v.bytes == marker.to_string().as_bytes())
        );
        assert!(values.iter().any(|v| v.key
            == format!("org.baozi.gltf:{site}-texture-transform-extras")
            && v.bytes == format!("[{marker}]").as_bytes()));
    }
    Ok(())
}

#[test]
fn shared_animation_sampler_and_channel_target_keep_source_ownership() -> TestResult<()> {
    use base64::Engine as _;
    let bytes: Vec<_> = [0.0_f32, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let json = serde_json::json!({
        "asset":{"version":"2.0"}, "scene":0,
        "scenes":[{"nodes":[0,1]}], "nodes":[{},{}],
        "buffers":[{"byteLength":32,"uri":format!("data:application/octet-stream;base64,{encoded}")}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":8},{"buffer":0,"byteOffset":8,"byteLength":24}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]},
            {"bufferView":1,"componentType":5126,"count":2,"type":"VEC3"}
        ],
        "animations":[{
            "extras":{},
            "samplers":[{"input":0,"output":1,"extras":null}],
            "channels":[
                {"sampler":0,"extras":false,"target":{"node":0,"path":"translation","extras":0}},
                {"sampler":0,"extras":true,"target":{"node":1,"path":"translation","extras":1}}
            ]
        }]
    });
    let (report, diagnostics) = import_assets(
        "mapping.gltf",
        [("mapping.gltf", serde_json::to_vec(&json)?)],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        report
            .scene()
            .animation_clips()
            .next()
            .unwrap()
            .channels()
            .len(),
        2
    );
    let values = source_json(report.scene());
    assert_eq!(values.len(), 6);
    for (prefix, key, raw) in [
        (
            "kind=animation;parent=-;index=0;",
            "org.baozi.gltf:extras",
            "{}",
        ),
        (
            "kind=animation-sampler;parent=0;index=0;",
            "org.baozi.gltf:extras",
            "null",
        ),
        (
            "kind=animation-channel;parent=0;index=0;",
            "org.baozi.gltf:extras",
            "false",
        ),
        (
            "kind=animation-channel;parent=0;index=0;",
            "org.baozi.gltf:target-extras",
            "0",
        ),
        (
            "kind=animation-channel;parent=0;index=1;",
            "org.baozi.gltf:extras",
            "true",
        ),
        (
            "kind=animation-channel;parent=0;index=1;",
            "org.baozi.gltf:target-extras",
            "1",
        ),
    ] {
        assert!(
            values.iter().any(|v| v.locator.starts_with(prefix)
                && v.key == key
                && v.bytes == raw.as_bytes())
        );
    }
    Ok(())
}

#[test]
fn metadata_bytes_admit_the_exact_boundary_and_reject_one_byte_over() -> TestResult<()> {
    // This limit also caps other canonical strings. Let the byte payload dominate those
    // small source/operation labels so the exact boundary isolates metadata admission.
    let raw = format!(r#"{{ "label" : "{}" }}"#, "x".repeat(4096));
    let json = format!(r#"{{"asset":{{"version":"2.0","extras":{raw}}}}}"#).into_bytes();
    let limit = "org.baozi.gltf:asset-extras".len() + "application/json".len() + raw.len();
    let mut options = GltfTestOptions::memory();
    options.limits.max_canonical_string_bytes = limit;
    let (report, _) = import_assets(
        "mapping.gltf",
        [("mapping.gltf", json.clone())],
        options.clone(),
    )?;
    assert_eq!(source_json(report.scene())[0].bytes, raw.as_bytes());
    options.limits.max_canonical_string_bytes = limit - 1;
    let (result, _) = import_assets_result("mapping.gltf", [("mapping.gltf", json)], options)?;
    let error = expected_error(result)?;
    common::assert_resource_limit(&error, "max_canonical_string_bytes");
    assert_eq!(error.ledger().current_live_bytes(), 0);
    assert_eq!(error.ledger().active_reservations(), 0);
    Ok(())
}

#[test]
fn oversized_selected_extras_fail_before_any_dependency_reference() -> TestResult<()> {
    let mut document: serde_json::Value = serde_json::from_slice(&common::triangle_gltf())?;
    document["asset"]["extras"] = serde_json::json!({"large":"x".repeat(1024)});
    let mut options = GltfTestOptions::memory();
    options.limits.max_canonical_string_bytes = 512;
    let (result, _) = import_assets_result(
        "mapping.gltf",
        [("mapping.gltf", serde_json::to_vec(&document)?)],
        options,
    )?;
    let error = expected_error(result)?;
    common::assert_resource_limit(&error, "max_canonical_string_bytes");
    assert_eq!(
        error
            .ledger()
            .dimension(baozi_import::ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0
    );
    assert_eq!(error.ledger().current_live_bytes(), 0);
    Ok(())
}
