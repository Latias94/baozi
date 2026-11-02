use std::sync::Arc;

use baozi::{
    AlphaMode, AssetPath, AttributeDomain, ColorInterpretation, Importer, MaterialAssignment,
    MemoryAssetIo, MipmapFilter, SamplerState, ShadingModel, TexelFilter, TextureAddressMode,
    TextureBlendOperation, TextureChannel, TextureChannelSwizzle, TextureRole, TextureTransform,
    UvOrigin,
};
use baozi_asset_consumer::{
    ApplicationAsset, Result, TangentBasis, TangentEncoding, UvExtractionError,
    UvExtractionErrorKind,
};
use serde_json::{Value, json};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];
const POSITIONS: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
const UV0: [f32; 8] = [-0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
const UV1: [f32; 8] = [0.25, 0.125, 0.75, 0.125, 0.75, 0.625, 0.25, 0.625];

fn document() -> (Value, Vec<u8>) {
    let mut payload = Vec::new();
    for value in POSITIONS
        .into_iter()
        .chain(UV0)
        .chain(UV1)
        .chain([0.0, 0.0, 1.0].repeat(4))
        .chain([1.0, 0.0, 0.0, -1.0].repeat(4))
    {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0_u16, 1, 2, 0, 2, 3] {
        payload.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(payload.len(), 236);
    let material = json!({
        "name": "paint", "alphaMode": "MASK", "alphaCutoff": 0.375, "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.125, 0.25, 0.5, 0.75], "metallicFactor": 0.625, "roughnessFactor": 0.375,
            "baseColorTexture": {"index": 0, "texCoord": 0, "extensions": {"KHR_texture_transform": {
                "texCoord": 1, "offset": [0.125, 0.25], "scale": [2.0, 0.5], "rotation": std::f32::consts::FRAC_PI_2
            }}},
            "metallicRoughnessTexture": {"index": 2, "texCoord": 1}
        },
        "normalTexture": {"index": 1, "texCoord": 1, "scale": 0.75},
        "occlusionTexture": {"index": 3, "texCoord": 1, "strength": 0.25},
        "emissiveTexture": {"index": 1, "texCoord": 1}, "emissiveFactor": [0.25, 0.5, 0.75]
    });
    let mut unlit = material.clone();
    unlit["name"] = json!("");
    unlit["extensions"] = json!({"KHR_materials_unlit": {}});
    unlit["alphaMode"] = json!("BLEND");
    unlit["doubleSided"] = json!(false);
    let attributes =
        json!({"POSITION": 0, "TEXCOORD_0": 1, "TEXCOORD_1": 2, "NORMAL": 3, "TANGENT": 4});
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "extensionsUsed": ["KHR_texture_transform", "KHR_materials_unlit"],
        "materials": [material, unlit],
        "images": [
            {"name": "image-owner", "uri": "shared.png", "mimeType": "image/png"},
            {"name": "", "uri": "unknown.png"}
        ],
        "samplers": [
            {"name": "sampler-owner", "magFilter": 9728, "minFilter": 9986, "wrapS": 33648, "wrapT": 33071},
            {"name": "", "magFilter": 9729, "minFilter": 9729, "wrapS": 10497, "wrapT": 33648}
        ],
        "textures": [
            {"name": "color-use", "source": 0, "sampler": 0},
            {"name": "normal-use", "source": 0, "sampler": 1},
            {"name": "", "source": 0, "sampler": 0},
            {"source": 1, "sampler": 1}
        ],
        "meshes": [{"primitives": [
            {"attributes": attributes, "indices": 5, "material": 0},
            {"attributes": attributes, "indices": 5, "material": 1}
        ]}],
        "buffers": [{"uri": "mesh.bin", "byteLength": payload.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": 32},
            {"buffer": 0, "byteOffset": 80, "byteLength": 32},
            {"buffer": 0, "byteOffset": 112, "byteLength": 48},
            {"buffer": 0, "byteOffset": 160, "byteLength": 64},
            {"buffer": 0, "byteOffset": 224, "byteLength": 12}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
            {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5126, "count": 4, "type": "VEC3"},
            {"bufferView": 4, "componentType": 5126, "count": 4, "type": "VEC4"},
            {"bufferView": 5, "componentType": 5123, "count": 6, "type": "SCALAR"}
        ]
    });
    (document, payload)
}

fn memory_io(document: &Value, payload: &[u8]) -> Result<MemoryAssetIo> {
    let io = MemoryAssetIo::new();
    for (path, bytes) in [
        ("appearance.gltf", serde_json::to_vec(document)?),
        ("mesh.bin", payload.to_vec()),
        ("shared.png", PNG.to_vec()),
        ("unknown.png", PNG.to_vec()),
    ] {
        io.try_insert(AssetPath::new(path)?, Arc::from(bytes))?;
    }
    Ok(io)
}

fn copy_document(document: &Value, payload: &[u8]) -> Result<ApplicationAsset> {
    let io = memory_io(document, payload)?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("appearance.gltf")?)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(evidence);
    drop(importer);
    drop(io);
    Ok(asset)
}

#[test]
fn detached_emission_keeps_authored_factor_when_strength_is_present() -> Result<()> {
    for strength in [0.0, 1.0, 4.0] {
        let (mut document, payload) = document();
        document["extensionsUsed"]
            .as_array_mut()
            .unwrap()
            .push(json!("KHR_materials_emissive_strength"));
        document["materials"][0]["extensions"] =
            json!({"KHR_materials_emissive_strength": {"emissiveStrength": strength}});
        let asset = copy_document(&document, &payload)?;
        let emission = asset.materials[0].factors.emission();
        let factor = emission.factor();
        assert_eq!([factor.x, factor.y, factor.z], [0.25, 0.5, 0.75]);
        assert_eq!(emission.strength(), Some(strength as f32));
        assert_eq!(
            emission.effective_rgb(),
            [0.25 * strength, 0.5 * strength, 0.75 * strength]
        );
        assert!(asset.materials[1].factors.emission().strength().is_none());
        assert_eq!(asset.images[0].encoded_bytes, PNG);
        assert_eq!(asset.materials[0].bindings.len(), 5);
    }
    Ok(())
}

#[test]
fn complete_material_uv_and_sampling_closure_survives_all_source_owners() -> Result<()> {
    let asset = {
        let (document, payload) = document();
        copy_document(&document, &payload)?
    };
    assert_eq!(asset.uv_origin, UvOrigin::UpperLeft);
    assert_eq!(
        (
            asset.draws.len(),
            asset.materials.len(),
            asset.textures.len(),
            asset.samplers.len(),
            asset.images.len()
        ),
        (2, 2, 4, 2, 2)
    );
    assert_eq!(asset.images[0].encoded_bytes, PNG);
    assert_eq!(asset.images[0].media_type.as_deref(), Some("image/png"));
    assert_eq!(asset.images[1].encoded_bytes, PNG);
    assert_eq!(asset.images[1].media_type, None); // The .png path and bytes do not declare a media type.
    assert_eq!(asset.images[0].name.as_deref(), Some("image-owner"));
    assert_eq!(asset.images[1].name.as_deref(), Some(""));
    assert_eq!(asset.samplers[0].name.as_deref(), Some("sampler-owner"));
    assert_eq!(asset.samplers[1].name.as_deref(), Some(""));
    let expected_samplers = [
        SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Nearest,
            MipmapFilter::Linear,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::ClampToEdge,
            TextureAddressMode::Repeat,
        ),
        SamplerState::new(
            TexelFilter::Linear,
            TexelFilter::Linear,
            MipmapFilter::None,
            TextureAddressMode::Repeat,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::Repeat,
        ),
    ];
    for (sampler, expected) in asset.samplers.iter().zip(expected_samplers) {
        assert_eq!(sampler.state, expected);
    }
    for (texture, (image, sampler, name)) in asset.textures.iter().zip([
        (0, 0, Some("color-use")),
        (0, 1, Some("normal-use")),
        (0, 0, Some("")),
        (1, 1, None),
    ]) {
        assert_eq!(
            (texture.image, texture.sampler, texture.name.as_deref()),
            (image, sampler, name)
        );
    }
    // Textures 0/1 share an image with different samplers; 0/2 retain separate identities
    // even though both image and sampler links match. Binding order is independently authored.
    let expected_bindings = [
        (
            TextureRole::BaseColor,
            0,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::SrgbColor,
            1.0,
            "baseColorTexture",
        ),
        (
            TextureRole::MetallicRoughness,
            2,
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])?,
            ColorInterpretation::Data,
            1.0,
            "metallicRoughnessTexture",
        ),
        (
            TextureRole::Normal,
            1,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::Data,
            0.75,
            "normalTexture",
        ),
        (
            TextureRole::Occlusion,
            3,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            0.25,
            "occlusionTexture",
        ),
        (
            TextureRole::Emissive,
            1,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::SrgbColor,
            1.0,
            "emissiveTexture",
        ),
    ];
    for (index, (draw, material)) in asset.draws.iter().zip(&asset.materials).enumerate() {
        assert_eq!(draw.material, MaterialAssignment::Authored(index));
        assert_eq!(
            material.name.as_deref(),
            Some(if index == 0 { "paint" } else { "" })
        );
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
            if index == 0 {
                AlphaMode::mask(0.375)?
            } else {
                AlphaMode::Blend
            }
        );
        assert_eq!(material.double_sided, index == 0);
        let pbr = material
            .factors
            .pbr_metallic_roughness()
            .ok_or("missing PBR factors")?;
        let base = pbr.base_color();
        assert_eq!([base.r, base.g, base.b, base.a], [0.125, 0.25, 0.5, 0.75]);
        assert_eq!((pbr.metallic(), pbr.roughness()), (0.625, 0.375));
        let emissive = material.factors.emission().factor();
        assert_eq!([emissive.x, emissive.y, emissive.z], [0.25, 0.5, 0.75]);
        assert!(material.factors.pbr_specular_glossiness().is_none());
        assert!(material.factors.legacy().is_none());
        assert!(material.factors.clearcoat().is_none());
        assert!(material.factors.transmission().is_none());
        assert!(material.factors.thickness().is_none());
        assert!(material.factors.sheen().is_none());
        assert!(material.factors.emission().strength().is_none());
        assert_eq!((draw.vertices.len(), draw.indices.len()), (4, 6));
        assert_eq!(draw.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(
            draw.uv_streams
                .iter()
                .map(|stream| stream.set)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        for (stream, expected) in draw.uv_streams.iter().zip([UV0, UV1]) {
            assert_eq!(stream.domain, AttributeDomain::Vertex);
            for (actual, expected) in stream.values.iter().zip(expected.chunks_exact(2)) {
                assert_eq!(
                    actual.map(f64::to_bits),
                    [f64::from(expected[0]), f64::from(expected[1])].map(f64::to_bits)
                );
            }
        }
        let tangent = draw
            .tangent_conventions
            .as_ref()
            .ok_or("missing tangent conventions")?;
        assert_eq!(tangent.domain, AttributeDomain::Vertex);
        assert_eq!(tangent.basis, TangentBasis::Unknown);
        assert_eq!(tangent.encoding, TangentEncoding::SignInW);
        assert!(
            draw.vertices
                .iter()
                .all(|vertex| vertex.tangent == Some([1.0, 0.0, 0.0, -1.0]))
        );
        assert_eq!(material.bindings.len(), expected_bindings.len());
        for (binding, &(role, texture, channels, interpretation, strength, source_key)) in
            material.bindings.iter().zip(&expected_bindings)
        {
            assert_eq!(
                (binding.role, binding.texture, binding.uv_set),
                (role, texture, 1)
            );
            assert_eq!(binding.channel_mapping, channels);
            assert_eq!(binding.color_interpretation, interpretation);
            assert_eq!(binding.strength.get(), strength);
            assert_eq!(binding.blend, TextureBlendOperation::Replace);
            assert_eq!(binding.source_key.as_deref(), Some(source_key));
            let texture = &asset.textures[binding.texture];
            assert!(texture.image < asset.images.len() && texture.sampler < asset.samplers.len());
            let uv = draw
                .uv_stream(u32::from(binding.uv_set))
                .ok_or("missing bound UV")?
                .values[0];
            assert_eq!(uv, [0.25, 0.125]);
            if role == TextureRole::BaseColor {
                let transform = binding.transform;
                assert_eq!([transform.offset().x, transform.offset().y], [0.125, 0.25]);
                assert_eq!([transform.scale().x, transform.scale().y], [2.0, 0.5]);
                assert_eq!(transform.rotation_radians(), std::f32::consts::FRAC_PI_2);
                // The retained UV1 is scaled, rotated, then translated by the binding.
                // (1/4,1/8) -> (1/2,1/16) -> (-1/16,1/2) -> (1/16,3/4).
                let (sine, cosine) = f64::from(transform.rotation_radians()).sin_cos();
                let transformed = [
                    0.125 + 2.0 * uv[0] * cosine - 0.5 * uv[1] * sine,
                    0.25 + 2.0 * uv[0] * sine + 0.5 * uv[1] * cosine,
                ];
                for (actual, expected) in transformed.into_iter().zip([0.0625, 0.75]) {
                    assert!((actual - expected).abs() < 1.0e-7);
                }
            } else {
                assert_eq!(binding.transform, TextureTransform::IDENTITY);
            }
        }
    }
    Ok(())
}

#[test]
fn implicit_and_authored_defaults_keep_distinct_assignments_after_copy_and_drop() -> Result<()> {
    let asset = {
        let (mut document, payload) = document();
        document["materials"] = json!([{}]);
        document["meshes"][0]["primitives"][0]
            .as_object_mut()
            .unwrap()
            .remove("material");
        document["meshes"][0]["primitives"][1]["material"] = json!(0);
        copy_document(&document, &payload)?
    };
    assert_eq!(asset.materials.len(), 2);
    let implicit = asset.draws[0].material;
    let authored = asset.draws[1].material;
    assert!(matches!(implicit, MaterialAssignment::ImplicitDefault(_)));
    assert!(matches!(authored, MaterialAssignment::Authored(_)));
    assert_eq!(implicit.authored(), None);
    assert_eq!(authored.authored(), authored.effective());
    let implicit_index = implicit
        .effective()
        .ok_or("missing implicit effective material")?;
    let authored_index = authored
        .effective()
        .ok_or("missing authored effective material")?;
    assert_ne!(implicit_index, authored_index);
    let implicit = &asset.materials[implicit_index];
    let authored = &asset.materials[authored_index];
    for material in [implicit, authored] {
        assert_eq!(material.name, None);
        assert_eq!(material.shading_model, ShadingModel::PbrMetallicRoughness);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert!(!material.double_sided);
        assert!(material.bindings.is_empty());
        let pbr = material
            .factors
            .pbr_metallic_roughness()
            .ok_or("missing default PBR factors")?;
        let base = pbr.base_color();
        assert_eq!([base.r, base.g, base.b, base.a], [1.0; 4]);
        assert_eq!((pbr.metallic(), pbr.roughness()), (1.0, 1.0));
        let emissive = material.factors.emission().factor();
        assert_eq!([emissive.x, emissive.y, emissive.z], [0.0; 3]);
    }
    assert_eq!(implicit.factors, authored.factors);
    Ok(())
}

#[test]
fn bindings_on_uv1_retain_both_sets_without_substituting_uv0() -> Result<()> {
    // glTF requires contiguous TEXCOORD sets; UV0 remains independently retained even
    // when every effective material binding selects UV1.
    let (document, payload) = document();
    let asset = copy_document(&document, &payload)?;
    for draw in &asset.draws {
        assert_eq!(draw.uv_streams.len(), 2);
        assert_eq!(
            draw.uv_stream(0).unwrap().values[0].map(f64::to_bits),
            [-0.0_f64, 0.0].map(f64::to_bits)
        );
        assert_eq!(draw.uv_stream(1).unwrap().values[0], [0.25, 0.125]);
        let material = &asset.materials[draw.material.effective().unwrap()];
        assert!(material.bindings.iter().all(|binding| binding.uv_set == 1));
    }
    Ok(())
}

#[test]
fn absent_names_stay_absent_on_their_own_image_texture_and_sampler() -> Result<()> {
    for absent_owner in ["images", "textures", "samplers"] {
        let asset = {
            let (mut document, payload) = document();
            document[absent_owner][0]
                .as_object_mut()
                .unwrap()
                .remove("name");
            copy_document(&document, &payload)?
        };
        for (owner, actual, name) in [
            ("images", asset.images[0].name.as_deref(), "image-owner"),
            ("textures", asset.textures[0].name.as_deref(), "color-use"),
            (
                "samplers",
                asset.samplers[0].name.as_deref(),
                "sampler-owner",
            ),
        ] {
            assert_eq!(
                actual,
                if owner == absent_owner {
                    None
                } else {
                    Some(name)
                }
            );
        }
        assert_eq!(asset.textures[1].name.as_deref(), Some("normal-use"));
        assert_eq!(asset.textures[2].name.as_deref(), Some(""));
    }
    Ok(())
}

#[test]
fn missing_referenced_uv_names_the_draw_binding_and_set_even_for_unlit_fallback() -> Result<()> {
    for unlit_only in [false, true] {
        let (mut document, payload) = document();
        if unlit_only {
            document["meshes"][0]["primitives"] = json!([document["meshes"][0]["primitives"][1]]);
            document["materials"][1]["normalTexture"]["texCoord"] = json!(9);
        } else {
            document["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("TEXCOORD_1");
        }
        let io = memory_io(&document, &payload)?;
        let importer = Importer::new()?;
        let report = importer.read_asset(&io, AssetPath::new("appearance.gltf")?)?;
        let error = ApplicationAsset::from_scene(report.scene()).unwrap_err();
        drop(report);
        drop(importer);
        drop(io);
        assert_eq!(
            error.downcast_ref::<UvExtractionError>(),
            Some(&UvExtractionError {
                mesh: 0,
                primitive: 0,
                binding: Some(if unlit_only { 2 } else { 0 }),
                uv_set: if unlit_only { 9 } else { 1 },
                kind: UvExtractionErrorKind::Missing,
            })
        );
    }
    Ok(())
}

#[test]
fn public_ply_copy_preserves_f64_uv_lanes_and_signed_zero() -> Result<()> {
    let asset = {
        let io = MemoryAssetIo::new();
        let source = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty double x\nproperty double y\nproperty double z\nproperty double u\nproperty double v\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 16777217 -0\n1 0 0 16777218 1e-200\n0 1 0 0 1\n3 0 1 2\n";
        io.try_insert(
            AssetPath::new("wide.ply")?,
            Arc::<[u8]>::from(source.as_slice()),
        )?;
        let importer = Importer::new()?;
        let report = importer.read_asset(&io, AssetPath::new("wide.ply")?)?;
        let (scene, evidence) = report.into_parts();
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        drop(evidence);
        drop(importer);
        drop(io);
        asset
    };
    assert_eq!(asset.draws[0].material, MaterialAssignment::Unspecified);
    assert_eq!(asset.draws[0].material.authored(), None);
    assert_eq!(asset.draws[0].material.effective(), None);
    let uv = asset.draws[0].uv_stream(0).ok_or("missing F64 UV stream")?;
    for (actual, expected) in uv.values.iter().zip([
        [16_777_217.0_f64, -0.0],
        [16_777_218.0, 1.0e-200],
        [0.0, 1.0],
    ]) {
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
    }
    Ok(())
}

#[test]
fn public_obj_legacy_factors_are_owned_independently_from_pbr_groups() -> Result<()> {
    let asset = {
        let io = MemoryAssetIo::new();
        for (path, bytes) in [
            ("legacy.obj", b"mtllib legacy.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl brass\nf 1 2 3\n".as_slice()),
            ("legacy.mtl", b"newmtl brass\nillum 2\nKa 0.125 0.25 0.5\nKd 2 0.5 0.25\nKs 0.75 0.5 0.25\nKe 0.25 0.5 0.75\nNs 64\nd 0.625\n".as_slice()),
        ] { io.try_insert(AssetPath::new(path)?, Arc::from(bytes))?; }
        let importer = Importer::new()?;
        let report = importer.read_asset(&io, AssetPath::new("legacy.obj")?)?;
        let (scene, evidence) = report.into_parts();
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        drop(evidence);
        drop(importer);
        drop(io);
        asset
    };
    let material = &asset.materials[0];
    assert_eq!(material.name.as_deref(), Some("brass"));
    assert_eq!(material.shading_model, ShadingModel::Phong);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert!(!material.double_sided);
    let legacy = material.factors.legacy().ok_or("missing legacy factors")?;
    for (actual, expected) in [
        (legacy.ambient(), [0.125, 0.25, 0.5]),
        (legacy.diffuse(), [2.0, 0.5, 0.25]),
        (legacy.specular(), [0.75, 0.5, 0.25]),
        (legacy.emissive(), [0.25, 0.5, 0.75]),
    ] {
        assert_eq!([actual.x, actual.y, actual.z], expected);
    }
    assert_eq!((legacy.shininess(), legacy.opacity()), (64.0, 0.625));
    assert!(material.factors.pbr_metallic_roughness().is_none());
    assert!(material.factors.pbr_specular_glossiness().is_none());
    assert!(material.factors.clearcoat().is_none());
    assert!(material.factors.transmission().is_none());
    assert!(material.factors.thickness().is_none());
    assert!(material.factors.sheen().is_none());
    assert!(material.factors.emission().strength().is_none());
    assert!(material.bindings.is_empty());
    Ok(())
}
