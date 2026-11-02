mod common;

use baozi_core::{
    AlphaMode, AttributeDomain, AttributeElement, AttributeSemanticRef, ColorInterpretation,
    MaterialAssignment, PbrMetallicRoughnessFactors, ProvenanceKindRef, ShadingModel, TextureRole,
};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportDimension, ImportFailure, ImportLossKind, ImportReport, LossPolicy, MetadataOwnerRef,
    MetadataValueRef, SceneSnapshotLimits, SourceObjectLocatorRef, SourceScopeEntityKind,
};
use common::{
    TestResult, assert_gltf_failure, assert_resource_limit, expected_error, glb_with_json,
    gltf_failure, import_assets, import_assets_result, sidecar_options, triangle_bin,
    triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

const UNLIT: &str = "KHR_materials_unlit";
const SOURCE: &str = "models/unlit.gltf";
const BINARY: &str = "models/triangle.bin";
const IMAGE_PATHS: [&str; 5] = [
    "models/textures/base.png",
    "models/textures/metallic.png",
    "models/textures/normal.png",
    "models/textures/occlusion.png",
    "models/textures/emissive.png",
];

fn triangle_document() -> TestResult<Value> {
    Ok(serde_json::from_slice(&triangle_gltf())?)
}

fn unlit_document(payload: Value) -> TestResult<Value> {
    let mut document = triangle_document()?;
    document["extensionsUsed"] = json!([UNLIT]);
    document["materials"][0]["extensions"] = json!({"KHR_materials_unlit": payload});
    Ok(document)
}

fn raw_material_document(material: &str) -> TestResult<Vec<u8>> {
    let document = unlit_document(json!({}))?;
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
            (IMAGE_PATHS[0], triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(report.loss_evidence().is_empty());
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
        "content rejection must precede dependency IO: {failure:?}",
    );
    assert!(failure.loss_evidence().is_empty());
    Ok(failure)
}

fn fallback_document() -> TestResult<Value> {
    let mut document = unlit_document(json!({}))?;
    document["extensionsUsed"] = json!([UNLIT, "KHR_texture_transform"]);
    document["images"] = json!([
        {"uri":"textures/base.png"},
        {"uri":"textures/metallic.png"},
        {"uri":"textures/normal.png"},
        {"uri":"textures/occlusion.png"},
        {"uri":"textures/emissive.png"},
        {"uri":"textures/not-selected.png"}
    ]);
    document["textures"] = (0..6)
        .map(|index| json!({"source":index, "sampler":0}))
        .collect::<Vec<_>>()
        .into();
    let material = &mut document["materials"][0];
    material["pbrMetallicRoughness"]["metallicRoughnessTexture"] = json!({"index":1, "texCoord":1});
    material["normalTexture"] = json!({"index":2, "texCoord":2, "scale":0.4});
    material["occlusionTexture"] = json!({"index":3, "texCoord":3, "strength":0.625});
    material["emissiveTexture"] = json!({"index":4, "texCoord":4});
    material["emissiveFactor"] = json!([0.2, 0.4, 0.6]);
    for path in [
        "/pbrMetallicRoughness/baseColorTexture",
        "/pbrMetallicRoughness/metallicRoughnessTexture",
        "/normalTexture",
        "/occlusionTexture",
        "/emissiveTexture",
    ] {
        material.pointer_mut(path).unwrap()["extensions"] = json!({
            "KHR_texture_transform": {
                "offset":[0.125, 0.25], "rotation":-0.375, "scale":[2.0, 0.5]
            }
        });
    }
    Ok(document)
}

#[test]
fn optional_and_required_unlit_objects_preserve_default_factors() -> TestResult<()> {
    for required in [false, true] {
        for payload in [json!({}), json!({"extensions":{}})] {
            let mut document = unlit_document(payload)?;
            let extension = document["materials"][0]["extensions"].clone();
            document["materials"][0] = json!({"extensions":extension});
            if required {
                document["extensionsRequired"] = json!([UNLIT]);
            }
            let report = import_document(serde_json::to_vec(&document)?)?;
            let material = report.scene().materials().next().unwrap();

            assert_eq!(material.shading_model(), ShadingModel::Unlit);
            assert_eq!(material.alpha_mode(), AlphaMode::Opaque);
            assert!(!material.double_sided());
            assert_eq!(material.binding_count(), 0);
            let factors = material.factors();
            assert_eq!(
                factors.pbr_metallic_roughness(),
                Some(PbrMetallicRoughnessFactors::default()),
            );
            let emissive = factors.emission().factor();
            assert_eq!([emissive.x, emissive.y, emissive.z], [0.0; 3]);
            assert!(factors.pbr_specular_glossiness().is_none());
            assert!(factors.legacy().is_none());
        }
    }
    Ok(())
}

#[test]
fn declarations_empty_extensions_and_descriptive_extras_do_not_select_unlit() -> TestResult<()> {
    for declaration in ["absent", "optional", "required"] {
        let mut document = triangle_document()?;
        document["materials"][0] = json!({
            "extensions":{}, "extras":{"KHR_materials_unlit":{}}
        });
        if declaration != "absent" {
            document["extensionsUsed"] = json!([UNLIT]);
        }
        if declaration == "required" {
            document["extensionsRequired"] = json!([UNLIT]);
        }
        let report = import_document(serde_json::to_vec(&document)?)?;
        let material = report.scene().materials().next().unwrap();
        assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
        assert_eq!(
            material.factors().pbr_metallic_roughness(),
            Some(PbrMetallicRoughnessFactors::default()),
        );
    }
    Ok(())
}

#[test]
fn mixed_lit_unlit_and_implicit_material_primitives_keep_their_links() -> TestResult<()> {
    let mut document = triangle_document()?;
    document["extensionsUsed"] = json!([UNLIT]);
    let mut unlit = document["materials"][0].clone();
    unlit["name"] = json!("Unlit");
    unlit["extensions"] = json!({"KHR_materials_unlit":{}});
    document["materials"].as_array_mut().unwrap().push(unlit);
    let primitive = document["meshes"][0]["primitives"][0].clone();
    let mut unlit_primitive = primitive.clone();
    unlit_primitive["material"] = json!(1);
    let mut implicit_primitive = primitive.clone();
    implicit_primitive
        .as_object_mut()
        .unwrap()
        .remove("material");
    document["meshes"][0]["primitives"] = json!([primitive, unlit_primitive, implicit_primitive]);

    let report = import_document(serde_json::to_vec(&document)?)?;
    let scene = report.scene();
    let materials = scene.materials().collect::<Vec<_>>();
    assert_eq!(materials.len(), 3);
    assert_eq!(materials[0].name(), Some("Red"));
    assert_eq!(
        materials[0].shading_model(),
        ShadingModel::PbrMetallicRoughness
    );
    assert_eq!(materials[1].name(), Some("Unlit"));
    assert_eq!(materials[1].shading_model(), ShadingModel::Unlit);
    assert_eq!(materials[0].factors(), materials[1].factors());
    let primitives = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .collect::<Vec<_>>();
    assert_eq!(primitives.len(), 3);
    assert_eq!(
        primitives[0].authored_material().unwrap().id(),
        materials[0].id()
    );
    assert_eq!(
        primitives[1].authored_material().unwrap().id(),
        materials[1].id()
    );
    assert!(primitives[2].authored_material().is_none());
    assert!(
        matches!(primitives[2].material_assignment(), MaterialAssignment::ImplicitDefault(material) if material.id() == materials[2].id())
    );
    assert_eq!(
        materials[2].shading_model(),
        ShadingModel::PbrMetallicRoughness
    );
    assert_eq!(
        report
            .source_scope_record()
            .unwrap()
            .counts(SourceScopeEntityKind::Material)
            .selected(),
        2,
    );
    Ok(())
}

#[test]
fn unlit_preserves_alpha_double_sided_and_vertex_color_semantics() -> TestResult<()> {
    let colors = [
        [0.25_f32, 0.5, 0.75, 0.875],
        [1.0, 0.25, 0.5, 0.5],
        [0.0, 1.0, 0.5, 1.0],
    ];
    for (mode, cutoff, double_sided, expected) in [
        ("OPAQUE", None, false, AlphaMode::Opaque),
        ("MASK", None, true, AlphaMode::mask(0.5)?),
        ("MASK", Some(0.375), false, AlphaMode::mask(0.375)?),
        ("BLEND", None, true, AlphaMode::Blend),
    ] {
        let mut document = unlit_document(json!({}))?;
        let material = &mut document["materials"][0];
        material["alphaMode"] = json!(mode);
        material["doubleSided"] = json!(double_sided);
        if let Some(cutoff) = cutoff {
            material["alphaCutoff"] = json!(cutoff);
        }
        let mut binary = triangle_bin();
        let color_offset = binary.len();
        binary.extend(
            colors
                .iter()
                .flatten()
                .flat_map(|value| value.to_le_bytes()),
        );
        document["buffers"][0]["byteLength"] = json!(binary.len());
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer":0, "byteOffset":color_offset, "byteLength":48, "target":34962
        }));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView":4, "componentType":5126, "count":3, "type":"VEC4"
        }));
        document["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4);
        let (report, diagnostics) = import_assets(
            SOURCE,
            [
                (SOURCE, serde_json::to_vec(&document)?),
                (BINARY, binary),
                (IMAGE_PATHS[0], triangle_png()),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        assert!(report.loss_evidence().is_empty());
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let material = primitive.authored_material().unwrap();
        assert_eq!(material.shading_model(), ShadingModel::Unlit);
        assert_eq!(material.alpha_mode(), expected);
        assert_eq!(material.double_sided(), double_sided);
        let base = material
            .factors()
            .pbr_metallic_roughness()
            .unwrap()
            .base_color();
        assert_eq!([base.r, base.g, base.b, base.a], [0.8, 0.1, 0.2, 0.7]);
        let color = primitive
            .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex)
            .unwrap();
        assert_eq!(color.len(), 3);
        assert_eq!(color.lanes(), 4);
        for (index, expected) in colors.iter().enumerate() {
            let Some(AttributeElement::F32(actual)) = color.element(index) else {
                panic!("vertex color must remain an F32 attribute");
            };
            assert_eq!(actual, expected);
        }
        assert!(primitive.normals(AttributeDomain::Vertex).is_some());
        assert!(primitive.texcoords(0, AttributeDomain::Vertex).is_some());
    }
    Ok(())
}

#[test]
fn unlit_retains_all_pbr_factors_bindings_and_selected_fallback_resources() -> TestResult<()> {
    let mut reports = Vec::new();
    for unlit in [false, true] {
        let mut document = fallback_document()?;
        if !unlit {
            document["materials"][0]
                .as_object_mut()
                .unwrap()
                .remove("extensions");
        }
        // The sixth image has no resource in the fixture and must stay outside the closure.
        let assets = [
            (SOURCE, serde_json::to_vec(&document)?),
            (BINARY, triangle_bin()),
        ]
        .into_iter()
        .chain(IMAGE_PATHS.into_iter().map(|path| (path, triangle_png())));
        let (report, diagnostics) = import_assets(SOURCE, assets, sidecar_options())?;
        assert!(diagnostics.is_empty());
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        let material = scene.materials().next().unwrap();
        assert_eq!(material.binding_count(), 5);
        assert_eq!(scene.texture_count(), 5);
        assert_eq!(scene.image_count(), 5);
        assert_eq!(scene.sampler_count(), 1);
        let pbr = material.factors().pbr_metallic_roughness().unwrap();
        let color = pbr.base_color();
        assert_eq!(
            [
                color.r,
                color.g,
                color.b,
                color.a,
                pbr.metallic(),
                pbr.roughness(),
            ]
            .map(f32::to_bits),
            [0.8_f32, 0.1, 0.2, 0.7, 0.5, 0.25].map(f32::to_bits),
        );
        let emissive = material.factors().emission().factor();
        assert_eq!([emissive.x, emissive.y, emissive.z], [0.2, 0.4, 0.6]);
        let expected_bindings = [
            (
                TextureRole::BaseColor,
                "baseColorTexture",
                1.0_f32,
                ColorInterpretation::SrgbColor,
            ),
            (
                TextureRole::MetallicRoughness,
                "metallicRoughnessTexture",
                1.0,
                ColorInterpretation::Data,
            ),
            (
                TextureRole::Normal,
                "normalTexture",
                0.4,
                ColorInterpretation::Data,
            ),
            (
                TextureRole::Occlusion,
                "occlusionTexture",
                0.625,
                ColorInterpretation::Data,
            ),
            (
                TextureRole::Emissive,
                "emissiveTexture",
                1.0,
                ColorInterpretation::SrgbColor,
            ),
        ];
        for (index, (binding, (role, key, strength, interpretation))) in
            material.bindings().zip(expected_bindings).enumerate()
        {
            assert_eq!(binding.role(), role);
            assert_eq!(binding.source_key(), Some(key));
            assert_eq!(usize::from(binding.uv_set()), index);
            assert_eq!(binding.strength().get().to_bits(), strength.to_bits());
            assert_eq!(binding.color_interpretation(), interpretation);
            assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
            let transform = binding.transform();
            assert_eq!(
                [
                    transform.offset().x,
                    transform.offset().y,
                    transform.rotation_radians(),
                    transform.scale().x,
                    transform.scale().y,
                ],
                [0.125, 0.25, -0.375, 2.0, 0.5],
            );
        }
        let record = report.source_scope_record().unwrap();
        for kind in [SourceScopeEntityKind::Texture, SourceScopeEntityKind::Image] {
            assert_eq!(record.counts(kind).selected(), 5);
            assert_eq!(record.counts(kind).omitted(), 1);
        }
        let mut resources = report
            .successful_input_manifest()
            .node_classes()
            .iter()
            .map(|class| class.identity().value().as_str())
            .collect::<Vec<_>>();
        resources.sort_unstable();
        let mut expected = vec![SOURCE, BINARY];
        expected.extend(IMAGE_PATHS);
        expected.sort_unstable();
        assert_eq!(resources, expected);
        reports.push(report);
    }
    let lit = reports[0].scene().materials().next().unwrap();
    let unlit = reports[1].scene().materials().next().unwrap();
    assert_eq!(lit.shading_model(), ShadingModel::PbrMetallicRoughness);
    assert_eq!(unlit.shading_model(), ShadingModel::Unlit);
    assert_eq!(lit.factors(), unlit.factors());
    assert_eq!(lit.alpha_mode(), unlit.alpha_mode());
    assert_eq!(lit.double_sided(), unlit.double_sided());
    for (lit, unlit) in lit.bindings().zip(unlit.bindings()) {
        assert_eq!(lit.channel_mapping(), unlit.channel_mapping());
        assert_eq!(lit.blend(), unlit.blend());
        assert_eq!(
            lit.texture().sampler().state(),
            unlit.texture().sampler().state(),
        );
    }
    Ok(())
}

#[test]
fn missing_selected_fallback_normal_image_still_fails_for_unlit() -> TestResult<()> {
    let document = fallback_document()?;
    let assets = [
        (SOURCE, serde_json::to_vec(&document)?),
        (BINARY, triangle_bin()),
    ]
    .into_iter()
    .chain(
        IMAGE_PATHS
            .into_iter()
            .filter(|path| *path != IMAGE_PATHS[2])
            .map(|path| (path, triangle_png())),
    );
    let (result, _) = import_assets_result(SOURCE, assets, sidecar_options())?;
    let failure = expected_error(result)?;
    let dependency = failure
        .cause()
        .dependency_context()
        .expect("a selected fallback image remains a required dependency");
    assert_eq!(dependency.requested_reference(), "textures/normal.png");
    assert_eq!(dependency.origin().path().as_str(), SOURCE);
    assert_eq!(dependency.role().as_str(), "external-image");
    assert!(failure.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn unlit_payload_and_required_declarations_need_extensions_used_before_io() -> TestResult<()> {
    for (payload, required) in [(true, false), (true, true), (false, true)] {
        let mut document = unlit_document(json!({}))?;
        document.as_object_mut().unwrap().remove("extensionsUsed");
        if !payload {
            document["materials"][0]
                .as_object_mut()
                .unwrap()
                .remove("extensions");
        }
        if required {
            document["extensionsRequired"] = json!([UNLIT]);
        }
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn duplicate_unlit_declarations_fail_before_dependency_io() -> TestResult<()> {
    for field in ["extensionsUsed", "extensionsRequired"] {
        let mut document = unlit_document(json!({}))?;
        document[field] = json!([UNLIT, UNLIT]);
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn malformed_selected_unlit_payloads_fail_before_dependency_io() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!([{}]),
        json!(true),
        json!(false),
        json!(0),
        json!("unlit"),
    ] {
        let document = unlit_document(payload)?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn undefined_unlit_fields_and_nonempty_nested_extensions_are_hard_unsupported() -> TestResult<()> {
    for payload in [
        json!({"unexpected":null}),
        json!({"baseColorFactor":[1, 1, 1, 1]}),
        json!({"extensions":{"VENDOR_nested":{}}}),
        json!({"extensions":{"KHR_materials_unlit":{}}}),
        json!({"extensions":{"KHR_texture_transform":{}}}),
    ] {
        for allow_semantic_loss in [false, true] {
            let document = unlit_document(payload.clone())?;
            let failure =
                reject_before_dependencies(serde_json::to_vec(&document)?, allow_semantic_loss)?;
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        }
    }
    Ok(())
}

#[test]
fn nested_unlit_extensions_must_be_objects_before_dependency_io() -> TestResult<()> {
    for nested in [
        Value::Null,
        json!([]),
        json!(true),
        json!(0),
        json!("extensions"),
    ] {
        let document = unlit_document(json!({"extensions":nested}))?;
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        let code = gltf_failure(&failure)?.code();
        assert!(
            [
                GltfFailureCode::InvalidDocument.as_str(),
                GltfFailureCode::UnsupportedFeature.as_str(),
            ]
            .contains(&code),
            "{failure:?}",
        );
    }
    Ok(())
}

#[test]
fn unlit_at_wrong_owners_and_material_sites_is_hard_unsupported() -> TestResult<()> {
    for path in [
        "",
        "/asset",
        "/scenes/0",
        "/nodes/1",
        "/meshes/0",
        "/meshes/0/primitives/0",
        "/materials/0/pbrMetallicRoughness",
        "/materials/0/pbrMetallicRoughness/baseColorTexture",
        "/textures/0",
        "/images/0",
        "/samplers/0",
        "/accessors/0",
        "/bufferViews/0",
        "/buffers/0",
    ] {
        let mut document = triangle_document()?;
        document["extensionsUsed"] = json!([UNLIT]);
        document.pointer_mut(path).unwrap()["extensions"] = json!({"KHR_materials_unlit":{}});
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    Ok(())
}

#[test]
fn optional_unknown_selected_payloads_are_hard_errors_under_both_loss_policies() -> TestResult<()> {
    for path in ["", "/materials/0"] {
        for allow_semantic_loss in [false, true] {
            let mut document = unlit_document(json!({}))?;
            document["extensionsUsed"] = json!([UNLIT, "VENDOR_unknown"]);
            document.pointer_mut(path).unwrap()["extensions"]["VENDOR_unknown"] = json!({});
            let failure =
                reject_before_dependencies(serde_json::to_vec(&document)?, allow_semantic_loss)?;
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        }
    }
    Ok(())
}

#[test]
fn unknown_required_extensions_stay_hard_errors_with_or_without_payloads() -> TestResult<()> {
    for payload in [false, true] {
        for allow_semantic_loss in [false, true] {
            let mut document = unlit_document(json!({}))?;
            document["extensionsUsed"] = json!([UNLIT, "VENDOR_unknown"]);
            document["extensionsRequired"] = json!(["VENDOR_unknown"]);
            if payload {
                document["materials"][0]["extensions"]["VENDOR_unknown"] = json!({});
            }
            let failure =
                reject_before_dependencies(serde_json::to_vec(&document)?, allow_semantic_loss)?;
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        }
    }
    Ok(())
}

#[test]
fn unsupported_declarations_without_retained_payloads_only_emit_a_diagnostic() -> TestResult<()> {
    for omitted_payload in [false, true] {
        for allow_semantic_loss in [false, true] {
            let mut document = unlit_document(json!({}))?;
            document["extensionsUsed"] = json!([UNLIT, "VENDOR_unknown"]);
            if omitted_payload {
                document["materials"].as_array_mut().unwrap().push(json!({
                    "extensions":{"VENDOR_unknown":{}}
                }));
            }
            let mut options = sidecar_options();
            if allow_semantic_loss {
                options.loss_policy =
                    LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss);
            }
            let (report, diagnostics) = import_assets(
                SOURCE,
                [
                    (SOURCE, serde_json::to_vec(&document)?),
                    (BINARY, triangle_bin()),
                    (IMAGE_PATHS[0], triangle_png()),
                ],
                options,
            )?;
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert_eq!(
                diagnostics[0].code().as_str(),
                "gltf.extension_uninterpreted_declaration",
            );
            assert!(diagnostics[0].message().contains("VENDOR_unknown"));
            assert!(report.loss_evidence().is_empty());
            assert_eq!(
                report.scene().materials().next().unwrap().shading_model(),
                ShadingModel::Unlit,
            );
        }
    }
    Ok(())
}

#[test]
fn omitted_invalid_unlit_payloads_do_not_load_their_material_resource_closure() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!({"unexpected":true}),
        json!({"extensions":{"VENDOR_nested":{}}}),
    ] {
        let mut document = triangle_document()?;
        document["extensionsUsed"] = json!([UNLIT]);
        document["scenes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"nodes":[2]}));
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"mesh":1}));
        document["buffers"].as_array_mut().unwrap().push(json!({
            "uri":"not-selected.bin", "byteLength":36
        }));
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer":1, "byteLength":36
        }));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView":4, "componentType":5126, "count":3, "type":"VEC3",
            "min":[0,0,0], "max":[1,1,0]
        }));
        document["images"]
            .as_array_mut()
            .unwrap()
            .push(json!({"uri":"textures/not-selected.png"}));
        document["textures"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":1, "sampler":0}));
        document["materials"].as_array_mut().unwrap().push(json!({
            "extensions":{"KHR_materials_unlit":payload},
            "pbrMetallicRoughness":{"baseColorTexture":{"index":1}},
            "normalTexture":{"index":1}, "emissiveTexture":{"index":1}
        }));
        document["meshes"].as_array_mut().unwrap().push(json!({
            "primitives":[{"attributes":{"POSITION":4}, "material":1}]
        }));
        let report = import_document(serde_json::to_vec(&document)?)?;
        assert_eq!(
            report.scene().materials().next().unwrap().shading_model(),
            ShadingModel::PbrMetallicRoughness,
        );
        let record = report.source_scope_record().unwrap();
        for kind in [
            SourceScopeEntityKind::Mesh,
            SourceScopeEntityKind::Material,
            SourceScopeEntityKind::Texture,
            SourceScopeEntityKind::Image,
            SourceScopeEntityKind::Buffer,
        ] {
            assert_eq!(record.counts(kind).selected(), 1);
            assert_eq!(record.counts(kind).omitted(), 1);
        }
        let manifest = report.successful_input_manifest();
        assert_eq!(manifest.counts().node_class_count(), 3);
        assert!(
            manifest
                .node_classes()
                .iter()
                .all(|class| { !class.identity().value().as_str().contains("not-selected") })
        );
        assert!(report.scene().source_identities().all(|source| {
            !matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                if value.starts_with("kind=material;parent=-;index=1;"))
        }));
    }
    Ok(())
}

#[test]
fn escaped_unlit_keys_preserve_exact_extras_on_the_existing_material_source() -> TestResult<()> {
    for raw in [
        "null",
        "{}",
        "[]",
        r#"{ "label" : "\u0061", "value": 1.2300e+04, "extensions":{"VENDOR_data":{}} }"#,
    ] {
        let material = format!(
            r#"{{"name":"材质","extras":{{"parent":true}},"pbrMetallicRoughness":{{"extras":[1,2]}},"exte\u006esions":{{"KHR_materials_unl\u0069t":{{"extensions":{{}},"extr\u0061s":{raw}}}}}}}"#
        );
        let json = raw_material_document(&material)?;
        let offset = std::str::from_utf8(&json)?.find(&material).unwrap();
        let material_length = material.len();
        for is_glb in [false, true] {
            let document = if is_glb {
                glb_with_json(&json)
            } else {
                json.clone()
            };
            let report = import_document(document)?;
            let scene = report.scene();
            let material = scene.materials().next().unwrap();
            assert_eq!(material.shading_model(), ShadingModel::Unlit);
            let sources = scene
                .source_identities()
                .filter(|source| {
                    matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                        if value.starts_with("kind=material;"))
                })
                .collect::<Vec<_>>();
            assert_eq!(sources.len(), 1);
            let source = sources[0];
            assert_eq!(source.generation(), scene.generation());
            assert_eq!(source.format(), "org.baozi.gltf");
            assert_eq!(source.resource().identity(), SOURCE);
            let expected_locator = format!(
                "kind=material;parent=-;index=0;offset={};length={}",
                offset + if is_glb { 20 } else { 0 },
                material_length,
            );
            assert_eq!(
                source.locator(),
                SourceObjectLocatorRef::Namespaced {
                    schema: "org.baozi.gltf:source-object",
                    schema_version: 1,
                    value: &expected_locator,
                },
            );
            let metadata = source.metadata_entries().collect::<Vec<_>>();
            assert_eq!(metadata.len(), 3);
            for (key, raw) in [
                ("org.baozi.gltf:extras", r#"{"parent":true}"#),
                ("org.baozi.gltf:pbr-metallic-roughness-extras", "[1,2]"),
                ("org.baozi.gltf:unlit-extras", raw),
            ] {
                let entry = metadata.iter().find(|entry| entry.key() == key).unwrap();
                assert_eq!(entry.owner(), MetadataOwnerRef::SourceObject(source));
                let MetadataValueRef::Bytes {
                    media_type,
                    payload,
                } = entry.value()
                else {
                    panic!("unlit extras must retain their exact source JSON bytes");
                };
                assert_eq!(media_type, "application/json");
                assert_eq!(payload, raw.as_bytes());
            }
            assert_eq!(scene.metadata_count(), 3);
            let lineage = scene
                .owner_provenance(material.metadata_owner())?
                .collect::<Vec<_>>();
            let leaf = lineage
                .iter()
                .find(|node| {
                    matches!(node.kind(), ProvenanceKindRef::Source(candidate) if candidate == source)
                })
                .expect("unlit material must keep its existing source leaf");
            assert_eq!(leaf.depth(), 1);
            assert_eq!(leaf.parents().len(), 0);
            assert_eq!(scene.provenance(leaf.id())?, *leaf);
            let snapshot =
                scene.capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 256, 1_000_000)?)?;
            assert_eq!(
                snapshot.profile_source_count(),
                scene.source_identity_count()
            );
            assert_eq!(snapshot.profile_operation_count(), 1);
            assert_eq!(
                snapshot.profile_provenance_node_count(),
                snapshot.profile_source_count() + 1,
            );
        }
    }
    Ok(())
}

#[test]
fn unlit_extras_key_limit_fails_before_scope_publication_and_dependency_io() -> TestResult<()> {
    const KEY: &str = "org.baozi.gltf:unlit-extras";
    let document = serde_json::to_vec(&unlit_document(json!({"extras":null}))?)?;
    let control = import_document(document.clone())?;
    let material = control.scene().materials().next().unwrap();
    assert_eq!(material.shading_model(), ShadingModel::Unlit);
    let entries = control
        .scene()
        .metadata_entries()
        .filter(|entry| entry.key() == KEY)
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    let entry = entries[0];
    let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
        panic!("unlit extras must belong to the existing material source object");
    };
    assert_eq!(source.format(), "org.baozi.gltf");
    assert_eq!(source.resource().identity(), SOURCE);
    assert!(matches!(
        source.locator(),
        SourceObjectLocatorRef::Namespaced { value, .. }
            if value.starts_with("kind=material;parent=-;index=0;")
    ));
    let MetadataValueRef::Bytes {
        media_type,
        payload,
    } = entry.value()
    else {
        panic!("unlit extras must retain their exact source JSON bytes");
    };
    assert_eq!(media_type, "application/json");
    assert_eq!(payload, b"null");

    let mut options = sidecar_options();
    options.limits.max_string_bytes = KEY.len() - 1;
    // Every resource path and authored fixture name fits below the rejected extras key.
    assert!(
        [SOURCE, BINARY, IMAGE_PATHS[0]]
            .iter()
            .all(|path| path.len() <= options.limits.max_string_bytes)
    );
    let (result, _) = import_assets_result(SOURCE, [(SOURCE, document)], options)?;
    let failure = expected_error(result)?;
    assert_resource_limit(&failure, "max_string_bytes");
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0,
    );
    assert!(failure.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn unlit_flag_changes_snapshot_digest_with_identical_source_ranges_and_factors() -> TestResult<()> {
    let unlit_material = r#"{"extensions":{"KHR_materials_unlit":{}}}"#;
    let mut lit_material = r#"{"extensions":{}}"#.to_owned();
    let padding = " ".repeat(unlit_material.len() - lit_material.len());
    lit_material.insert_str(lit_material.len() - 1, &padding);
    let lit_document = raw_material_document(&lit_material)?;
    let unlit_document = raw_material_document(unlit_material)?;
    assert_eq!(lit_document.len(), unlit_document.len());
    let lit = import_document(lit_document)?;
    let unlit = import_document(unlit_document)?;
    let lit_scene = lit.scene();
    let unlit_scene = unlit.scene();
    let lit_material = lit_scene.materials().next().unwrap();
    let unlit_material = unlit_scene.materials().next().unwrap();
    assert_eq!(
        lit_material.shading_model(),
        ShadingModel::PbrMetallicRoughness
    );
    assert_eq!(unlit_material.shading_model(), ShadingModel::Unlit);
    assert_eq!(lit_material.factors(), unlit_material.factors());
    assert_eq!(lit_material.binding_count(), unlit_material.binding_count());
    assert_eq!(lit_material.alpha_mode(), unlit_material.alpha_mode());
    assert_eq!(lit_material.double_sided(), unlit_material.double_sided());
    assert_eq!(
        lit_scene.source_identity_count(),
        unlit_scene.source_identity_count(),
    );
    assert_eq!(
        lit_scene.provenance_node_count(),
        unlit_scene.provenance_node_count(),
    );
    for (lit, unlit) in lit_scene
        .source_identities()
        .zip(unlit_scene.source_identities())
    {
        assert_eq!(lit.resource().identity(), unlit.resource().identity());
        assert_eq!(lit.format(), unlit.format());
        assert_eq!(lit.locator(), unlit.locator());
        assert_eq!(lit.display_name(), unlit.display_name());
        assert_eq!(lit.content_digest(), unlit.content_digest());
    }
    let limits = SceneSnapshotLimits::try_new(64 * 1024, 256, 1_000_000)?;
    let lit_snapshot = lit_scene.capture_snapshot(limits)?;
    let unlit_snapshot = unlit_scene.capture_snapshot(limits)?;
    assert_eq!(
        lit_snapshot.profile_source_count(),
        unlit_snapshot.profile_source_count(),
    );
    assert_eq!(
        lit_snapshot.profile_operation_count(),
        unlit_snapshot.profile_operation_count(),
    );
    assert_ne!(lit_snapshot.digest(), unlit_snapshot.digest());
    Ok(())
}
