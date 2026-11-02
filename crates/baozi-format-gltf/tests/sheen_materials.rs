mod common;

use baozi_core::{
    ColorInterpretation, MetadataOwnerRef, MetadataValueRef, ProvenanceKindRef,
    SceneSnapshotLimits, SourceObjectLocatorRef, TextureBlendOperation, TextureChannel,
    TextureRole,
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

const SHEEN: &str = "KHR_materials_sheen";
const CLEARCOAT: &str = "KHR_materials_clearcoat";
const TRANSFORM: &str = "KHR_texture_transform";
const SOURCE: &str = "models/sheen.gltf";
const BINARY: &str = "models/triangle.bin";
const IMAGE: &str = "models/textures/base.png";

fn material_document(material: Value) -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!([SHEEN]);
    document["materials"][0] = material;
    Ok(document)
}

fn sheen_document(payload: Value) -> TestResult<Value> {
    material_document(json!({"extensions": {"KHR_materials_sheen": payload}}))
}

fn raw_material_document(material: &str) -> TestResult<Vec<u8>> {
    let mut document = material_document(json!({"name": "sheen source placeholder"}))?;
    document["extensionsUsed"] = json!([SHEEN, TRANSFORM]);
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
        "sheen rejection must precede sidecar IO: {failure:?}",
    );
    assert!(failure.loss_evidence().is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(failure)
}

#[test]
fn absent_empty_and_explicit_zero_preserve_sheen_presence_and_raw_bits() -> TestResult<()> {
    for (payload, expected) in [
        (None, None),
        (Some(json!({})), Some([0.0_f32; 4])),
        (
            Some(json!({"sheenColorFactor": [-0.0, 0.0, -0.0], "sheenRoughnessFactor": -0.0})),
            Some([-0.0, 0.0, -0.0, -0.0]),
        ),
        (
            Some(
                json!({"sheenColorFactor": [1.0, 0.25, f32::from_bits(1)], "sheenRoughnessFactor": 0.625}),
            ),
            Some([1.0, 0.25, f32::from_bits(1), 0.625]),
        ),
    ] {
        let document = if let Some(payload) = payload {
            sheen_document(payload)?
        } else {
            material_document(json!({}))?
        };
        let report = import_document(serde_json::to_vec(&document)?)?;
        let material = report.scene().materials().next().unwrap();
        let actual = material.factors().sheen().map(|sheen| {
            let color = sheen.color();
            [color.x, color.y, color.z, sheen.roughness()].map(f32::to_bits)
        });
        assert_eq!(actual, expected.map(|values| values.map(f32::to_bits)));
        assert!(material.factors().clearcoat().is_none());
        assert_eq!(material.binding_count(), 0);
        assert_eq!(
            (report.scene().texture_count(), report.scene().image_count()),
            (0, 0)
        );
    }
    Ok(())
}

#[test]
fn zero_sheen_keeps_color_rgb_srgb_and_roughness_alpha_data_bindings() -> TestResult<()> {
    let mut document = sheen_document(json!({
        "sheenColorTexture": {"index": 0, "texCoord": 7},
        "sheenRoughnessTexture": {"index": 0, "texCoord": 9}
    }))?;
    document["extensionsRequired"] = json!([SHEEN]);
    let report = import_document(serde_json::to_vec(&document)?)?;
    let material = report.scene().materials().next().unwrap();
    let sheen = material.factors().sheen().unwrap();
    let color = sheen.color();
    assert_eq!([color.x, color.y, color.z, sheen.roughness()], [0.0; 4]);
    assert_eq!(material.binding_count(), 2);
    assert_eq!(
        (report.scene().texture_count(), report.scene().image_count()),
        (1, 1)
    );
    let expected: [(
        TextureRole,
        &str,
        &[TextureChannel],
        ColorInterpretation,
        u16,
    ); 2] = [
        (
            TextureRole::SheenColor,
            "sheenColorTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::SrgbColor,
            7,
        ),
        (
            TextureRole::SheenRoughness,
            "sheenRoughnessTexture",
            &[TextureChannel::Alpha],
            ColorInterpretation::Data,
            9,
        ),
    ];
    for (binding, (role, key, channels, transfer, uv)) in material.bindings().zip(expected) {
        assert_eq!(binding.role(), role);
        assert_eq!(binding.source_key(), Some(key));
        assert_eq!(binding.channel_mapping().as_slice(), channels);
        assert_eq!(binding.color_interpretation(), transfer);
        assert_eq!(binding.uv_set(), uv);
        assert_eq!(binding.strength().get().to_bits(), 1.0_f32.to_bits());
        assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
    }
    Ok(())
}

#[test]
fn sparse_selected_material_ranges_preserve_all_ten_slots_and_shared_resources() -> TestResult<()> {
    let mut document = material_document(json!({
        "name": "All ten",
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.25, 0.5, 0.75, 1],
            "metallicFactor": 0.375, "roughnessFactor": 0.625,
            "baseColorTexture": {"index": 0, "texCoord": 0},
            "metallicRoughnessTexture": {"index": 0, "texCoord": 1}
        },
        "normalTexture": {"index": 0, "texCoord": 2, "scale": -0.5},
        "occlusionTexture": {"index": 0, "texCoord": 3, "strength": 0.25},
        "emissiveTexture": {"index": 0, "texCoord": 4},
        "emissiveFactor": [0.25, 0.5, 1.0],
        "extensions": {
            "KHR_materials_clearcoat": {
                "clearcoatTexture": {"index": 0, "texCoord": 5},
                "clearcoatRoughnessTexture": {"index": 0, "texCoord": 6},
                "clearcoatNormalTexture": {"index": 0, "texCoord": 7, "scale": -2.5}
            },
            "KHR_materials_sheen": {
                "sheenColorTexture": {"index": 0, "texCoord": 8},
                "sheenRoughnessTexture": {"index": 0, "texCoord": 9}
            }
        }
    }))?;
    document["extensionsUsed"] = json!([CLEARCOAT, SHEEN]);
    document["materials"].as_array_mut().unwrap().extend([
        json!({"name": "Omitted", "extensions": {"KHR_materials_sheen": null}, "emissiveTexture": {"index": 2}}),
        json!({"name": "Empty"}),
        json!({"name": "Trailing", "extensions": {"KHR_materials_sheen": {
            "sheenRoughnessTexture": {"index": 1}
        }}}),
    ]);
    document["textures"].as_array_mut().unwrap().extend([
        json!({"source": 0, "sampler": 1, "name": "Same image, another sampler"}),
        json!({"source": 1}),
    ]);
    document["images"]
        .as_array_mut()
        .unwrap()
        .push(json!({"uri": "omitted.png"}));
    document["samplers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"wrapS": 10497, "wrapT": 10497, "magFilter": 9728}));
    let primitive = document["meshes"][0]["primitives"][0].clone();
    for index in [2, 3] {
        let mut selected = primitive.clone();
        selected["material"] = json!(index);
        document["meshes"][0]["primitives"]
            .as_array_mut()
            .unwrap()
            .push(selected);
    }
    let report = import_document(serde_json::to_vec(&document)?)?;
    let scene = report.scene();
    assert_eq!(
        (scene.material_count(), scene.material_binding_count()),
        (3, 11)
    );
    assert_eq!(
        (
            scene.texture_count(),
            scene.image_count(),
            scene.sampler_count()
        ),
        (2, 1, 2)
    );
    let materials = scene.materials().collect::<Vec<_>>();
    assert_eq!(
        materials
            .iter()
            .map(|material| material.name())
            .collect::<Vec<_>>(),
        [Some("All ten"), Some("Empty"), Some("Trailing")]
    );
    assert_eq!(materials[0].binding_count(), 10);
    assert_eq!(materials[1].binding_count(), 0);
    assert_eq!(materials[2].binding_count(), 1);
    assert!(materials[1].factors().clearcoat().is_none());
    assert!(materials[1].factors().sheen().is_none());
    let mr = materials[0].factors().pbr_metallic_roughness().unwrap();
    let base = mr.base_color();
    assert_eq!(
        [
            base.r,
            base.g,
            base.b,
            base.a,
            mr.metallic(),
            mr.roughness()
        ],
        [0.25, 0.5, 0.75, 1.0, 0.375, 0.625]
    );
    let expected: [(
        TextureRole,
        &str,
        &[TextureChannel],
        ColorInterpretation,
        f32,
    ); 10] = [
        (
            TextureRole::BaseColor,
            "baseColorTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
                TextureChannel::Alpha,
            ],
            ColorInterpretation::SrgbColor,
            1.0,
        ),
        (
            TextureRole::MetallicRoughness,
            "metallicRoughnessTexture",
            &[TextureChannel::Blue, TextureChannel::Green],
            ColorInterpretation::Data,
            1.0,
        ),
        (
            TextureRole::Normal,
            "normalTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::Data,
            -0.5,
        ),
        (
            TextureRole::Occlusion,
            "occlusionTexture",
            &[TextureChannel::Red],
            ColorInterpretation::Data,
            0.25,
        ),
        (
            TextureRole::Emissive,
            "emissiveTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::SrgbColor,
            1.0,
        ),
        (
            TextureRole::Clearcoat,
            "clearcoatTexture",
            &[TextureChannel::Red],
            ColorInterpretation::Data,
            1.0,
        ),
        (
            TextureRole::ClearcoatRoughness,
            "clearcoatRoughnessTexture",
            &[TextureChannel::Green],
            ColorInterpretation::Data,
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
            ColorInterpretation::Data,
            -2.5,
        ),
        (
            TextureRole::SheenColor,
            "sheenColorTexture",
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
            ],
            ColorInterpretation::SrgbColor,
            1.0,
        ),
        (
            TextureRole::SheenRoughness,
            "sheenRoughnessTexture",
            &[TextureChannel::Alpha],
            ColorInterpretation::Data,
            1.0,
        ),
    ];
    for (uv, (binding, (role, key, channels, transfer, strength))) in
        materials[0].bindings().zip(expected).enumerate()
    {
        assert_eq!(binding.role(), role);
        assert_eq!(binding.source_key(), Some(key));
        assert_eq!(binding.channel_mapping().as_slice(), channels);
        assert_eq!(binding.color_interpretation(), transfer);
        assert_eq!(usize::from(binding.uv_set()), uv);
        assert_eq!(binding.strength().get().to_bits(), strength.to_bits());
        assert_eq!(binding.blend(), TextureBlendOperation::Replace);
    }
    let first = materials[0].bindings().next().unwrap().texture();
    let trailing = materials[2].bindings().next().unwrap();
    assert_eq!(trailing.role(), TextureRole::SheenRoughness);
    assert_eq!(
        trailing.channel_mapping().as_slice(),
        &[TextureChannel::Alpha]
    );
    assert_ne!(first.id(), trailing.texture().id());
    assert_eq!(first.image().id(), trailing.texture().image().id());
    assert_ne!(first.sampler().id(), trailing.texture().sampler().id());
    assert_ne!(
        first.sampler().state(),
        trailing.texture().sampler().state()
    );
    assert_eq!(first.image().encoded_bytes(), triangle_png());
    for (material, source_index, texture_index) in [(materials[0], 0, 0), (materials[2], 3, 1)] {
        for binding in material.bindings() {
            for prefix in [
                format!("kind=material;parent=-;index={source_index};"),
                format!("kind=texture;parent=-;index={texture_index};"),
            ] {
                assert!(scene.owner_provenance(binding.metadata_owner())?.any(|node| matches!(node.kind(), ProvenanceKindRef::Source(source) if matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value.starts_with(&prefix)))));
            }
        }
    }
    let scope = report.source_scope_record().unwrap();
    for (kind, selected, omitted) in [
        (SourceScopeEntityKind::Material, 3, 1),
        (SourceScopeEntityKind::Texture, 2, 1),
        (SourceScopeEntityKind::Image, 1, 1),
        (SourceScopeEntityKind::Sampler, 2, 0),
    ] {
        assert_eq!(scope.counts(kind).selected(), selected);
        assert_eq!(scope.counts(kind).omitted(), omitted);
    }
    let mut resources = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    let mut expected_resources = vec![SOURCE, BINARY, IMAGE];
    expected_resources.sort_unstable();
    assert_eq!(resources, expected_resources);
    Ok(())
}

#[test]
fn all_five_layer_slots_extend_the_selected_image_and_sampler_closure() -> TestResult<()> {
    const IMAGES: [&str; 5] = [
        "models/textures/coat.png",
        "models/textures/coat-roughness.png",
        "models/textures/coat-normal.png",
        "models/textures/sheen-color.png",
        "models/textures/sheen-roughness.png",
    ];
    let mut document = material_document(json!({
        "normalTexture": {"index": 2},
        "extensions": {
            "KHR_materials_clearcoat": {
                "clearcoatTexture": {"index": 0},
                "clearcoatRoughnessTexture": {"index": 1},
                "clearcoatNormalTexture": {"index": 2}
            },
            "KHR_materials_sheen": {
                "sheenColorTexture": {"index": 3},
                "sheenRoughnessTexture": {"index": 4}
            }
        }
    }))?;
    document["extensionsUsed"] = json!([CLEARCOAT, SHEEN]);
    document["images"] = json!([
        {"uri": "textures/coat.png"},
        {"uri": "textures/coat-roughness.png"},
        {"uri": "textures/coat-normal.png"},
        {"uri": "textures/sheen-color.png"},
        {"uri": "textures/sheen-roughness.png"},
        {"uri": "textures/omitted.png"}
    ]);
    document["textures"] = json!([
        {"source": 0, "sampler": 0},
        {"source": 1, "sampler": 1},
        {"source": 2, "sampler": 2},
        {"source": 3, "sampler": 3},
        {"source": 4, "sampler": 4},
        {"source": 5, "sampler": 5}
    ]);
    document["samplers"] = json!([{}, {}, {}, {}, {}, {}]);
    let assets = [
        (SOURCE, serde_json::to_vec(&document)?),
        (BINARY, triangle_bin()),
    ]
    .into_iter()
    .chain(IMAGES.into_iter().map(|path| (path, triangle_png())));
    let (report, diagnostics) = import_assets(SOURCE, assets, sidecar_options())?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    let scene = report.scene();
    assert_eq!(
        (
            scene.texture_count(),
            scene.image_count(),
            scene.sampler_count()
        ),
        (5, 5, 5)
    );
    let material = scene.materials().next().unwrap();
    assert_eq!(material.binding_count(), 6);
    for (binding, role) in material.bindings().skip(1).zip([
        TextureRole::Clearcoat,
        TextureRole::ClearcoatRoughness,
        TextureRole::ClearcoatNormal,
        TextureRole::SheenColor,
        TextureRole::SheenRoughness,
    ]) {
        assert_eq!(binding.role(), role);
        assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
    }
    let scope = report.source_scope_record().unwrap();
    for kind in [
        SourceScopeEntityKind::Texture,
        SourceScopeEntityKind::Image,
        SourceScopeEntityKind::Sampler,
    ] {
        assert_eq!(scope.counts(kind).selected(), 5);
        assert_eq!(scope.counts(kind).omitted(), 1);
    }
    let mut resources = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    let mut expected_resources = vec![SOURCE, BINARY];
    expected_resources.extend(IMAGES);
    expected_resources.sort_unstable();
    assert_eq!(resources, expected_resources);
    Ok(())
}

#[test]
fn each_sheen_slot_compiles_transform_uv_override_and_unit_strength() -> TestResult<()> {
    for (slot, role) in [
        ("sheenColorTexture", TextureRole::SheenColor),
        ("sheenRoughnessTexture", TextureRole::SheenRoughness),
    ] {
        for uv in [1_u16, u16::MAX] {
            let mut payload = json!({});
            payload[slot] = json!({"index": 0, "texCoord": 65536, "extensions": {
                "KHR_texture_transform": {
                    "offset": [-0.0, 0.125], "rotation": -0.375,
                    "scale": [2.0, -0.5], "texCoord": uv
                }
            }});
            let mut document = sheen_document(payload)?;
            document["extensionsUsed"] = json!([SHEEN, TRANSFORM]);
            let report = import_document(serde_json::to_vec(&document)?)?;
            let binding = report
                .scene()
                .materials()
                .next()
                .unwrap()
                .bindings()
                .next()
                .unwrap();
            assert_eq!(binding.role(), role);
            assert_eq!(binding.uv_set(), uv);
            assert_eq!(binding.strength().get(), 1.0);
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
                [-0.0_f32, 0.125, -0.375, 2.0, -0.5].map(f32::to_bits),
            );
        }
    }
    Ok(())
}

#[test]
fn selected_sheen_shape_color_and_roughness_domains_fail_before_dependencies() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!(true),
        json!(0),
        json!("sheen"),
        json!({"sheenColorFactor": null}),
        json!({"sheenColorFactor": []}),
        json!({"sheenColorFactor": [0, 0]}),
        json!({"sheenColorFactor": [0, 0, 0, 0]}),
        json!({"sheenColorFactor": [null, 0, 0]}),
        json!({"sheenColorFactor": [0, false, 0]}),
        json!({"sheenColorFactor": [0, 0, "1"]}),
        json!({"sheenRoughnessFactor": null}),
        json!({"sheenRoughnessFactor": false}),
        json!({"sheenRoughnessFactor": []}),
        json!({"sheenRoughnessFactor": {}}),
        json!({"sheenRoughnessFactor": "0.5"}),
    ] {
        let failure =
            reject_before_dependencies(serde_json::to_vec(&sheen_document(payload)?)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for number in [
        "-0.5",
        "1.25",
        "-1e-50",
        "1e-50",
        "-1e-400",
        "1e-400",
        "1.0000000001",
        "1e40",
        "1e400",
    ] {
        for member in [
            format!(r#""sheenColorFactor":[{number},0,0]"#),
            format!(r#""sheenColorFactor":[0,{number},0]"#),
            format!(r#""sheenColorFactor":[0,0,{number}]"#),
            format!(r#""sheenRoughnessFactor":{number}"#),
        ] {
            let material = format!(r#"{{"extensions":{{"KHR_materials_sheen":{{{member}}}}}}}"#);
            let failure = reject_before_dependencies(raw_material_document(&material)?, false)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn selected_sheen_texture_shapes_indices_and_transforms_fail_before_io() -> TestResult<()> {
    for slot in ["sheenColorTexture", "sheenRoughnessTexture"] {
        for info in [
            Value::Null,
            json!([]),
            json!(true),
            json!({}),
            json!({"index": null}),
            json!({"index": -1}),
            json!({"index": 1}),
            json!({"index": "0"}),
            json!({"index": 0, "texCoord": null}),
            json!({"index": 0, "texCoord": 65536}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": null}}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": {"texCoord": 65536}}}),
            json!({"index": 0, "extensions": {"KHR_texture_transform": {"rotation": 1e40}}}),
        ] {
            let mut document = sheen_document(json!({}))?;
            document["extensionsUsed"] = json!([SHEEN, TRANSFORM]);
            document["materials"][0]["extensions"][SHEEN][slot] = info;
            let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn declaration_only_is_absent_and_missing_or_duplicate_declarations_fail_pre_io() -> TestResult<()>
{
    let mut document = material_document(json!({"extras": {"KHR_materials_sheen": {}}}))?;
    document["extensionsRequired"] = json!([SHEEN]);
    let report = import_document(serde_json::to_vec(&document)?)?;
    assert!(
        report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .sheen()
            .is_none()
    );
    for field in ["extensionsUsed", "extensionsRequired"] {
        let mut document = sheen_document(json!({}))?;
        document[field] = json!([SHEEN, SHEEN]);
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for required in [false, true] {
        let mut document = sheen_document(json!({}))?;
        document.as_object_mut().unwrap().remove("extensionsUsed");
        if required {
            document["extensionsRequired"] = json!([SHEEN]);
        }
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    for slot in ["sheenColorTexture", "sheenRoughnessTexture"] {
        let mut payload = json!({});
        payload[slot] = json!({"index": 0, "extensions": {"KHR_texture_transform": {}}});
        let failure =
            reject_before_dependencies(serde_json::to_vec(&sheen_document(payload)?)?, false)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn sheen_and_unlit_are_excluded_while_specular_glossiness_stays_unsupported() -> TestResult<()> {
    for other in ["KHR_materials_unlit", "KHR_materials_pbrSpecularGlossiness"] {
        for allow_loss in [false, true] {
            let mut document = sheen_document(json!({"sheenColorFactor": [0, 0, 0]}))?;
            document["extensionsUsed"] = json!([SHEEN, other]);
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
fn misplaced_sheen_and_unknown_nested_properties_fail_even_with_loss_allowed() -> TestResult<()> {
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
        document["extensionsUsed"] = json!([SHEEN]);
        document.pointer_mut(path).unwrap()["extensions"] = json!({"KHR_materials_sheen": {}});
        let failure = reject_before_dependencies(serde_json::to_vec(&document)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    for payload in [
        json!({"unexpected": null}),
        json!({"extensions": {"VENDOR_nested": {}}}),
        json!({"sheenColorTexture": {"index": 0, "extensions": {"KHR_materials_sheen": {}}}}),
    ] {
        let failure =
            reject_before_dependencies(serde_json::to_vec(&sheen_document(payload)?)?, true)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
    }
    Ok(())
}

#[test]
fn omitted_invalid_sheen_and_nested_references_do_not_acquire_resources() -> TestResult<()> {
    for payload in [
        Value::Null,
        json!([]),
        json!({"sheenColorFactor": [2, 0, 0]}),
        json!({"sheenRoughnessTexture": {"index": 999}}),
        json!({"extensions": {"VENDOR_nested": {}}}),
    ] {
        let mut document = material_document(json!({}))?;
        document["materials"].as_array_mut().unwrap().push(json!({
            "extensions": {"KHR_materials_sheen": payload},
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
        assert_eq!(
            (
                report.scene().material_count(),
                report.scene().texture_count(),
                report.scene().image_count()
            ),
            (1, 0, 0)
        );
        assert!(
            report
                .scene()
                .materials()
                .next()
                .unwrap()
                .factors()
                .sheen()
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
fn exact_sheen_texture_and_transform_extras_keep_provenance_in_json_and_glb() -> TestResult<()> {
    let raw = r#"{ "label" : "\u0062", "number": 4.500e+02, "extensions":{"VENDOR_data":{}} }"#;
    let material = format!(
        r#"{{"name":"Cloth","extras":{{"parent":true}},"extensions":{{"KHR_materials_sh\u0065en":{{"sheenColorFactor":[0.25,0.5,0.75],"extras":{raw},"sheenColorTexture":{{"index":0,"extras":null,"extensions":{{"KHR_texture_transform":{{"extras":[1]}}}}}},"sheenRoughnessTexture":{{"index":0,"extras":{{}},"extensions":{{"KHR_texture_transform":{{"extras":[]}}}}}}}}}}}}"#
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
        assert_eq!(entries.len(), 6);
        for (key, expected) in [
            ("org.baozi.gltf:extras", r#"{"parent":true}"#),
            ("org.baozi.gltf:sheen-extras", raw),
            ("org.baozi.gltf:sheen-color-texture-extras", "null"),
            ("org.baozi.gltf:sheen-color-texture-transform-extras", "[1]"),
            ("org.baozi.gltf:sheen-roughness-texture-extras", "{}"),
            (
                "org.baozi.gltf:sheen-roughness-texture-transform-extras",
                "[]",
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
        }
    }
    Ok(())
}

#[test]
fn snapshots_observe_sheen_presence_factors_bindings_and_transforms() -> TestResult<()> {
    let mut digests = Vec::new();
    for material in [
        json!({}),
        json!({"extensions": {"KHR_materials_sheen": {}}}),
        json!({"extensions": {"KHR_materials_sheen": {"sheenColorFactor": [0.25, 0.5, 0.75]}}}),
        json!({"extensions": {"KHR_materials_sheen": {"sheenColorTexture": {"index": 0}}}}),
        json!({"extensions": {"KHR_materials_sheen": {"sheenColorTexture": {"index": 0, "extensions": {"KHR_texture_transform": {"scale": [2, 1]}}}}}}),
    ] {
        let mut document = material_document(material)?;
        document["extensionsUsed"] = json!([SHEEN, TRANSFORM]);
        let report = import_document(serde_json::to_vec(&document)?)?;
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 128, 1_000_000)?)?;
        digests.push(snapshot.digest());
    }
    for left in 0..digests.len() {
        for right in left + 1..digests.len() {
            assert_ne!(digests[left], digests[right]);
        }
    }
    Ok(())
}
