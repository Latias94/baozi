use std::sync::Arc;

use baozi::{
    AlphaMode, AssetOpenErrorKind, AssetPath, AttributeData, AttributeDomain, AttributeSemanticRef,
    DiagnosticSeverity, ImmutableMemoryAssetIo, ImportCause, ImportConfigOverlay, ImportDimension,
    ImportLossKind, ImportOperationControl, ImportRequest, ImportRequestLayer, Importer,
    LossPolicy, MaterialAssignment, OptionCompileLimits, ScopeAbortCause, ShadingModel,
    SourceScopeEntityKind, TextureRole, TextureTransform, ValidatedScene,
    postprocess::{
        PostProcessControl, PostProcessRunDisposition, SpaceConversionOptions, convert_scene_space,
    },
};
use baozi_asset_consumer::{ApplicationAsset, NormalIntent, Options, Result, process};
use serde_json::{Value, json};

const SOURCE: &str = "unlit/model.gltf";
const UNLIT: &str = "KHR_materials_unlit";
const UNSUPPORTED: &str = "KHR_materials_transmission";
const NAMES: [&str; 4] = ["lit", "unlit-opaque", "unlit-mask", "unlit-blend"];
const BINDINGS: [(TextureRole, &str); 5] = [
    (TextureRole::BaseColor, "baseColorTexture"),
    (TextureRole::MetallicRoughness, "metallicRoughnessTexture"),
    (TextureRole::Normal, "normalTexture"),
    (TextureRole::Occlusion, "occlusionTexture"),
    (TextureRole::Emissive, "emissiveTexture"),
];
const POSITIONS: [f32; 9] = [0.0, 0.0, 1.0, 2.0, 0.0, 1.0, 0.0, 2.0, 1.0];
const UV0: [f32; 6] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
const UV1: [f32; 6] = [0.25, 0.5, 0.75, 0.5, 0.25, 1.0];
const COLORS: [f32; 12] = [
    1.0, 0.5, 0.25, 0.75, 0.25, 1.0, 0.5, 0.5, 0.5, 0.25, 1.0, 1.0,
];
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];

fn document(required: bool) -> Value {
    let materials: Vec<_> = NAMES
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut material = json!({
                "name": name,
                "extras": {"fixture": "unlit-consumer", "material": name},
                "pbrMetallicRoughness": {
                    "baseColorFactor": [0.8, 0.4, 0.2, 0.6],
                    "metallicFactor": 0.625, "roughnessFactor": 0.375,
                    "baseColorTexture": {"index": 0, "texCoord": 0, "extensions": {
                        "KHR_texture_transform": {
                            "offset": [0.125, 0.25], "rotation": 0.375,
                            "scale": [2.0, 0.5], "texCoord": 1
                        }
                    }},
                    "metallicRoughnessTexture": {"index": 1}
                },
                "normalTexture": {"index": 2, "scale": 0.75},
                "occlusionTexture": {"index": 3, "strength": 0.25},
                "emissiveTexture": {"index": 4}, "emissiveFactor": [0.125, 0.25, 0.5]
            });
            if index != 0 {
                material["extensions"] = json!({UNLIT: {}});
            }
            if index == 2 {
                material["alphaMode"] = json!("MASK");
                material["alphaCutoff"] = json!(0.375);
                material["doubleSided"] = json!(true);
            } else if index == 3 {
                material["alphaMode"] = json!("BLEND");
                material["doubleSided"] = json!(false);
            }
            material
        })
        .collect();
    let primitives: Vec<_> = (0..NAMES.len())
        .map(|index| {
            json!({
                "attributes": {"POSITION": 0, "TEXCOORD_0": 1, "TEXCOORD_1": 2, "COLOR_0": 3},
                "material": index, "mode": 4
            })
        })
        .collect();
    let images: Vec<_> = (0..BINDINGS.len())
        .map(|index| json!({"uri": format!("image-{index}.png")}))
        .collect();
    let textures: Vec<_> = (0..BINDINGS.len())
        .map(|index| json!({"source": index}))
        .collect();
    let mut document = json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": [UNLIT, "KHR_texture_transform"],
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": primitives}], "materials": materials,
        "images": images, "textures": textures,
        "buffers": [{"uri": "mesh.bin", "byteLength": 132}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 24},
            {"buffer": 0, "byteOffset": 60, "byteLength": 24},
            {"buffer": 0, "byteOffset": 84, "byteLength": 48}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 1.0], "max": [2.0, 2.0, 1.0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"}
        ]
    });
    if required {
        document["extensionsRequired"] = json!([UNLIT]);
    }
    document
}

fn memory_io(document: &Value, missing_image: Option<usize>) -> Result<ImmutableMemoryAssetIo> {
    let mut payload = Vec::new();
    for value in POSITIONS.into_iter().chain(UV0).chain(UV1).chain(COLORS) {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(payload.len(), 132);
    let mut entries = vec![
        (
            AssetPath::new(SOURCE)?,
            Arc::<[u8]>::from(serde_json::to_vec(document)?),
        ),
        (AssetPath::new("unlit/mesh.bin")?, Arc::from(payload)),
    ];
    for index in 0..BINDINGS.len() {
        if missing_image != Some(index) {
            entries.push((
                AssetPath::new(format!("unlit/image-{index}.png"))?,
                Arc::from(PNG),
            ));
        }
    }
    Ok(ImmutableMemoryAssetIo::try_from_entries(entries)?)
}

fn assert_colors_and_uvs(scene: &ValidatedScene) -> Result<()> {
    for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
        for (semantic, lanes, expected) in [
            (AttributeSemanticRef::Color(0), 4, COLORS.as_slice()),
            (AttributeSemanticRef::Texcoord(1), 2, UV1.as_slice()),
        ] {
            let attribute = primitive
                .attribute(semantic, AttributeDomain::Vertex)
                .ok_or("missing authored unlit color or UV stream")?;
            let AttributeData::DenseF32 {
                lanes: actual_lanes,
                values,
            } = attribute.data()
            else {
                panic!("authored attributes must retain dense F32 storage");
            };
            assert_eq!((actual_lanes, values), (lanes, expected));
        }
    }
    Ok(())
}

#[test]
fn optional_and_required_unlit_survive_conversion_and_source_owner_destruction() -> Result<()> {
    for required in [false, true] {
        let asset = {
            let importer = Importer::new()?;
            let io = memory_io(&document(required), None)?;
            let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
            assert!(report.diagnostics().is_empty());
            assert!(report.loss_evidence().is_empty());
            // Equal factor groups and bindings do not select the rendering model.
            assert_eq!(
                report
                    .scene()
                    .materials()
                    .map(|material| material.shading_model())
                    .collect::<Vec<_>>(),
                [
                    ShadingModel::PbrMetallicRoughness,
                    ShadingModel::Unlit,
                    ShadingModel::Unlit,
                    ShadingModel::Unlit,
                ]
            );
            let lit_factors = report.scene().materials().next().unwrap().factors();
            assert!(
                report
                    .scene()
                    .materials()
                    .all(|material| material.factors() == lit_factors)
            );
            assert_colors_and_uvs(report.scene())?;
            let before = ApplicationAsset::from_scene(report.scene())?;
            let (scene, evidence) = report.into_parts();
            let (scene, conversion) = convert_scene_space(
                scene,
                SpaceConversionOptions {
                    target: Options::default().target,
                    source_assumptions: None,
                },
                PostProcessControl::default(),
            )?
            .into_parts();
            assert_eq!(conversion.disposition(), PostProcessRunDisposition::Changed);
            assert_colors_and_uvs(&scene)?;
            for (material, copied) in scene.materials().zip(&before.materials) {
                assert_eq!(material.shading_model(), copied.shading_model);
                assert_eq!(material.alpha_mode(), copied.alpha_mode);
                assert_eq!(material.double_sided(), copied.double_sided);
                assert_eq!(material.factors(), copied.factors);
                assert_eq!(material.binding_count(), BINDINGS.len());
                for (index, binding) in material.bindings().enumerate() {
                    assert_eq!(binding.source_key(), Some(BINDINGS[index].1));
                    assert_eq!(binding.role(), BINDINGS[index].0);
                    assert_eq!(
                        binding.texture().image().id(),
                        scene.images().nth(index).unwrap().id()
                    );
                    assert_eq!(binding.uv_set(), copied.bindings[index].uv_set);
                    assert_eq!(binding.transform(), copied.bindings[index].transform);
                    assert_eq!(binding.strength().get(), [1.0, 1.0, 0.75, 0.25, 1.0][index]);
                }
            }
            let asset = ApplicationAsset::from_scene(&scene)?;
            assert_eq!(asset.source_json_metadata, before.source_json_metadata);
            assert_ne!(asset.draws[0].vertices, before.draws[0].vertices);
            drop(conversion);
            drop(evidence);
            drop(scene);
            drop(io);
            drop(importer);
            asset
        };
        assert_eq!(
            (asset.draws.len(), asset.materials.len(), asset.images.len()),
            (4, 4, 5)
        );
        for (index, material) in asset.materials.iter().enumerate() {
            assert_eq!(
                asset.draws[index].material,
                MaterialAssignment::Authored(index)
            );
            let draw = &asset.draws[index];
            assert_eq!(
                draw.uv_streams
                    .iter()
                    .map(|stream| stream.set)
                    .collect::<Vec<_>>(),
                [0, 1]
            );
            for (vertex_index, vertex) in draw.vertices.iter().enumerate() {
                let source = UV0
                    .chunks_exact(2)
                    .position(|uv| {
                        draw.uv_stream(0).unwrap().values[vertex_index]
                            == [f64::from(uv[0]), f64::from(uv[1])]
                    })
                    .ok_or("missing source UV for owned vertex color")?;
                let expected: [f32; 4] = COLORS[source * 4..source * 4 + 4].try_into()?;
                assert_eq!(
                    vertex.color.map(|color| color.map(f64::to_bits)),
                    Some(expected.map(|value| f64::from(value).to_bits()))
                );
            }
            assert_eq!(material.name.as_deref(), Some(NAMES[index]));
            assert_eq!(
                material.shading_model,
                if index == 0 {
                    ShadingModel::PbrMetallicRoughness
                } else {
                    ShadingModel::Unlit
                }
            );
            assert_eq!(
                material.alpha_mode,
                [
                    AlphaMode::Opaque,
                    AlphaMode::Opaque,
                    AlphaMode::mask(0.375)?,
                    AlphaMode::Blend,
                ][index]
            );
            assert_eq!(material.double_sided, index == 2);
            // Lighting-only values remain fallback source truth for Unlit materials.
            let pbr = material
                .factors
                .pbr_metallic_roughness()
                .ok_or("missing fallback PBR")?;
            let base = pbr.base_color();
            assert_eq!([base.r, base.g, base.b, base.a], [0.8, 0.4, 0.2, 0.6]);
            assert_eq!((pbr.metallic(), pbr.roughness()), (0.625, 0.375));
            let emissive = material.factors.emission().factor();
            assert_eq!([emissive.x, emissive.y, emissive.z], [0.125, 0.25, 0.5]);
            assert_eq!(material.bindings.len(), BINDINGS.len());
            for (image, binding) in material.bindings.iter().enumerate() {
                assert_eq!((binding.role, binding.texture), (BINDINGS[image].0, image));
                let texture = &asset.textures[binding.texture];
                assert_eq!(texture.image, image);
                assert_eq!(asset.images[texture.image].encoded_bytes, PNG);
                assert!(texture.sampler < asset.samplers.len());
                let transform = binding.transform;
                if image == 0 {
                    assert_eq!(binding.uv_set, 1);
                    assert_eq!(
                        [
                            transform.offset().x,
                            transform.offset().y,
                            transform.rotation_radians(),
                            transform.scale().x,
                            transform.scale().y,
                        ],
                        [0.125, 0.25, 0.375, 2.0, 0.5]
                    );
                } else {
                    assert_eq!(binding.uv_set, 0);
                    assert_eq!(transform, TextureTransform::IDENTITY);
                }
            }
        }
        assert_eq!(asset.source_json_metadata.len(), NAMES.len());
        for (index, name) in NAMES.into_iter().enumerate() {
            let metadata = asset
                .source_json_metadata
                .iter()
                .find(|entry| {
                    entry
                        .locator
                        .starts_with(&format!("kind=material;parent=-;index={index};"))
                })
                .ok_or("missing owned material source metadata")?;
            assert_eq!(metadata.resource, SOURCE);
            assert_eq!(metadata.format, "org.baozi.gltf");
            assert_eq!(metadata.locator_schema, "org.baozi.gltf:source-object");
            assert_eq!(metadata.locator_schema_version, 1);
            assert_eq!(metadata.key, "org.baozi.gltf:extras");
            assert_eq!(metadata.media_type, "application/json");
            assert_eq!(
                serde_json::from_slice::<Value>(&metadata.payload)?,
                json!({"fixture": "unlit-consumer", "material": name})
            );
        }
    }
    Ok(())
}

#[test]
fn unlit_base_factor_and_plain_texture_keep_default_material_state() -> Result<()> {
    for textured in [false, true] {
        let mut document = document(false);
        document["extensionsUsed"] = json!([UNLIT]);
        document["meshes"][0]["primitives"] = json!([document["meshes"][0]["primitives"][1]]);
        document["materials"][1] = json!({
            "extensions": {UNLIT: {}},
            "pbrMetallicRoughness": {"baseColorFactor": [0.25, 0.5, 0.75, 0.5]}
        });
        if textured {
            document["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"] =
                json!({"index": 0});
        } else {
            document["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("COLOR_0");
        }
        let io = memory_io(&document, None)?;
        let report = Importer::new()?.read_asset(&io, AssetPath::new(SOURCE)?)?;
        assert!(report.diagnostics().is_empty());
        assert!(report.loss_evidence().is_empty());
        let asset = ApplicationAsset::from_scene(report.scene())?;
        drop(report);
        drop(io);
        assert_eq!(asset.materials.len(), 1);
        let material = &asset.materials[0];
        assert_eq!(material.shading_model, ShadingModel::Unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert!(!material.double_sided);
        let pbr = material.factors.pbr_metallic_roughness().unwrap();
        let base = pbr.base_color();
        assert_eq!([base.r, base.g, base.b, base.a], [0.25, 0.5, 0.75, 0.5]);
        assert_eq!((pbr.metallic(), pbr.roughness()), (1.0, 1.0));
        assert_eq!(asset.images.len(), usize::from(textured));
        assert_eq!(material.bindings.len(), usize::from(textured));
        for (vertex, expected) in asset.draws[0].vertices.iter().zip(COLORS.chunks_exact(4)) {
            let expected: [f32; 4] = expected.try_into()?;
            assert_eq!(vertex.color, textured.then(|| expected.map(f64::from)));
        }
        if textured {
            let binding = &material.bindings[0];
            assert_eq!(
                (binding.role, binding.texture, binding.uv_set),
                (TextureRole::BaseColor, 0, 0)
            );
            assert_eq!(binding.transform, TextureTransform::IDENTITY);
            assert_eq!(
                asset.images[asset.textures[binding.texture].image].encoded_bytes,
                PNG
            );
        }
    }
    Ok(())
}

#[test]
fn unlit_lighting_fallback_images_stay_selected_and_required() -> Result<()> {
    let mut document = document(false);
    // Remove the lit draw so it cannot keep the fallback dependencies alive for Unlit.
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let importer = Importer::new()?;
    let io = memory_io(&document, None)?;
    let report = importer.read_asset(&io, AssetPath::new(SOURCE)?)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.scene().material_count(), 3);
    assert!(
        report
            .scene()
            .materials()
            .all(|material| material.shading_model() == ShadingModel::Unlit)
    );
    let scope = report
        .source_scope_record()
        .ok_or("missing resource selection evidence")?;
    for kind in [SourceScopeEntityKind::Texture, SourceScopeEntityKind::Image] {
        let counts = scope.counts(kind);
        assert_eq!((counts.selected(), counts.omitted()), (5, 0));
    }
    for (dimension, expected) in [
        (ImportDimension::AssetOpen, 7),
        (ImportDimension::DependencyReference, 6),
    ] {
        let counts = report
            .ledger()
            .dimension(dimension)
            .ok_or("missing dependency accounting")?;
        assert_eq!(
            (counts.attempted(), counts.consumed()),
            (expected, expected)
        );
    }
    drop(report);
    for missing in 1..BINDINGS.len() {
        let io = memory_io(&document, Some(missing))?;
        let error = importer
            .read_asset(&io, AssetPath::new(SOURCE)?)
            .unwrap_err();
        let baozi::Error::Import(failure) = error else {
            panic!("missing fallback image must retain an import failure");
        };
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::AssetOpen(error)
                    if error.kind() == AssetOpenErrorKind::NotFound)
        ));
        let context = failure
            .cause()
            .dependency_context()
            .ok_or("missing fallback image context")?;
        assert_eq!(
            context.requested_reference(),
            format!("image-{missing}.png")
        );
        assert!(failure.loss_evidence().is_empty());
    }
    Ok(())
}

#[test]
fn unsupported_optional_selected_payload_stays_a_hard_failure_under_loss_opt_in() -> Result<()> {
    let mut document = document(false);
    document["extensionsUsed"]
        .as_array_mut()
        .unwrap()
        .push(json!(UNSUPPORTED));
    document["materials"][1]["extensions"][UNSUPPORTED] = json!({"transmissionFactor": 0.5});
    let io = memory_io(&document, None)?;
    let importer = Importer::new()?;
    for policy in [
        LossPolicy::preserve_or_fail(),
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    ] {
        let request = importer.plan(
            &ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
                ImportConfigOverlay::default().with_loss_policy(policy),
            )),
            OptionCompileLimits::default(),
        )?;
        let error = request
            .read_asset(
                &io,
                AssetPath::new(SOURCE)?,
                ImportOperationControl::default(),
            )
            .map_err(baozi::Error::from)
            .unwrap_err();
        let baozi::Error::Import(failure) = error else {
            panic!("unsupported selected payload must retain an import failure");
        };
        assert!(matches!(
            failure.cause(),
            ImportCause::Format(format) if format.code() == "gltf.unsupported_feature"
        ));
        assert!(failure.diagnostics().is_empty());
        assert!(failure.loss_evidence().is_empty());
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn declaration_without_selected_payload_warns_without_semantic_loss() -> Result<()> {
    let mut document = document(false);
    document["extensionsUsed"]
        .as_array_mut()
        .unwrap()
        .push(json!(UNSUPPORTED));
    let io = memory_io(&document, None)?;
    let report = Importer::new()?.read_asset(&io, AssetPath::new(SOURCE)?)?;
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.diagnostics().len(), 1);
    let diagnostic = &report.diagnostics()[0];
    assert_eq!(
        diagnostic.code().as_str(),
        "gltf.extension_uninterpreted_declaration"
    );
    assert_eq!(diagnostic.severity(), DiagnosticSeverity::Warning);
    assert_eq!(diagnostic.source().as_str(), SOURCE);
    assert!(diagnostic.message().contains(UNSUPPORTED));
    let asset = ApplicationAsset::from_scene(report.scene())?;
    drop(report);
    assert_eq!(asset.materials[1].shading_model, ShadingModel::Unlit);
    assert_eq!(asset.materials[1].bindings.len(), BINDINGS.len());
    Ok(())
}

#[test]
fn real_workflow_intent_preserves_unlit_absence_or_completes_static_flat_fields() -> Result<()> {
    for normal_intent in [NormalIntent::Preserve, NormalIntent::SourceFormat] {
        let output = {
            let io = memory_io(&document(false), None)?;
            process(
                &io,
                AssetPath::new(SOURCE)?,
                Options {
                    normal_intent,
                    ..Options::default()
                },
            )?
        };
        let asset = output.asset;
        drop(output.import_evidence);
        assert_eq!(
            asset
                .materials
                .iter()
                .map(|m| m.shading_model)
                .collect::<Vec<_>>(),
            [
                ShadingModel::PbrMetallicRoughness,
                ShadingModel::Unlit,
                ShadingModel::Unlit,
                ShadingModel::Unlit
            ]
        );
        assert_eq!(asset.materials[1].bindings.len(), BINDINGS.len());
        assert_eq!(asset.materials[2].alpha_mode, AlphaMode::mask(0.375)?);
        assert!(asset.materials[2].double_sided);
        assert_eq!(asset.materials[3].alpha_mode, AlphaMode::Blend);
        assert_eq!(asset.images.len(), BINDINGS.len());
        for draw in &asset.draws {
            assert_eq!(draw.uv_streams.len(), 2);
            for vertex in &draw.vertices {
                assert_eq!(
                    vertex.normal,
                    if normal_intent == NormalIntent::Preserve {
                        None
                    } else {
                        Some([0.0, 0.0, -1.0])
                    }
                );
                assert!(vertex.tangent.is_none());
            }
        }
    }
    Ok(())
}
