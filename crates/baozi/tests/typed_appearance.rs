#![cfg(feature = "format-obj")]

use std::sync::Arc;

use baozi::{
    AlphaMode, AssetPath, ColorInterpretation, ImageRef, ImportReport, Importer,
    MaterialBindingRef, MaterialRef, MemoryAssetIo, SamplerRef, SamplerState, SceneEntityKind,
    SceneLookupError, ShadingModel, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRef, TextureRole, TextureStrength,
    TextureTransform, ValidatedScene, Vec3,
};

const OBJ: &[u8] = b"mtllib materials/paint.mtl\n\
v 0 0 0\nv 1 0 0\nv 0 1 0\n\
vt 0 0\nvt 1 0\nvt 0 1\n\
o red\nusemtl red\nf 1/1 2/2 3/3\n\
o matte\nusemtl matte\nf 1/1 2/2 3/3\n\
o blue\nusemtl blue-painted\nf 1/1 2/2 3/3\n";

const MTL: &[u8] = b"newmtl red\nKd 0.8 0.1 0.2\nKe 0.1 0.2 0.3\n\
Ns 32\nd 0.5\nmap_Kd textures/red.png\n\
newmtl matte\nKd 0.4 0.5 0.6\nNs 4\n\
newmtl blue-painted\nKd 0.2 0.3 0.9\nNs 8\nmap_Kd textures/blue.png\n";

// Images are retained as encoded bytes; this API does not require a pixel decoder.
const RED_BYTES: &[u8] = b"first-encoded-texture";
const BLUE_BYTES: &[u8] = b"a-longer-second-encoded-texture";

fn appearance_report() -> ImportReport {
    let io = MemoryAssetIo::new();
    for (path, bytes) in [
        ("models/paint.obj", OBJ),
        ("models/materials/paint.mtl", MTL),
        ("models/materials/textures/red.png", RED_BYTES),
        ("models/materials/textures/blue.png", BLUE_BYTES),
        (
            "models/textures/red.png",
            b"wrong-origin-texture".as_slice(),
        ),
    ] {
        io.try_insert(
            AssetPath::new(path).expect("valid fixture path"),
            Arc::from(bytes),
        )
        .expect("fixture asset insertion");
    }
    Importer::new()
        .expect("default importers register")
        .read_asset(
            &io,
            AssetPath::new("models/paint.obj").expect("valid model path"),
        )
        .expect("OBJ, memory MTL, and textures import")
}

// These helpers deliberately return Scene borrows through temporary views and iterators.
fn last_binding(scene: &ValidatedScene) -> MaterialBindingRef<'_> {
    scene
        .materials()
        .next_back()
        .unwrap()
        .bindings()
        .next()
        .unwrap()
}

fn last_encoded_bytes(scene: &ValidatedScene) -> &[u8] {
    last_binding(scene).texture().image().encoded_bytes()
}

fn last_material_name(scene: &ValidatedScene) -> &str {
    scene.materials().next_back().unwrap().name().unwrap()
}

fn last_source_key(scene: &ValidatedScene) -> &str {
    last_binding(scene).source_key().unwrap()
}

#[test]
fn facade_reads_material_factors_bindings_and_encoded_images() {
    let report = appearance_report();
    assert!(report.diagnostics().is_empty());
    let scene = report.scene();
    assert_eq!(scene.material_count(), 3);
    assert_eq!(scene.material_binding_count(), 2);
    assert_eq!(scene.image_count(), 2);
    assert_eq!(scene.sampler_count(), 2);
    assert_eq!(scene.texture_count(), 2);
    assert_eq!(scene.mesh_count(), 3);
    assert_eq!(scene.primitive_count(), 3);

    // Start with nonzero name, binding, factor, source-key, and image payload offsets.
    let blue: MaterialRef<'_> = scene.materials().next_back().unwrap();
    assert_eq!(blue.name(), Some("blue-painted"));
    assert_eq!(blue.shading_model(), ShadingModel::Phong);
    assert_eq!(blue.alpha_mode(), AlphaMode::Opaque);
    assert!(!blue.double_sided());
    assert_eq!(
        blue.factors().legacy().unwrap().diffuse(),
        Vec3::new(0.2, 0.3, 0.9)
    );
    assert_eq!(blue.factors().legacy().unwrap().shininess(), 8.0);
    assert_eq!(blue.binding_count(), 1);
    let blue_binding: MaterialBindingRef<'_> = blue.bindings().next().unwrap();
    let blue_texture: TextureRef<'_> = blue_binding.texture();
    let blue_image: ImageRef<'_> = blue_texture.image();
    let blue_sampler: SamplerRef<'_> = blue_texture.sampler();
    assert_eq!(blue_image.encoded_bytes(), BLUE_BYTES);
    assert_eq!(blue_image.media_type(), None);
    assert_eq!(blue_sampler.state(), SamplerState::CANONICAL_DEFAULT);

    let red = scene.materials().next().unwrap();
    assert_eq!(red.name(), Some("red"));
    assert_eq!(red.shading_model(), ShadingModel::Phong);
    assert_eq!(red.alpha_mode(), AlphaMode::Blend);
    assert!(!red.double_sided());
    let factors = red.factors();
    assert_eq!(factors.emissive(), Vec3::new(0.1, 0.2, 0.3));
    assert!(factors.pbr_metallic_roughness().is_none());
    assert!(factors.pbr_specular_glossiness().is_none());
    let legacy = factors.legacy().expect("OBJ retains legacy factors");
    assert_eq!(legacy.diffuse(), Vec3::new(0.8, 0.1, 0.2));
    assert_eq!(legacy.emissive(), Vec3::new(0.1, 0.2, 0.3));
    assert_eq!(legacy.shininess(), 32.0);
    assert_eq!(legacy.opacity(), 0.5);

    let binding = red.bindings().next().unwrap();
    assert_eq!(binding.role(), TextureRole::Diffuse);
    assert_eq!(
        binding.coordinate_source(),
        TextureCoordinateSource::UvSet(0)
    );
    assert_eq!(
        binding.channel_mapping(),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA)
    );
    assert_eq!(
        binding.color_interpretation(),
        ColorInterpretation::SrgbColor
    );
    assert_eq!(binding.transform(), TextureTransform::IDENTITY);
    assert_eq!(binding.strength(), TextureStrength::ONE);
    assert_eq!(binding.blend(), TextureBlendOperation::Replace);
    assert_eq!(binding.source_key(), Some("map_Kd"));
    assert_eq!(binding.texture().image().encoded_bytes(), RED_BYTES);

    let matte = scene.materials().nth(1).unwrap();
    assert_eq!(matte.name(), Some("matte"));
    assert_eq!(matte.binding_count(), 0);
    assert_eq!(matte.bindings().len(), 0);
    assert!(matte.bindings().next().is_none());

    for (mesh, material) in scene.meshes().zip(scene.materials()) {
        assert_eq!(
            mesh.primitives().next().unwrap().material().unwrap().id(),
            material.id()
        );
    }

    assert_eq!(last_material_name(scene), "blue-painted");
    assert_eq!(last_source_key(scene), "map_Kd");
    assert_eq!(last_encoded_bytes(scene), BLUE_BYTES);
    assert!(std::ptr::eq(
        last_encoded_bytes(scene),
        scene.images().next_back().unwrap().encoded_bytes()
    ));
    assert!(std::ptr::eq(
        blue.name().unwrap(),
        scene.material(blue.id()).unwrap().name().unwrap()
    ));
    assert!(std::ptr::eq(
        blue_binding.source_key().unwrap(),
        last_source_key(scene)
    ));
}

#[test]
fn appearance_iterators_and_checked_lookups_are_independent() {
    let report = appearance_report();
    let scene = report.scene();
    let mut forward = scene.materials();
    let mut reverse = scene.materials().rev();
    assert_eq!(forward.len(), 3);
    assert_eq!(reverse.len(), 3);
    let red = forward.next().unwrap();
    let blue = reverse.next().unwrap();
    assert_eq!(forward.len(), 2);
    assert_eq!(reverse.len(), 2);
    assert_eq!(forward.next().unwrap().name(), Some("matte"));
    assert_eq!(reverse.next().unwrap().name(), Some("matte"));
    assert_eq!(forward.next().unwrap().id(), blue.id());
    assert_eq!(reverse.next().unwrap().id(), red.id());
    assert!(forward.next().is_none());
    assert!(reverse.next().is_none());

    let mut bindings = red.bindings();
    let other_binding = red.bindings().next().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings.next().unwrap().texture().id(),
        other_binding.texture().id()
    );
    assert_eq!(bindings.len(), 0);

    assert_eq!(scene.images().len(), scene.image_count());
    assert_eq!(scene.samplers().len(), scene.sampler_count());
    assert_eq!(scene.textures().len(), scene.texture_count());
    for material in scene.materials().rev() {
        assert_eq!(
            scene.material(material.id()).unwrap().name(),
            material.name()
        );
    }
    for texture in scene.textures().rev() {
        assert_eq!(
            scene.texture(texture.id()).unwrap().image().id(),
            texture.image().id()
        );
        let image = texture.image();
        assert!(std::ptr::eq(
            scene.image(image.id()).unwrap().encoded_bytes(),
            image.encoded_bytes()
        ));
        let sampler = texture.sampler();
        assert_eq!(
            scene.sampler(sampler.id()).unwrap().state(),
            sampler.state()
        );
    }
    assert_eq!(
        scene.images().next_back().unwrap().encoded_bytes(),
        BLUE_BYTES
    );
    assert_eq!(scene.images().next().unwrap().encoded_bytes(), RED_BYTES);
    assert_eq!(scene.materials().len(), 3);
}

#[test]
fn appearance_handles_reject_another_scene_generation() {
    let first = appearance_report();
    let second = appearance_report();
    let original = first.scene();
    let foreign = second.scene();
    let material = original.materials().next_back().unwrap().id();
    let image = original.images().next_back().unwrap().id();
    let sampler = original.samplers().next_back().unwrap().id();
    let texture = original.textures().next_back().unwrap().id();
    let expected = |kind| SceneLookupError::GenerationMismatch {
        kind,
        expected: foreign.generation(),
        actual: original.generation(),
    };
    assert_eq!(
        foreign.material(material).unwrap_err(),
        expected(SceneEntityKind::Material)
    );
    assert_eq!(
        foreign.image(image).unwrap_err(),
        expected(SceneEntityKind::Image)
    );
    assert_eq!(
        foreign.sampler(sampler).unwrap_err(),
        expected(SceneEntityKind::Sampler)
    );
    assert_eq!(
        foreign.texture(texture).unwrap_err(),
        expected(SceneEntityKind::Texture)
    );

    drop(first);
    assert!(matches!(
        foreign.material(material),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        foreign.image(image),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        foreign.sampler(sampler),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        foreign.texture(texture),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
}

#[test]
fn scene_without_appearance_has_empty_independent_iterators() {
    let report = Importer::new()
        .expect("default importers register")
        .read_bytes("plain.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
        .expect("untextured OBJ imports");
    let scene = report.scene();
    assert_eq!(scene.material_count(), 0);
    assert_eq!(scene.material_binding_count(), 0);
    assert_eq!(scene.image_count(), 0);
    assert_eq!(scene.sampler_count(), 0);
    assert_eq!(scene.texture_count(), 0);
    assert_eq!(scene.materials().len(), 0);
    assert_eq!(scene.images().len(), 0);
    assert_eq!(scene.samplers().len(), 0);
    assert_eq!(scene.textures().len(), 0);
    assert!(scene.materials().next().is_none());
    assert!(scene.images().next_back().is_none());
    assert!(scene.samplers().next().is_none());
    assert!(scene.textures().next_back().is_none());
    assert!(
        scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .material()
            .is_none()
    );
}
