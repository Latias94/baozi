mod common;

use baozi_core::{
    ColorInterpretation, MaterialAssignment, MetadataOwnerRef, MetadataValueRef, ProvenanceKindRef,
    ShadingModel, SourceObjectLocatorRef, TextureRole,
};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    FormatFailureDetailValue, ImportDimension, ImportFailure, ImportLossKind, ImportReport,
    LossPolicy, SourceScopeEntityKind,
};
use common::{
    TestResult, assert_gltf_failure, expected_error, glb_with_json, gltf_failure, import_assets,
    import_assets_result, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

const STRENGTH: &str = "KHR_materials_emissive_strength";
const UNLIT: &str = "KHR_materials_unlit";
const SOURCE: &str = "models/emission.gltf";
const BINARY: &str = "models/triangle.bin";
const IMAGE: &str = "models/textures/base.png";

fn material_document(material: Value) -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!([STRENGTH]);
    document["materials"][0] = material;
    Ok(document)
}

fn strength_document(payload: Value) -> TestResult<Value> {
    material_document(json!({
        "emissiveFactor": [0.25, 0.5, 1.0],
        "extensions": {"KHR_materials_emissive_strength": payload}
    }))
}

fn raw_material_document(material: &str) -> TestResult<Vec<u8>> {
    let document = strength_document(json!({}))?;
    let original = serde_json::to_string(&document["materials"][0])?;
    let source = serde_json::to_string(&document)?;
    assert_eq!(source.matches(&original).count(), 1);
    Ok(source.replacen(&original, material, 1).into_bytes())
}

fn import_document(document: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [
            (SOURCE, document),
            (BINARY, triangle_bin()),
            (IMAGE, triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    Ok(report)
}

fn reject_before_dependencies(
    document: Vec<u8>,
    allow_semantic_loss: bool,
) -> TestResult<ImportFailure> {
    let mut options = sidecar_options();
    if allow_semantic_loss {
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
            .expect("dependency references are configured")
            .attempted(),
        0,
        "material rejection must precede sidecar IO: {failure:?}",
    );
    assert!(failure.loss_evidence().is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(failure)
}

#[test]
fn strength_preserves_raw_factors_and_unit_texture_strength() -> TestResult<()> {
    for strength in [0.0_f32, -0.0, 1.0, 8.0, f32::MAX, f32::from_bits(1)] {
        for textured in [false, true] {
            let mut document = strength_document(json!({"emissiveStrength": strength}))?;
            document["extensionsRequired"] = json!([STRENGTH]);
            if textured {
                document["materials"][0]["emissiveTexture"] = json!({"index": 0});
            }
            let report = import_document(serde_json::to_vec(&document)?)?;
            let scene = report.scene();
            let material = scene.materials().next().unwrap();
            assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
            let emission = material.factors().emission();
            let factor = emission.factor();
            assert_eq!(
                [factor.x, factor.y, factor.z].map(f32::to_bits),
                [0.25_f32, 0.5, 1.0].map(f32::to_bits),
            );
            assert_eq!(
                emission.strength().map(f32::to_bits),
                Some(strength.to_bits())
            );
            assert_eq!(emission.effective_strength().to_bits(), strength.to_bits());
            assert_eq!(
                emission.effective_rgb().map(f64::to_bits),
                [
                    0.25_f64 * f64::from(strength),
                    0.5_f64 * f64::from(strength),
                    f64::from(strength),
                ]
                .map(f64::to_bits),
            );
            assert_eq!(material.binding_count(), usize::from(textured));
            assert_eq!(scene.texture_count(), usize::from(textured));
            assert_eq!(scene.image_count(), usize::from(textured));
            if textured {
                let binding = material.bindings().next().unwrap();
                assert_eq!(binding.role(), TextureRole::Emissive);
                assert_eq!(binding.source_key(), Some("emissiveTexture"));
                assert_eq!(binding.strength().get().to_bits(), 1.0_f32.to_bits());
                assert_eq!(
                    binding.color_interpretation(),
                    ColorInterpretation::SrgbColor
                );
                assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
            }
        }
    }
    Ok(())
}

#[test]
fn absent_empty_and_explicit_one_keep_distinct_authored_presence() -> TestResult<()> {
    for (extensions, expected) in [
        (json!({}), None),
        (
            json!({"KHR_materials_emissive_strength": {}}),
            Some(1.0_f32),
        ),
        (
            json!({"KHR_materials_emissive_strength": {"emissiveStrength": 1}}),
            Some(1.0),
        ),
    ] {
        let document = material_document(json!({
            "emissiveFactor": [0.25, 0.5, 1.0], "extensions": extensions
        }))?;
        let report = import_document(serde_json::to_vec(&document)?)?;
        let material = report.scene().materials().next().unwrap();
        let emission = material.factors().emission();
        assert_eq!(
            emission.strength().map(f32::to_bits),
            expected.map(f32::to_bits)
        );
        assert_eq!(emission.effective_strength(), 1.0);
        assert_eq!(emission.effective_rgb(), [0.25, 0.5, 1.0]);
    }
    Ok(())
}

#[test]
fn declarations_and_descriptive_extras_do_not_create_strength() -> TestResult<()> {
    for required in [false, true] {
        let mut document = material_document(json!({
            "extras": {"KHR_materials_emissive_strength": {"emissiveStrength": 8}}
        }))?;
        if required {
            document["extensionsRequired"] = json!([STRENGTH]);
        }
        let report = import_document(serde_json::to_vec(&document)?)?;
        let emission = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .emission();
        assert_eq!(emission.strength(), None);
        assert_eq!(emission.effective_rgb(), [0.0; 3]);
    }
    Ok(())
}

#[test]
fn zero_factor_keeps_strength_and_hdr_product_is_computed_in_f64() -> TestResult<()> {
    for (factor, strength, expected) in [
        ([0.0_f32; 3], 8.0_f32, [0.0_f64; 3]),
        (
            [f32::MAX, 2.0, -0.0],
            f32::MAX,
            [
                f64::from(f32::MAX) * f64::from(f32::MAX),
                2.0 * f64::from(f32::MAX),
                -0.0,
            ],
        ),
    ] {
        let document = material_document(json!({
            "emissiveFactor": factor,
            "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": strength}}
        }))?;
        let report = import_document(serde_json::to_vec(&document)?)?;
        let emission = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .emission();
        let raw = emission.factor();
        assert_eq!(
            [raw.x, raw.y, raw.z].map(f32::to_bits),
            factor.map(f32::to_bits)
        );
        assert_eq!(
            emission.strength().map(f32::to_bits),
            Some(strength.to_bits())
        );
        assert_eq!(
            emission.effective_rgb().map(f64::to_bits),
            expected.map(f64::to_bits)
        );
        assert!(emission.effective_rgb().into_iter().all(f64::is_finite));
    }
    Ok(())
}

#[test]
fn selected_invalid_payloads_and_members_fail_before_sidecar_io() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!(true),
        json!(0),
        json!("strength"),
        json!({"emissiveStrength": null}),
        json!({"emissiveStrength": false}),
        json!({"emissiveStrength": []}),
        json!({"emissiveStrength": {}}),
        json!({"emissiveStrength": "8"}),
        json!({"emissiveStrength": -1}),
    ] {
        let document = strength_document(payload)?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn negative_overflow_and_nonzero_underflow_are_rejected_before_narrowing() -> TestResult<()> {
    for strength in [
        "-0.5", "-1e-50", "1e-50", "-1e-400", "1e-400", "1e40", "1e400",
    ] {
        let material = format!(
            r#"{{"emissiveFactor":[0,0,0],"extensions":{{"KHR_materials_emissive_strength":{{"emissiveStrength":{strength}}}}}}}"#
        );
        let failure = reject_before_dependencies(raw_material_document(&material)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn zero_mantissas_keep_signed_zero_with_nonzero_exponents() -> TestResult<()> {
    for (number, expected) in [("0.0e30", 0.0_f32), ("-0.0e-400", -0.0_f32)] {
        let material = format!(
            r#"{{"extensions":{{"KHR_materials_emissive_strength":{{"emissiveStrength":{number}}}}}}}"#
        );
        let report = import_document(raw_material_document(&material)?)?;
        let emission = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .emission();
        assert_eq!(
            emission.strength().map(f32::to_bits),
            Some(expected.to_bits())
        );
    }
    Ok(())
}

#[test]
fn selected_base_factors_and_alpha_are_also_compiled_before_sidecar_io() -> TestResult<()> {
    for material in [
        json!({"emissiveFactor": [-1, 0, 0]}),
        json!({"pbrMetallicRoughness": {"metallicFactor": 2}}),
        json!({"alphaMode": "MASK", "alphaCutoff": -0.5}),
    ] {
        let mut document = material_document(material)?;
        document["materials"][0]["extensions"] =
            json!({"KHR_materials_emissive_strength": {"emissiveStrength": 8}});
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn payload_and_required_declarations_need_extensions_used_before_io() -> TestResult<()> {
    for (payload, required) in [(true, false), (true, true), (false, true)] {
        let mut document = strength_document(json!({}))?;
        document.as_object_mut().unwrap().remove("extensionsUsed");
        if !payload {
            document["materials"][0]
                .as_object_mut()
                .unwrap()
                .remove("extensions");
        }
        if required {
            document["extensionsRequired"] = json!([STRENGTH]);
        }
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for field in ["extensionsUsed", "extensionsRequired"] {
        let mut document = strength_document(json!({}))?;
        document[field] = json!([STRENGTH, STRENGTH]);
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn strength_and_unlit_are_incompatible_even_with_zero_emission() -> TestResult<()> {
    for payload in [json!({}), json!({"emissiveStrength": 0})] {
        for allow_semantic_loss in [false, true] {
            let mut document = material_document(json!({
                "emissiveFactor": [0, 0, 0],
                "extensions": {
                    "KHR_materials_unlit": {},
                    "KHR_materials_emissive_strength": payload
                }
            }))?;
            document["extensionsUsed"] = json!([STRENGTH, UNLIT]);
            let failure =
                reject_before_dependencies(serde_json::to_vec(&document)?, allow_semantic_loss)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn strength_is_supported_only_at_the_material_entity_site() -> TestResult<()> {
    for path in [
        "",
        "/asset",
        "/nodes/1",
        "/meshes/0/primitives/0",
        "/materials/0/pbrMetallicRoughness",
        "/materials/0/pbrMetallicRoughness/baseColorTexture",
        "/textures/0",
        "/images/0",
    ] {
        for escaped in [false, true] {
            let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
            document["extensionsUsed"] = json!([STRENGTH]);
            document.pointer_mut(path).unwrap()["extensions"] =
                json!({"KHR_materials_emissive_strength": {"emissiveStrength": 8}});
            let mut bytes = serde_json::to_string(&document)?;
            if escaped {
                bytes = bytes.replace(STRENGTH, r#"KHR_materials_emissive_str\u0065ngth"#);
            }
            let failure = reject_before_dependencies(bytes.into_bytes(), true)?;
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        }
    }
    Ok(())
}

#[test]
fn unknown_members_and_nested_extension_payloads_remain_hard_errors() -> TestResult<()> {
    for payload in [
        json!({"unexpected": null}),
        json!({"extensions": {"KHR_materials_emissive_strength": {}}}),
        json!({"extensions": {"KHR_materials_unlit": {}}}),
        json!({"extensions": {"VENDOR_nested": {}}}),
    ] {
        let document = strength_document(payload)?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    Ok(())
}

#[test]
fn duplicate_escaped_strength_keys_are_rejected_before_dependencies() -> TestResult<()> {
    for material in [
        r#"{"extensions":{"KHR_materials_emissive_strength":{},"KHR_materials_emissive_str\u0065ngth":{}}}"#,
        r#"{"extensions":{"KHR_materials_emissive_strength":{"emissiveStrength":1,"emissiveStr\u0065ngth":2}}}"#,
    ] {
        let failure = reject_before_dependencies(raw_material_document(material)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn omitted_invalid_strength_does_not_load_its_material_resources() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!({"emissiveStrength": null}),
        json!({"emissiveStrength": -1}),
        json!({"emissiveStrength": 1e40}),
        json!({"extensions": {"VENDOR_nested": {}}}),
    ] {
        let mut document = material_document(json!({}))?;
        document["extensionsUsed"] = json!([STRENGTH, UNLIT]);
        document["images"]
            .as_array_mut()
            .unwrap()
            .push(json!({"uri": "not-selected.png"}));
        document["textures"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source": 1}));
        document["materials"].as_array_mut().unwrap().push(json!({
            "extensions": {
                "KHR_materials_emissive_strength": payload,
                "KHR_materials_unlit": {}
            },
            "emissiveTexture": {"index": 1}
        }));
        let report = import_document(serde_json::to_vec(&document)?)?;
        let scene = report.scene();
        assert_eq!(scene.material_count(), 1);
        assert_eq!(
            scene
                .materials()
                .next()
                .unwrap()
                .factors()
                .emission()
                .strength(),
            None
        );
        assert_eq!((scene.texture_count(), scene.image_count()), (0, 0));
        let record = report.source_scope_record().unwrap();
        assert_eq!(record.counts(SourceScopeEntityKind::Material).selected(), 1);
        assert_eq!(record.counts(SourceScopeEntityKind::Material).omitted(), 1);
        assert_eq!(record.counts(SourceScopeEntityKind::Texture).omitted(), 2);
        assert!(
            !report
                .successful_input_manifest()
                .node_classes()
                .iter()
                .any(|class| { class.identity().value().as_str().contains("not-selected") })
        );
    }
    Ok(())
}

#[test]
fn implicit_default_has_absent_strength_and_is_distinct_from_authored_empty_extension()
-> TestResult<()> {
    let mut document = strength_document(json!({}))?;
    document["materials"][0]
        .as_object_mut()
        .unwrap()
        .remove("emissiveFactor");
    let mut implicit = document["meshes"][0]["primitives"][0].clone();
    implicit.as_object_mut().unwrap().remove("material");
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(implicit);
    let report = import_document(serde_json::to_vec(&document)?)?;
    let scene = report.scene();
    let primitives = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .collect::<Vec<_>>();
    let authored = primitives[0].authored_material().unwrap();
    let generated = primitives[1].effective_material().unwrap();
    assert_eq!(scene.material_count(), 2);
    assert_ne!(authored.id(), generated.id());
    assert!(matches!(
        primitives[1].material_assignment(),
        MaterialAssignment::ImplicitDefault(_)
    ));
    assert_eq!(authored.factors().emission().strength(), Some(1.0));
    assert_eq!(generated.factors().emission().strength(), None);
    assert_eq!(authored.factors().emission().effective_rgb(), [0.0; 3]);
    assert_eq!(generated.factors().emission().effective_rgb(), [0.0; 3]);
    assert_eq!(
        authored.factors().pbr_metallic_roughness(),
        generated.factors().pbr_metallic_roughness()
    );
    Ok(())
}

#[test]
fn escaped_long_keys_keep_exact_extras_and_material_source_ranges_in_gltf_and_glb() -> TestResult<()>
{
    assert_eq!(STRENGTH.len(), 31);
    for raw in [
        "null",
        "{}",
        "[]",
        r#"{ "label" : "\u0061", "value": 1.2300e+04, "extensions":{"VENDOR_data":{}} }"#,
    ] {
        let material = format!(
            r#"{{"name":"Emission","emissiveFactor":[0.25,0.5,1],"extras":{{"parent":true}},"exte\u006esions":{{"KHR_materials_emissive_str\u0065ngth":{{"emissiveStr\u0065ngth":8,"extensions":{{}},"extr\u0061s":{raw}}}}}}}"#
        );
        let json = raw_material_document(&material)?;
        let offset = std::str::from_utf8(&json)?.find(&material).unwrap();
        for is_glb in [false, true] {
            let report = import_document(if is_glb {
                glb_with_json(&json)
            } else {
                json.clone()
            })?;
            let scene = report.scene();
            let retained = scene.materials().next().unwrap();
            assert_eq!(retained.factors().emission().strength(), Some(8.0));
            let sources = scene
                .source_identities()
                .filter(|source| {
                    matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with("kind=material;"))
                })
                .collect::<Vec<_>>();
            assert_eq!(sources.len(), 1);
            let source = sources[0];
            assert_eq!(source.resource().identity(), SOURCE);
            let locator = format!(
                "kind=material;parent=-;index=0;offset={};length={}",
                offset + if is_glb { 20 } else { 0 },
                material.len(),
            );
            assert_eq!(
                source.locator(),
                SourceObjectLocatorRef::Namespaced {
                    schema: "org.baozi.gltf:source-object",
                    schema_version: 1,
                    value: &locator,
                }
            );
            let entries = source.metadata_entries().collect::<Vec<_>>();
            assert_eq!(entries.len(), 2);
            for (key, expected) in [
                ("org.baozi.gltf:extras", r#"{"parent":true}"#),
                ("org.baozi.gltf:emissive-strength-extras", raw),
            ] {
                let entry = entries.iter().find(|entry| entry.key() == key).unwrap();
                assert_eq!(entry.owner(), MetadataOwnerRef::SourceObject(source));
                let MetadataValueRef::Bytes {
                    media_type,
                    payload,
                } = entry.value()
                else {
                    panic!("strength extras must retain exact JSON bytes");
                };
                assert_eq!(media_type, "application/json");
                assert_eq!(payload, expected.as_bytes());
            }
            assert!(scene.owner_provenance(retained.metadata_owner())?.any(|node| {
                matches!(node.kind(), ProvenanceKindRef::Source(candidate) if candidate == source)
            }));
        }
    }
    Ok(())
}

#[test]
fn misplaced_escaped_strength_reports_its_exact_payload_range_with_glb_offset() -> TestResult<()> {
    let payload = r#"{ "emissiveStrength" : 8 }"#;
    let material = format!(
        r#"{{"pbrMetallicRoughness":{{"extensions":{{"KHR_materials_emissive_str\u0065ngth":{payload}}}}}}}"#
    );
    let json = raw_material_document(&material)?;
    let offset = std::str::from_utf8(&json)?.find(payload).unwrap();
    for is_glb in [false, true] {
        let failure = reject_before_dependencies(
            if is_glb {
                glb_with_json(&json)
            } else {
                json.clone()
            },
            true,
        )?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        let envelope = gltf_failure(&failure)?;
        for (key, expected) in [
            ("byte_offset", (offset + if is_glb { 20 } else { 0 }) as u64),
            ("byte_length", payload.len() as u64),
        ] {
            assert!(
                envelope.details().iter().any(|detail| {
                    detail.key() == key
                        && matches!(detail.value(),
                    FormatFailureDetailValue::Unsigned(value) if value == expected)
                }),
                "missing {key}={expected}: {envelope:?}"
            );
        }
    }
    Ok(())
}
