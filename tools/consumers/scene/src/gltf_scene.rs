//! Hand-authored glTF inputs checked through the public Baozi facade.

use baozi::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTarget, AnimationTimeDomain,
    AttributeData, AttributeDomain, AuthoredTransformKind, Color, ColorInterpretation, Importer,
    Mat4, MipmapFilter, MorphDeltaData, MorphDeltaSemantic, Quat, SceneLookupError, ShadingModel,
    SkinMatrixSpace, TexelFilter, TextureAddressMode, TextureChannelMapping, TextureChannelSwizzle,
    TextureRole, TextureTransform, TopologyKind, ValidatedScene, Vec3,
};
use std::sync::{Arc, atomic::AtomicUsize};

// A complete red, opaque, one-pixel PNG. The consumer reads encoded bytes, not decoded pixels.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

// Only the buffer and image descriptors differ between the two physical encodings.
// All accessor addresses, authored values, and semantic assertions are fixed by this consumer.
const DOCUMENT: &str = r#"{
  "asset": {"version": "2.0", "generator": "independent-baozi-scene-consumer"},
  "scene": 1,
  "scenes": [{"nodes": [0]}, {"name": "Selected", "nodes": [3]}],
  "nodes": [
    {"name": "Outside"},
    {"name": "MeshA", "mesh": 0, "skin": 0, "weights": [0.75, -0.25],
     "matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 7,8,9,1]},
    {"name": "MeshB", "mesh": 0, "skin": 0,
     "translation": [-2,0,1], "rotation": [0,0,1,0], "scale": [2,3,4]},
    {"name": "Root", "children": [2,4,1],
     "matrix": [1,0,0,0, 0,2,0,0, 0,0,3,0, 10,20,30,1]},
    {"name": "JointRoot", "children": [5], "translation": [0,1,0]},
    {"name": "JointTip", "translation": [0,2,0]}
  ],
  "meshes": [{
    "name": "Panel", "weights": [0.25, 0.5],
    "primitives": [
      {"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2,
                      "JOINTS_0": 3, "WEIGHTS_0": 4},
       "indices": 5, "material": 0, "targets": [{"POSITION": 6}, {}]},
      {"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2,
                      "JOINTS_0": 3, "WEIGHTS_0": 4}, "material": 1}
    ]
  }],
  "materials": [
    {"name": "Coat", "doubleSided": true, "alphaMode": "MASK", "alphaCutoff": 0.375,
     "emissiveFactor": [0.125,0.25,0.5],
     "pbrMetallicRoughness": {"baseColorFactor": [0.25,0.5,0.75,1],
       "metallicFactor": 0.125, "roughnessFactor": 0.875,
       "baseColorTexture": {"index": 0, "texCoord": 0}}},
    {"name": "", "alphaMode": "BLEND",
     "pbrMetallicRoughness": {"baseColorFactor": [1,0.5,0.25,0.75],
       "metallicFactor": 0.5, "roughnessFactor": 0.25,
       "baseColorTexture": {"index": 1, "texCoord": 0}}}
  ],
  "images": [@IMAGE@],
  "textures": [{"source": 0, "sampler": 0}, {"source": 0, "sampler": 0}],
  "samplers": [{"magFilter": 9728, "minFilter": 9987, "wrapS": 33071, "wrapT": 33648}],
  "skins": [{"name": "Rig", "skeleton": 4, "joints": [5,4], "inverseBindMatrices": 7}],
  "buffers": [@BUFFER@],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 72, "byteStride": 24},
    {"buffer": 0, "byteOffset": 72, "byteLength": 24},
    {"buffer": 0, "byteOffset": 96, "byteLength": 12},
    {"buffer": 0, "byteOffset": 108, "byteLength": 48},
    {"buffer": 0, "byteOffset": 156, "byteLength": 6},
    {"buffer": 0, "byteOffset": 164, "byteLength": 36},
    {"buffer": 0, "byteOffset": 200, "byteLength": 128},
    {"buffer": 0, "byteOffset": 328, "byteLength": 8},
    {"buffer": 0, "byteOffset": 336, "byteLength": 24},
    {"buffer": 0, "byteOffset": 360, "byteLength": 32},
    {"buffer": 0, "byteOffset": 392, "byteLength": 72},
    {"buffer": 0, "byteOffset": 464, "byteLength": 48},
    {"buffer": 0, "byteOffset": 512, "byteLength": 16},
    {"buffer": 0, "byteOffset": 528, "byteLength": 70}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
     "min": [-1,0,0], "max": [2,3,0]},
    {"bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": 3, "type": "VEC3"},
    {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"},
    {"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"},
    {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"},
    {"bufferView": 4, "componentType": 5123, "count": 3, "type": "SCALAR"},
    {"bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC3",
     "min": [0,0,0], "max": [1,0.5,0.25]},
    {"bufferView": 6, "componentType": 5126, "count": 2, "type": "MAT4"},
    {"bufferView": 7, "componentType": 5126, "count": 2, "type": "SCALAR",
     "min": [0.25], "max": [1.5]},
    {"bufferView": 8, "componentType": 5126, "count": 2, "type": "VEC3"},
    {"bufferView": 9, "componentType": 5126, "count": 2, "type": "VEC4"},
    {"bufferView": 10, "componentType": 5126, "count": 6, "type": "VEC3"},
    {"bufferView": 11, "componentType": 5126, "count": 12, "type": "SCALAR"},
    {"bufferView": 12, "componentType": 5126, "count": 4, "type": "SCALAR"}
  ],
  "animations": [
    {"name": "Outside", "samplers": [{"input": 8, "output": 9}],
     "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]},
    {"name": "Pose-Δ", "samplers": [
       {"input": 8, "output": 9, "interpolation": "LINEAR"},
       {"input": 8, "output": 10, "interpolation": "STEP"},
       {"input": 8, "output": 11, "interpolation": "CUBICSPLINE"}],
     "channels": [
       {"sampler": 0, "target": {"node": 2, "path": "translation"}},
       {"sampler": 1, "target": {"node": 2, "path": "rotation"}},
       {"sampler": 2, "target": {"node": 2, "path": "scale"}}]},
    {"name": "", "samplers": [
       {"input": 8, "output": 12, "interpolation": "CUBICSPLINE"},
       {"input": 8, "output": 13, "interpolation": "STEP"}],
     "channels": [
       {"sampler": 0, "target": {"node": 1, "path": "weights"}},
       {"sampler": 1, "target": {"node": 2, "path": "weights"}}]}
  ]
}"#;

pub(crate) fn verify_gltf(importer: &Importer) -> baozi::Result<()> {
    let binary = binary();
    let embedded_json = document(false);
    let embedded = importer.read_bytes("scene.glb", glb(&embedded_json, &binary))?;
    assert_eq!(embedded.format().id().as_str(), "org.baozi.gltf");
    assert!(embedded.diagnostics().is_empty());
    assert!(embedded.loss_evidence().is_empty());
    assert_scene(embedded.scene());

    let io = crate::CountingIo {
        memory: baozi::MemoryAssetIo::new(),
        opens: AtomicUsize::new(0),
    };
    for (path, bytes) in [
        ("models/scene.gltf", document(true).into_bytes()),
        ("models/payload.bin", binary.clone()),
        ("models/paint.png", PNG.to_vec()),
    ] {
        io.memory
            .try_insert(baozi::AssetPath::new(path)?, Arc::from(bytes))?;
    }
    let external = importer.read_asset(&io, baozi::AssetPath::new("models/scene.gltf")?)?;
    assert_eq!(external.format().id().as_str(), "org.baozi.gltf");
    assert!(external.diagnostics().is_empty());
    assert!(external.loss_evidence().is_empty());
    assert_scene(external.scene());
    assert_eq!(io.opens.load(std::sync::atomic::Ordering::Relaxed), 3);
    assert_eq!(
        external
            .successful_input_manifest()
            .counts()
            .node_class_count(),
        3
    );
    assert!(matches!(
        external
            .scene()
            .mesh(embedded.scene().meshes().next().unwrap().id()),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));

    // Both malformed documents retain valid binary addresses and fail on their semantic contract.
    for (from, to) in [
        (
            r#""target": {"node": 1, "path": "weights"}"#,
            r#""target": {"node": 3, "path": "weights"}"#,
        ),
        (
            r#""count": 12, "type": "SCALAR""#,
            r#""count": 11, "type": "SCALAR""#,
        ),
    ] {
        assert_eq!(embedded_json.matches(from).count(), 1);
        let invalid = embedded_json.replacen(from, to, 1);
        match importer.read_bytes("invalid.glb", glb(&invalid, &binary)) {
            Err(baozi::Error::Import(failure)) => {
                let baozi::ImportCause::Format(envelope) = failure.cause() else {
                    panic!("expected a glTF format failure: {:?}", failure.cause());
                };
                assert_eq!(envelope.format().as_str(), "org.baozi.gltf");
                assert_eq!(envelope.code(), "gltf.invalid_document");
            }
            other => panic!("expected invalid glTF input to fail: {other:?}"),
        }
    }
    Ok(())
}

fn document(external: bool) -> String {
    let (buffer, image) = if external {
        (
            r#"{"uri": "payload.bin", "byteLength": 598}"#,
            r#"{"uri": "paint.png", "mimeType": "image/png"}"#,
        )
    } else {
        (
            r#"{"byteLength": 598}"#,
            r#"{"bufferView": 13, "mimeType": "image/png"}"#,
        )
    };
    DOCUMENT
        .replace("@BUFFER@", buffer)
        .replace("@IMAGE@", image)
}

fn binary() -> Vec<u8> {
    let mut bytes = Vec::new();
    // Three records: XYZ position followed by XYZ normal, with a 24-byte stride.
    floats(
        &mut bytes,
        &[
            -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 3.0, 0.0, 1.0, 0.0,
            0.0,
        ],
    );
    assert_eq!(bytes.len(), 72);
    floats(&mut bytes, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    bytes.extend_from_slice(&[0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]);
    floats(
        &mut bytes,
        &[0.75, 0.25, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0],
    );
    assert_eq!(bytes.len(), 156);
    for index in [2_u16, 0, 1] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    floats(&mut bytes, &[0.0, 0.0, 0.25, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(bytes.len(), 200);
    // Inverse binds follow the deliberately reversed joint palette [JointTip, JointRoot].
    floats(
        &mut bytes,
        &[
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, -2.0, -3.0, 1.0, 0.5,
            0.0, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 4.0, 5.0, 6.0, 1.0,
        ],
    );
    assert_eq!(bytes.len(), 328);
    floats(&mut bytes, &[0.25, 1.5]);
    floats(&mut bytes, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    floats(&mut bytes, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    assert_eq!(bytes.len(), 392);
    floats(
        &mut bytes,
        &[
            0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.25, 0.5, 0.75, -0.25, -0.5, -0.75, 2.0, 3.0, 4.0, 0.0,
            0.0, 0.0,
        ],
    );
    assert_eq!(bytes.len(), 464);
    floats(
        &mut bytes,
        &[
            -1.0, -2.0, 0.125, 0.875, 3.0, 4.0, 5.0, 6.0, 1.0, -0.5, -7.0, -8.0,
        ],
    );
    floats(&mut bytes, &[0.25, 0.5, 0.75, 1.0]);
    assert_eq!(bytes.len(), 528);
    bytes.extend_from_slice(PNG);
    assert_eq!(bytes.len(), 598);
    bytes
}

fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn glb(document: &str, binary: &[u8]) -> Vec<u8> {
    let mut json = document.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = binary.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = u32::try_from(28 + json.len() + bin.len()).expect("fixed consumer GLB fits u32");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&total.to_le_bytes());
    for (kind, payload) in [(b"JSON", json.as_slice()), (b"BIN\0", bin.as_slice())] {
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
    }
    bytes
}

fn assert_scene(scene: &ValidatedScene) {
    assert_eq!(scene.node_count(), 5);
    assert_eq!(scene.root_count(), 1);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 2);
    assert_eq!(scene.mesh_instance_count(), 2);
    assert_eq!(
        scene.nodes().map(|node| node.name()).collect::<Vec<_>>(),
        [
            Some("Root"),
            Some("MeshB"),
            Some("JointRoot"),
            Some("MeshA"),
            Some("JointTip")
        ]
    );
    let root = scene.roots().next().unwrap();
    assert_eq!(
        root.authored_transform().kind(),
        AuthoredTransformKind::Matrix
    );
    assert_eq!(
        root.authored_transform().authored_matrix(),
        Some(Mat4 {
            cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 2.0, 0.0, 0.0],
                [0.0, 0.0, 3.0, 0.0],
                [10.0, 20.0, 30.0, 1.0]
            ]
        })
    );
    let children = root.children().collect::<Vec<_>>();
    assert_eq!(
        children.iter().map(|node| node.name()).collect::<Vec<_>>(),
        [Some("MeshB"), Some("JointRoot"), Some("MeshA")]
    );
    assert_eq!(
        children[1].children().next().unwrap().name(),
        Some("JointTip")
    );
    let trs = children[0].authored_transform();
    assert_eq!(trs.kind(), AuthoredTransformKind::Trs);
    let trs = trs.authored_trs().unwrap();
    assert_eq!(trs.translation(), Vec3::new(-2.0, 0.0, 1.0));
    assert_eq!(trs.rotation(), Quat::from_xyzw(0.0, 0.0, 1.0, 0.0));
    assert_eq!(trs.scale(), Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(
        children[2].authored_transform().authored_matrix(),
        Some(Mat4 {
            cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [7.0, 8.0, 9.0, 1.0]
            ]
        })
    );

    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.name(), Some("Panel"));
    assert_eq!(mesh.primitive_count(), 2);
    let primitives = mesh.primitives().collect::<Vec<_>>();
    assert_eq!(primitives[0].indices(), Some(&[2, 0, 1][..]));
    assert_eq!(primitives[1].indices(), None);
    for primitive in &primitives {
        assert_eq!(primitive.topology(), TopologyKind::TriangleList);
        assert_eq!(primitive.vertex_count(), 3);
        assert_eq!(primitive.corner_count(), 3);
        assert_eq!(primitive.face_count(), 1);
        assert_eq!(
            primitive.positions().unwrap().domain(),
            AttributeDomain::Vertex
        );
        assert_eq!(
            primitive.positions().unwrap().data(),
            AttributeData::DenseF32 {
                lanes: 3,
                values: &[-1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0]
            }
        );
        assert_eq!(
            primitive.normals(AttributeDomain::Vertex).unwrap().data(),
            AttributeData::DenseF32 {
                lanes: 3,
                values: &[0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]
            }
        );
        assert_eq!(
            primitive
                .texcoords(0, AttributeDomain::Vertex)
                .unwrap()
                .data(),
            AttributeData::DenseF32 {
                lanes: 2,
                values: &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]
            }
        );
        let rows = primitive.influences().unwrap();
        assert_eq!(rows.len(), 3);
        for (vertex, expected) in [
            &[(0, 0.75), (1, 0.25)][..],
            &[(1, 1.0)][..],
            &[(0, 0.5), (1, 0.5)][..],
        ]
        .iter()
        .enumerate()
        {
            let contributions = rows
                .get(vertex)
                .unwrap()
                .iter()
                .filter(|influence| influence.weight() != 0.0)
                .map(|influence| (influence.palette_joint(), influence.weight()))
                .collect::<Vec<_>>();
            assert_eq!(contributions, *expected);
        }
        assert!(rows.get(3).is_none());
        assert_eq!(
            scene.primitive(primitive.id()).unwrap().id(),
            primitive.id()
        );
    }
    assert_eq!(primitives[0].material().unwrap().name(), Some("Coat"));
    assert_eq!(primitives[1].material().unwrap().name(), Some(""));
    assert_appearance(scene);
    assert_deformation(scene);
    assert_animation(scene);
}

fn assert_appearance(scene: &ValidatedScene) {
    assert_eq!(scene.material_count(), 2);
    assert_eq!(scene.material_binding_count(), 2);
    assert_eq!(scene.image_count(), 1);
    assert_eq!(scene.texture_count(), 2);
    assert_eq!(scene.sampler_count(), 1);
    let materials = scene.materials().collect::<Vec<_>>();
    assert_eq!(materials[0].name(), Some("Coat"));
    assert_eq!(materials[1].name(), Some(""));
    assert_eq!(materials[0].alpha_mode().mask_cutoff(), Some(0.375));
    assert_eq!(materials[1].alpha_mode(), baozi::AlphaMode::Blend);
    assert!(materials[0].double_sided());
    assert!(!materials[1].double_sided());
    assert_eq!(
        materials[0].factors().emissive(),
        Vec3::new(0.125, 0.25, 0.5)
    );
    assert_eq!(materials[1].factors().emissive(), Vec3::ZERO);
    for (index, color, metallic, roughness) in [
        (0, Color::linear_rgba(0.25, 0.5, 0.75, 1.0), 0.125, 0.875),
        (1, Color::linear_rgba(1.0, 0.5, 0.25, 0.75), 0.5, 0.25),
    ] {
        let material = materials[index];
        assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
        let factors = material.factors().pbr_metallic_roughness().unwrap();
        assert_eq!(factors.base_color(), color);
        assert_eq!(factors.metallic(), metallic);
        assert_eq!(factors.roughness(), roughness);
        assert_eq!(material.binding_count(), 1);
        let binding = material.bindings().next().unwrap();
        assert_eq!(binding.role(), TextureRole::BaseColor);
        assert_eq!(binding.coordinate_source().uv_set_index(), Some(0));
        assert_eq!(
            binding.channel_mapping(),
            TextureChannelMapping::Swizzle(TextureChannelSwizzle::RGBA)
        );
        assert_eq!(
            binding.color_interpretation(),
            ColorInterpretation::SrgbColor
        );
        assert_eq!(binding.transform(), TextureTransform::IDENTITY);
        assert_eq!(binding.strength().get(), 1.0);
        assert_eq!(
            binding.texture().id(),
            scene.textures().nth(index).unwrap().id()
        );
        assert_eq!(
            scene.material(material.id()).unwrap().name(),
            material.name()
        );
    }
    let textures = scene.textures().collect::<Vec<_>>();
    assert_ne!(textures[0].id(), textures[1].id());
    assert_eq!(textures[0].image().id(), textures[1].image().id());
    assert_eq!(textures[0].sampler().id(), textures[1].sampler().id());
    let image = textures[0].image();
    assert_eq!(image.media_type(), Some("image/png"));
    assert_eq!(image.encoded_bytes(), PNG);
    assert_eq!(scene.image(image.id()).unwrap().encoded_bytes(), PNG);
    let sampler = textures[0].sampler();
    let state = sampler.state();
    assert_eq!(state.minification(), TexelFilter::Linear);
    assert_eq!(state.magnification(), TexelFilter::Nearest);
    assert_eq!(state.mipmap(), MipmapFilter::Linear);
    assert_eq!(state.address_u(), TextureAddressMode::ClampToEdge);
    assert_eq!(state.address_v(), TextureAddressMode::MirroredRepeat);
    assert_eq!(state.address_w(), TextureAddressMode::Repeat);
    assert_eq!(scene.sampler(sampler.id()).unwrap().state(), state);
    assert_eq!(
        scene.texture(textures[1].id()).unwrap().id(),
        textures[1].id()
    );
}

fn assert_deformation(scene: &ValidatedScene) {
    let instances = scene.mesh_instances().collect::<Vec<_>>();
    assert_eq!(instances[0].node().name(), Some("MeshB"));
    assert_eq!(instances[1].node().name(), Some("MeshA"));
    assert_eq!(instances[0].mesh().id(), instances[1].mesh().id());
    assert_eq!(instances[0].morph_weights(), None);
    assert_eq!(instances[1].morph_weights(), Some(&[0.75, -0.25][..]));
    let skin = instances[0].skin().unwrap();
    assert_eq!(skin.id(), instances[1].skin().unwrap().id());
    assert_eq!(scene.skin_count(), 1);
    assert_eq!(skin.name(), Some("Rig"));
    assert_eq!(skin.skeleton_root().unwrap().name(), Some("JointRoot"));
    assert_eq!(skin.joint_count(), 2);
    assert_eq!(
        skin.joints()
            .map(|id| scene.node(id).unwrap().name())
            .collect::<Vec<_>>(),
        [Some("JointTip"), Some("JointRoot")]
    );
    assert_eq!(
        skin.inverse_bind_matrices().collect::<Vec<_>>(),
        [
            Mat4 {
                cols: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [-1.0, -2.0, -3.0, 1.0]
                ]
            },
            Mat4 {
                cols: [
                    [0.5, 0.0, 0.0, 0.0],
                    [0.0, 0.25, 0.0, 0.0],
                    [0.0, 0.0, 2.0, 0.0],
                    [4.0, 5.0, 6.0, 1.0]
                ]
            },
        ]
    );
    let spaces = skin.matrix_spaces();
    assert_eq!(
        spaces.input_position(),
        SkinMatrixSpace::LogicalMeshLocalBind
    );
    assert_eq!(spaces.inverse_bind_to(), SkinMatrixSpace::JointLocalBind);
    assert_eq!(spaces.current_joint_to(), SkinMatrixSpace::Scene);
    assert_eq!(spaces.output_position(), SkinMatrixSpace::MeshInstanceLocal);
    assert_eq!(spaces.correction(), None);
    assert_eq!(scene.skin(skin.id()).unwrap().name(), Some("Rig"));

    let mesh = instances[0].mesh();
    assert_eq!(mesh.morph_target_count(), 2);
    assert_eq!(mesh.default_morph_weights(), &[0.25, 0.5]);
    let definitions = mesh.morph_target_definitions().collect::<Vec<_>>();
    for (ordinal, definition) in definitions.iter().enumerate() {
        assert_eq!(definition.ordinal(), ordinal as u32);
        assert_eq!(definition.stable_identity(), ordinal as u32);
        assert_eq!(definition.name(), None);
        assert_eq!(definition.mesh().id(), mesh.id());
    }
    assert_eq!(definitions.len(), 2);
    let primitives = mesh.primitives().collect::<Vec<_>>();
    assert_eq!(primitives[0].morph_target_count(), 2);
    assert_eq!(primitives[1].morph_target_count(), 2);
    let target = primitives[0].morph_target(definitions[0].id()).unwrap();
    assert_eq!(target.definition().id(), definitions[0].id());
    assert_eq!(target.delta_count(), 1);
    let delta = target.delta(MorphDeltaSemantic::Position).unwrap();
    assert_eq!(
        delta.data(),
        MorphDeltaData::Dense {
            values: &[0.0, 0.0, 0.25, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0]
        }
    );
    assert_eq!(delta.data().value(1), Some(Vec3::new(0.0, 0.5, 0.0)));
    assert_eq!(delta.data().value(3), None);
    assert!(target.delta(MorphDeltaSemantic::Normal).is_none());
    assert_eq!(
        primitives[0]
            .morph_target(definitions[1].id())
            .unwrap()
            .delta_count(),
        0
    );
    for definition in definitions {
        let projected = primitives[1].morph_target(definition.id()).unwrap();
        assert_eq!(projected.id(), definition.id());
        assert_eq!(projected.delta_count(), 0);
    }
}

fn assert_animation(scene: &ValidatedScene) {
    assert_eq!(scene.animation_clip_count(), 2);
    let clips = scene.animation_clips().collect::<Vec<_>>();
    assert_ne!(clips[0].id(), clips[1].id());
    assert_eq!(clips[0].name(), Some("Pose-Δ"));
    assert_eq!(clips[1].name(), Some(""));
    assert_eq!(clips[0].channel_count(), 3);
    assert_eq!(clips[1].channel_count(), 2);
    for clip in &clips {
        assert_eq!(clip.duration(), 1.5);
        assert_eq!(clip.time_domain(), AnimationTimeDomain::Seconds);
        assert_eq!(scene.animation_clip(clip.id()).unwrap().name(), clip.name());
        for channel in clip.channels() {
            assert_eq!(channel.key_count(), 2);
            assert_eq!(channel.key_times(), &[0.25, 1.5]);
            assert_eq!(
                channel.pre_extrapolation(),
                AnimationExtrapolation::SourceDefault
            );
            assert_eq!(
                channel.post_extrapolation(),
                AnimationExtrapolation::SourceDefault
            );
        }
    }
    let instances = scene.mesh_instances().collect::<Vec<_>>();
    let node = instances[0].node().id();
    let translation = clips[0].channel(0).unwrap();
    assert_eq!(translation.target(), AnimationTarget::NodeTranslation(node));
    assert_eq!(translation.interpolation(), AnimationInterpolation::Linear);
    assert_eq!(translation.value_width(), 3);
    assert_eq!(translation.values(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(translation.in_tangents(), None);
    assert_eq!(translation.out_tangents(), None);
    let rotation = clips[0].channel(1).unwrap();
    assert_eq!(rotation.target(), AnimationTarget::NodeRotation(node));
    assert_eq!(rotation.interpolation(), AnimationInterpolation::Step);
    assert_eq!(rotation.value_width(), 4);
    assert_eq!(rotation.values(), &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    assert_eq!(rotation.in_tangents(), None);
    assert_eq!(rotation.out_tangents(), None);
    let scale = clips[0].channel(2).unwrap();
    assert_eq!(scale.target(), AnimationTarget::NodeScale(node));
    assert_eq!(scale.interpolation(), AnimationInterpolation::CubicSpline);
    assert_eq!(scale.value_width(), 3);
    assert_eq!(scale.values(), &[1.0, 1.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(
        scale.in_tangents(),
        Some(&[0.0, 0.0, 0.0, -0.25, -0.5, -0.75][..])
    );
    assert_eq!(
        scale.out_tangents(),
        Some(&[0.25, 0.5, 0.75, 0.0, 0.0, 0.0][..])
    );
    let weights = clips[1].channel(0).unwrap();
    assert_eq!(
        weights.target(),
        AnimationTarget::MeshInstanceMorphWeights(instances[1].id())
    );
    assert_eq!(weights.interpolation(), AnimationInterpolation::CubicSpline);
    assert_eq!(weights.value_width(), 2);
    assert_eq!(weights.values(), &[0.125, 0.875, 1.0, -0.5]);
    assert_eq!(weights.in_tangents(), Some(&[-1.0, -2.0, 5.0, 6.0][..]));
    assert_eq!(weights.out_tangents(), Some(&[3.0, 4.0, -7.0, -8.0][..]));
    let step = clips[1].channel(1).unwrap();
    assert_eq!(
        step.target(),
        AnimationTarget::MeshInstanceMorphWeights(instances[0].id())
    );
    assert_eq!(step.interpolation(), AnimationInterpolation::Step);
    assert_eq!(step.value_width(), 2);
    assert_eq!(step.values(), &[0.25, 0.5, 0.75, 1.0]);
    assert_eq!(step.in_tangents(), None);
    assert_eq!(step.out_tangents(), None);
}
