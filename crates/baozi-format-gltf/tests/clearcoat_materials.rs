mod common;

use baozi_core::{
    AttributeElement, AttributeSemanticRef, ColorInterpretation, MaterialAssignment,
    MetadataOwnerRef, MetadataValueRef, ProvenanceKindRef, SceneSnapshotLimits, ShadingModel,
    SourceObjectLocatorRef, TextureBlendOperation, TextureChannel, TextureRole,
};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportDimension, ImportFailure, ImportLossKind, ImportReport, LossPolicy, SourceScopeEntityKind,
};
use common::{
    TestResult, assert_gltf_failure, expected_error, glb_with_json, import_assets,
    import_assets_result, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

const CLEARCOAT: &str = "KHR_materials_clearcoat";
const TRANSFORM: &str = "KHR_texture_transform";
const SOURCE: &str = "models/clearcoat.gltf";
const BINARY: &str = "models/triangle.bin";
const IMAGE: &str = "models/textures/base.png";

fn material_document(material: Value) -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!([CLEARCOAT]);
    document["materials"][0] = material;
    Ok(document)
}

fn coat_document(payload: Value) -> TestResult<Value> {
    material_document(json!({"extensions": {"KHR_materials_clearcoat": payload}}))
}

fn raw_material_document(material: &str) -> TestResult<Vec<u8>> {
    let mut document = material_document(json!({"name": "clearcoat source placeholder"}))?;
    document["extensionsUsed"] = json!([CLEARCOAT, TRANSFORM]);
    let original = serde_json::to_string(&document["materials"][0])?;
    let source = serde_json::to_string(&document)?;
    assert_eq!(source.matches(&original).count(), 1);
    Ok(source.replacen(&original, material, 1).into_bytes())
}

fn import_with_binary(document: Vec<u8>, binary: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [
            (SOURCE, document),
            (BINARY, binary),
            (IMAGE, triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    Ok(report)
}

fn import_document(document: Vec<u8>) -> TestResult<ImportReport> {
    import_with_binary(document, triangle_bin())
}

fn reject_before_dependencies(document: Vec<u8>, allow_loss: bool) -> TestResult<ImportFailure> {
    let mut options = sidecar_options();
    if allow_loss {
        options.loss_policy =
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss);
    }
    let (result, diagnostics) = import_assets_result(SOURCE, [(SOURCE, document)], options)?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let failure = expected_error(result)?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0,
        "clearcoat rejection must precede sidecar IO: {failure:?}",
    );
    assert!(failure.loss_evidence().is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(failure)
}

#[test]
fn absent_empty_and_explicit_zero_preserve_layer_presence() -> TestResult<()> {
    for (material, present) in [
        (json!({}), false),
        (json!({"extensions": {"KHR_materials_clearcoat": {}}}), true),
        (
            json!({"extensions": {"KHR_materials_clearcoat": {
                "clearcoatFactor": 0, "clearcoatRoughnessFactor": 0
            }}}),
            true,
        ),
    ] {
        let report = import_document(serde_json::to_vec(&material_document(material)?)?)?;
        let scene = report.scene();
        let material = scene.materials().next().unwrap();
        assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
        assert_eq!(material.factors().clearcoat().is_some(), present);
        if let Some(coat) = material.factors().clearcoat() {
            assert_eq!([coat.factor(), coat.roughness()].map(f32::to_bits), [0, 0]);
        }
        assert!(material.factors().sheen().is_none());
        assert_eq!(material.binding_count(), 0);
        assert_eq!((scene.texture_count(), scene.image_count()), (0, 0));
    }
    Ok(())
}

#[test]
fn unit_endpoints_subnormal_and_signed_zero_keep_authored_bits() -> TestResult<()> {
    for (factor, roughness) in [
        (0.0_f32, 1.0_f32),
        (1.0, 0.0),
        (-0.0, -0.0),
        (f32::from_bits(1), 0.625),
    ] {
        let mut document = coat_document(json!({
            "clearcoatFactor": factor, "clearcoatRoughnessFactor": roughness
        }))?;
        document["extensionsRequired"] = json!([CLEARCOAT]);
        let report = import_document(serde_json::to_vec(&document)?)?;
        let coat = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .clearcoat()
            .unwrap();
        assert_eq!(
            [coat.factor(), coat.roughness()].map(f32::to_bits),
            [factor, roughness].map(f32::to_bits),
        );
    }
    for number in ["-0.0e-400", "0.0e30"] {
        let material = format!(
            r#"{{"extensions":{{"KHR_materials_clearcoat":{{"clearcoatFactor":{number}}}}}}}"#
        );
        let report = import_document(raw_material_document(&material)?)?;
        let coat = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .clearcoat()
            .unwrap();
        assert_eq!(
            coat.factor().to_bits(),
            if number.starts_with('-') {
                0x8000_0000
            } else {
                0
            }
        );
    }
    Ok(())
}

#[test]
fn zero_layers_keep_all_clearcoat_bindings_and_independent_texture_semantics() -> TestResult<()> {
    let mut document = material_document(json!({
        "normalTexture": {"index": 0, "scale": -0.5},
        "extensions": {"KHR_materials_clearcoat": {
            "clearcoatTexture": {"index": 0, "texCoord": 7},
            "clearcoatRoughnessTexture": {"index": 0, "texCoord": 8},
            "clearcoatNormalTexture": {"index": 0, "texCoord": 9, "scale": -2.5}
        }}
    }))?;
    document["extensionsRequired"] = json!([CLEARCOAT]);
    let report = import_document(serde_json::to_vec(&document)?)?;
    let scene = report.scene();
    let material = scene.materials().next().unwrap();
    let coat = material.factors().clearcoat().unwrap();
    assert_eq!([coat.factor(), coat.roughness()], [0.0, 0.0]);
    assert_eq!(material.binding_count(), 4);
    assert_eq!(
        (
            scene.texture_count(),
            scene.image_count(),
            scene.sampler_count()
        ),
        (1, 1, 1)
    );
    let expected: [(TextureRole, &str, &[TextureChannel], u16, f32); 4] = [
        (
            TextureRole::Normal,
            "normalTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            0,
            -0.5,
        ),
        (
            TextureRole::Clearcoat,
            "clearcoatTexture",
            &[TextureChannel::Red],
            7,
            1.0,
        ),
        (
            TextureRole::ClearcoatRoughness,
            "clearcoatRoughnessTexture",
            &[TextureChannel::Green],
            8,
            1.0,
        ),
        (
            TextureRole::ClearcoatNormal,
            "clearcoatNormalTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            9,
            -2.5,
        ),
    ];
    let texture_id = material.bindings().next().unwrap().texture().id();
    for (binding, (role, key, channels, uv, strength)) in material.bindings().zip(expected) {
        assert_eq!(binding.role(), role);
        assert_eq!(binding.source_key(), Some(key));
        assert_eq!(binding.channel_mapping().as_slice(), channels);
        assert_eq!(binding.color_interpretation(), ColorInterpretation::Data);
        assert_eq!(binding.uv_set(), uv);
        assert_eq!(binding.strength().get().to_bits(), strength.to_bits());
        assert_eq!(binding.blend(), TextureBlendOperation::Replace);
        assert_eq!(binding.texture().id(), texture_id);
        assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
    }
    Ok(())
}

#[test]
fn each_clearcoat_slot_uses_the_same_uv_transform_contract() -> TestResult<()> {
    for (slot, role) in [
        ("clearcoatTexture", TextureRole::Clearcoat),
        ("clearcoatRoughnessTexture", TextureRole::ClearcoatRoughness),
        ("clearcoatNormalTexture", TextureRole::ClearcoatNormal),
    ] {
        for uv_set in [1_u16, u16::MAX] {
            let mut payload = json!({});
            payload[slot] = json!({"index": 0, "texCoord": 65536, "extensions": {
                "KHR_texture_transform": {
                    "offset": [-0.0, 0.25], "rotation": -0.375,
                    "scale": [2.0, -0.5], "texCoord": uv_set
                }
            }});
            let mut document = coat_document(payload)?;
            document["materials"][0]["normalTexture"] = json!({"index": 0});
            document["extensionsUsed"] = json!([CLEARCOAT, TRANSFORM]);
            let report = import_document(serde_json::to_vec(&document)?)?;
            let binding = report
                .scene()
                .materials()
                .next()
                .unwrap()
                .bindings()
                .find(|binding| binding.role() == role)
                .unwrap();
            assert_eq!(binding.uv_set(), uv_set);
            let transform = binding.transform();
            assert_eq!(
                [
                    transform.offset().x,
                    transform.offset().y,
                    transform.rotation_radians(),
                    transform.scale().x,
                    transform.scale().y
                ]
                .map(f32::to_bits),
                [-0.0_f32, 0.25, -0.375, 2.0, -0.5].map(f32::to_bits),
            );
        }
    }
    Ok(())
}

#[test]
fn selected_payload_types_null_fields_and_unit_domains_fail_before_io() -> TestResult<()> {
    for payload in [Value::Null, json!([]), json!(true), json!(0), json!("coat")] {
        let failure =
            reject_before_dependencies(serde_json::to_vec(&coat_document(payload)?)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for field in ["clearcoatFactor", "clearcoatRoughnessFactor"] {
        for value in [
            Value::Null,
            json!(false),
            json!([]),
            json!({}),
            json!("0.5"),
            json!(-0.25),
            json!(1.25),
        ] {
            let mut payload = json!({});
            payload[field] = value;
            let failure =
                reject_before_dependencies(serde_json::to_vec(&coat_document(payload)?)?, false)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
        for number in [
            "-1e-50",
            "1e-50",
            "-1e-400",
            "1e-400",
            "1.0000000001",
            "1e40",
            "1e400",
        ] {
            let material =
                format!(r#"{{"extensions":{{"KHR_materials_clearcoat":{{"{field}":{number}}}}}}}"#);
            let failure = reject_before_dependencies(raw_material_document(&material)?, false)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn selected_texture_shapes_indices_uvs_and_transforms_fail_before_io() -> TestResult<()> {
    for slot in [
        "clearcoatTexture",
        "clearcoatRoughnessTexture",
        "clearcoatNormalTexture",
    ] {
        for info in [
            Value::Null,
            json!([]),
            json!(false),
            json!({}),
            json!({"index": null}),
            json!({"index": -1}),
            json!({"index": 1}),
            json!({"index": 0, "texCoord": null}),
            json!({"index": 0, "texCoord": 65536}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": null}}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": {"texCoord": 65536}}}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": {"scale": [1e40, 1]}}}),
        ] {
            let mut document = coat_document(json!({}))?;
            document["extensionsUsed"] = json!([CLEARCOAT, TRANSFORM]);
            document["materials"][0]["extensions"][CLEARCOAT][slot] = info;
            let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    for value in [Value::Null, json!(false), json!("1"), json!(1e40)] {
        let document =
            coat_document(json!({"clearcoatNormalTexture": {"index": 0, "scale": value}}))?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn declarations_are_required_and_do_not_create_a_layer() -> TestResult<()> {
    for field in ["extensionsUsed", "extensionsRequired"] {
        let mut document = coat_document(json!({}))?;
        document[field] = json!([CLEARCOAT, CLEARCOAT]);
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for required in [false, true] {
        let mut document = coat_document(json!({}))?;
        document.as_object_mut().unwrap().remove("extensionsUsed");
        if required {
            document["extensionsRequired"] = json!([CLEARCOAT]);
        }
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    let mut document = material_document(json!({"extras": {"KHR_materials_clearcoat": {}}}))?;
    document["extensionsRequired"] = json!([CLEARCOAT]);
    let report = import_document(serde_json::to_vec(&document)?)?;
    assert!(
        report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .clearcoat()
            .is_none()
    );
    for slot in [
        "clearcoatTexture",
        "clearcoatRoughnessTexture",
        "clearcoatNormalTexture",
    ] {
        let mut payload = json!({});
        payload[slot] = json!({"index": 0, "extensions": {"KHR_texture_transform": {}}});
        let document = coat_document(payload)?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn unlit_is_excluded_and_specular_glossiness_remains_unsupported() -> TestResult<()> {
    for other in ["KHR_materials_unlit", "KHR_materials_pbrSpecularGlossiness"] {
        for allow_loss in [false, true] {
            let mut document = coat_document(json!({"clearcoatFactor": 0}))?;
            document["extensionsUsed"] = json!([CLEARCOAT, other]);
            document["materials"][0]["extensions"][other] = json!({});
            let failure = reject_before_dependencies(serde_json::to_vec(&document)?, allow_loss)?;
            assert_gltf_failure(
                &failure,
                if other == "KHR_materials_unlit" {
                    GltfFailureCode::InvalidDocument
                } else {
                    GltfFailureCode::UnsupportedFeature
                },
                None,
            )?;
        }
    }
    Ok(())
}

#[test]
fn misplaced_payloads_and_unknown_members_remain_pre_io_errors() -> TestResult<()> {
    for path in [
        "",
        "/asset",
        "/nodes/1",
        "/meshes/0/primitives/0",
        "/materials/0/pbrMetallicRoughness",
        "/textures/0",
        "/images/0",
    ] {
        let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
        document["extensionsUsed"] = json!([CLEARCOAT]);
        document.pointer_mut(path).unwrap()["extensions"] = json!({"KHR_materials_clearcoat": {}});
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    for payload in [
        json!({"unexpected": null}),
        json!({"extensions": {"VENDOR_nested": {}}}),
        json!({"clearcoatTexture": {"index": 0, "extensions": {"KHR_materials_clearcoat": {}}}}),
    ] {
        let failure =
            reject_before_dependencies(serde_json::to_vec(&coat_document(payload)?)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    Ok(())
}

#[test]
fn omitted_invalid_layers_and_texture_references_do_not_enter_the_pool() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!({"clearcoatFactor": -1}),
        json!({"clearcoatNormalTexture": {"index": 999}}),
        json!({"extensions": {"VENDOR_nested": {}}}),
    ] {
        let mut document = material_document(json!({}))?;
        document["materials"].as_array_mut().unwrap().push(json!({
            "extensions": {"KHR_materials_clearcoat": payload},
            "emissiveTexture": {"index": 1}
        }));
        document["images"]
            .as_array_mut()
            .unwrap()
            .push(json!({"uri": "not-selected.png"}));
        document["textures"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source": 1}));
        let report = import_document(serde_json::to_vec(&document)?)?;
        let scene = report.scene();
        assert_eq!(
            (
                scene.material_count(),
                scene.texture_count(),
                scene.image_count()
            ),
            (1, 0, 0)
        );
        assert!(
            scene
                .materials()
                .next()
                .unwrap()
                .factors()
                .clearcoat()
                .is_none()
        );
        let scope = report.source_scope_record().unwrap();
        assert_eq!(scope.counts(SourceScopeEntityKind::Material).selected(), 1);
        assert_eq!(scope.counts(SourceScopeEntityKind::Material).omitted(), 1);
        assert!(
            !report
                .successful_input_manifest()
                .node_classes()
                .iter()
                .any(|class| class.identity().value().as_str().contains("not-selected"))
        );
    }
    Ok(())
}

#[test]
fn coat_normal_without_base_normal_requires_both_authored_frame_attributes_before_io()
-> TestResult<()> {
    for (normal, tangent) in [(false, false), (true, false), (false, true)] {
        let mut document = coat_document(json!({"clearcoatNormalTexture": {"index": 0}}))?;
        let attributes = document["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap();
        if !normal {
            attributes.remove("NORMAL");
        }
        if tangent {
            attributes.insert("TANGENT".into(), json!(4));
            // Give the accessor a valid stored layout so an implicit-zero tangent
            // failure cannot mask the missing NORMAL prerequisite before IO.
            document["bufferViews"]
                .as_array_mut()
                .unwrap()
                .push(json!({"buffer":0,"byteLength":48,"target":34962}));
            document["accessors"]
                .as_array_mut()
                .unwrap()
                .push(json!({"bufferView":4,"componentType": 5126, "count": 3, "type": "VEC4"}));
        }
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidDocument,
            Some("clearcoatNormalTexture requires NORMAL and TANGENT"),
        )?;
    }
    Ok(())
}

#[test]
fn base_normal_keeps_coat_normal_even_without_authored_frames_or_positive_factor() -> TestResult<()>
{
    for scale in [-2.5_f32, -0.0, f32::from_bits(1), f32::MAX] {
        let mut document =
            coat_document(json!({"clearcoatNormalTexture": {"index": 0, "scale": scale}}))?;
        document["materials"][0]["normalTexture"] = json!({"index": 0});
        document["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("NORMAL");
        let report = import_document(serde_json::to_vec(&document)?)?;
        let material = report.scene().materials().next().unwrap();
        assert_eq!(material.factors().clearcoat().unwrap().factor(), 0.0);
        assert_eq!(material.binding_count(), 2);
        let coat_normal = material
            .bindings()
            .find(|binding| binding.role() == TextureRole::ClearcoatNormal)
            .unwrap();
        assert_eq!(coat_normal.strength().get().to_bits(), scale.to_bits());
    }
    Ok(())
}

#[test]
fn coat_normal_preserves_authored_frames_across_a_mirrored_uv_seam() -> TestResult<()> {
    let mut document = coat_document(json!({"clearcoatNormalTexture": {"index": 0}}))?;
    document["buffers"][0]["byteLength"] = json!(300);
    document["bufferViews"] = json!([
        {"buffer": 0, "byteLength": 72, "target": 34962},
        {"buffer": 0, "byteOffset": 72, "byteLength": 72, "target": 34962},
        {"buffer": 0, "byteOffset": 144, "byteLength": 48, "target": 34962},
        {"buffer": 0, "byteOffset": 192, "byteLength": 96, "target": 34962},
        {"buffer": 0, "byteOffset": 288, "byteLength": 12, "target": 34963}
    ]);
    document["accessors"] = json!([
        {"bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
        {"bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3"},
        {"bufferView": 2, "componentType": 5126, "count": 6, "type": "VEC2"},
        {"bufferView": 3, "componentType": 5126, "count": 6, "type": "VEC4"},
        {"bufferView": 4, "componentType": 5123, "count": 6, "type": "SCALAR"}
    ]);
    document["meshes"][0]["primitives"][0]["attributes"]["TANGENT"] = json!(3);
    document["meshes"][0]["primitives"][0]["indices"] = json!(4);
    let mut binary = Vec::new();
    for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .repeat(2)
        .into_iter()
        .chain([0.0, 0.0, 1.0].repeat(6))
        .chain([0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0])
        .chain([1.0, 0.0, 0.0, 1.0].repeat(3))
        .chain([-1.0, 0.0, 0.0, -1.0].repeat(3))
    {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0_u16, 1, 2, 3, 4, 5] {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(binary.len(), 300);
    let report = import_with_binary(serde_json::to_vec(&document)?, binary)?;
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let tangent = primitive
        .attributes()
        .find(|attribute| attribute.semantic() == AttributeSemanticRef::Tangent)
        .unwrap();
    let normal = primitive
        .attributes()
        .find(|attribute| attribute.semantic() == AttributeSemanticRef::Normal)
        .unwrap();
    assert!(tangent.tangent_conventions().is_some());
    for vertex in 0..6 {
        let expected = if vertex < 3 {
            [1.0_f32, 0.0, 0.0, 1.0]
        } else {
            [-1.0, 0.0, 0.0, -1.0]
        };
        assert_eq!(
            tangent.element(vertex),
            Some(AttributeElement::F32(&expected))
        );
        assert_eq!(
            normal.element(vertex),
            Some(AttributeElement::F32(&[0.0, 0.0, 1.0]))
        );
    }
    assert_eq!(
        report.scene().materials().next().unwrap().binding_count(),
        1
    );
    Ok(())
}

#[test]
fn exact_layer_texture_and_transform_extras_keep_material_provenance_in_json_and_glb()
-> TestResult<()> {
    let raw = r#"{ "label" : "\u0061", "number": 1.2300e+04, "extensions":{"VENDOR_data":{}} }"#;
    let material = format!(
        r#"{{"name":"Coated","normalTexture":{{"index":0}},"extras":{{"parent":true}},"extensions":{{"KHR_materials_clearc\u006fat":{{"extras":{raw},"clearcoatTexture":{{"index":0,"extras":[1],"extensions":{{"KHR_texture_transform":{{"extras":null}}}}}},"clearcoatRoughnessTexture":{{"index":0,"extras":{{}},"extensions":{{"KHR_texture_transform":{{"extras":[]}}}}}},"clearcoatNormalTexture":{{"index":0,"extras":"normal","extensions":{{"KHR_texture_transform":{{"extras":false}}}}}}}}}}}}"#
    );
    let json = raw_material_document(&material)?;
    let offset = std::str::from_utf8(&json)?.find(&material).unwrap();
    for glb in [false, true] {
        let report = import_document(if glb {
            glb_with_json(&json)
        } else {
            json.clone()
        })?;
        let scene = report.scene();
        let source = scene.source_identities().find(|source| matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value.starts_with("kind=material;"))).unwrap();
        let locator = format!(
            "kind=material;parent=-;index=0;offset={};length={}",
            offset + if glb { 20 } else { 0 },
            material.len()
        );
        assert_eq!(source.resource().identity(), SOURCE);
        assert_eq!(
            source.locator(),
            SourceObjectLocatorRef::Namespaced {
                schema: "org.baozi.gltf:source-object",
                schema_version: 1,
                value: &locator
            }
        );
        let entries = source.metadata_entries().collect::<Vec<_>>();
        assert_eq!(entries.len(), 8);
        for (key, expected) in [
            ("org.baozi.gltf:extras", r#"{"parent":true}"#),
            ("org.baozi.gltf:clearcoat-extras", raw),
            ("org.baozi.gltf:clearcoat-texture-extras", "[1]"),
            ("org.baozi.gltf:clearcoat-texture-transform-extras", "null"),
            ("org.baozi.gltf:clearcoat-roughness-texture-extras", "{}"),
            (
                "org.baozi.gltf:clearcoat-roughness-texture-transform-extras",
                "[]",
            ),
            (
                "org.baozi.gltf:clearcoat-normal-texture-extras",
                r#""normal""#,
            ),
            (
                "org.baozi.gltf:clearcoat-normal-texture-transform-extras",
                "false",
            ),
        ] {
            let entry = entries.iter().find(|entry| entry.key() == key).unwrap();
            assert_eq!(entry.owner(), MetadataOwnerRef::SourceObject(source));
            let MetadataValueRef::Bytes {
                media_type,
                payload,
            } = entry.value()
            else {
                panic!("extras must retain JSON bytes");
            };
            assert_eq!(media_type, "application/json");
            assert_eq!(payload, expected.as_bytes());
        }
        for binding in scene.materials().next().unwrap().bindings() {
            assert!(scene.owner_provenance(binding.metadata_owner())?.any(|node| matches!(node.kind(), ProvenanceKindRef::Source(candidate) if candidate == source)));
            assert!(scene.owner_provenance(binding.metadata_owner())?.any(|node| matches!(node.kind(), ProvenanceKindRef::Source(candidate) if matches!(candidate.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value.starts_with("kind=texture;")))));
        }
    }
    Ok(())
}

#[test]
fn snapshots_distinguish_absence_and_zero_and_implicit_default_stays_absent() -> TestResult<()> {
    let mut digests = Vec::new();
    for payload in [
        None,
        Some(json!({})),
        Some(json!({"clearcoatFactor": 0.75})),
    ] {
        let document = if let Some(payload) = payload {
            coat_document(payload)?
        } else {
            material_document(json!({}))?
        };
        let report = import_document(serde_json::to_vec(&document)?)?;
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 128, 1_000_000)?)?;
        digests.push(snapshot.digest());
    }
    assert_ne!(digests[0], digests[1]);
    assert_ne!(digests[1], digests[2]);
    let mut document = coat_document(json!({}))?;
    let mut implicit = document["meshes"][0]["primitives"][0].clone();
    implicit.as_object_mut().unwrap().remove("material");
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(implicit);
    let report = import_document(serde_json::to_vec(&document)?)?;
    let primitives = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .collect::<Vec<_>>();
    assert!(
        primitives[0]
            .authored_material()
            .unwrap()
            .factors()
            .clearcoat()
            .is_some()
    );
    assert!(
        primitives[1]
            .effective_material()
            .unwrap()
            .factors()
            .clearcoat()
            .is_none()
    );
    assert!(matches!(
        primitives[1].material_assignment(),
        MaterialAssignment::ImplicitDefault(_)
    ));
    Ok(())
}
