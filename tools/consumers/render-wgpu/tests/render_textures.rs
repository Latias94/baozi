mod common;
use baozi::*;
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, SampleTime, Sampler,
    Texture, UvStream, evaluate,
};
use baozi_render_wgpu::{
    Control, Error,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    material::*,
    renderer::*,
    spatial::LightFrame,
};

fn render(
    renderer: &Renderer,
    asset: &ApplicationAsset,
    lights: &[LightFrame],
    split: bool,
) -> RenderedFrame {
    let evaluation = evaluate(
        asset,
        None,
        SampleTime::Declared(0.0),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeight,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )
    .unwrap();
    let frames = evaluated_frames(&evaluation).unwrap();
    let limits = if split {
        DeviceLimits {
            max_vertices_per_chunk: 3,
            ..Default::default()
        }
    } else {
        DeviceLimits::default()
    };
    let plan =
        DevicePlan::prepare(asset, &evaluation, &frames, limits, Control::default()).unwrap();
    let uploaded = renderer
        .upload(
            &plan,
            common::camera(),
            RenderSettings::default(),
            lights,
            Control::default(),
        )
        .unwrap();
    renderer.render(&uploaded, Control::default()).unwrap()
}
fn center(frame: &RenderedFrame) -> [f32; 4] {
    frame.pixel(31, 31).unwrap()
}
fn region(frame: &RenderedFrame, expected: [f32; 4], tolerance: f32) {
    for y in 29..33 {
        for x in 29..33 {
            for (lane, (actual, expected)) in frame
                .pixel(x, y)
                .unwrap()
                .into_iter()
                .zip(expected)
                .enumerate()
            {
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "{x},{y}:{lane}: {actual} != {expected}"
                );
            }
        }
    }
}
fn white(asset: &mut ApplicationAsset) {
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
        PbrMetallicRoughnessFactors::try_new(Color::WHITE, 0.0, 1.0).unwrap(),
    ));
}
fn uv(asset: &mut ApplicationAsset, set: u32, values: Vec<[f64; 2]>) {
    asset.draws[0].uv_streams.push(UvStream {
        set,
        domain: AttributeDomain::Vertex,
        values,
    });
    asset.draws[0].uv_streams.sort_by_key(|s| s.set);
}
fn light(outgoing: [f64; 3]) -> LightFrame {
    LightFrame {
        position: [0.0; 3],
        outgoing,
        kind: LightKindCategory::Directional,
        tint: [1.0; 3],
        intensity: 1.0,
        range: None,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit: 1.0,
    }
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn uv1_transform_swizzle_strength_alpha_and_actual_jpeg_upload() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    white(&mut asset);
    uv(&mut asset, 1, vec![[0.25, 0.125]; 3]);
    let texture = common::image(
        &mut asset,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
        2,
        2,
    );
    let mut binding = common::binding(
        TextureRole::BaseColor,
        texture,
        1,
        TextureChannelSwizzle::RGBA,
        ColorInterpretation::LinearColor,
    );
    binding.transform = TextureTransform::try_new(
        Vec2::new(0.125, 0.25),
        std::f32::consts::FRAC_PI_2,
        Vec2::new(2.0, 0.5),
    )
    .unwrap();
    asset.materials[0].bindings.push(binding);
    region(
        &render(&renderer, &asset, &[], false),
        [0.0, 0.0, 1.0, 1.0],
        0.002,
    );
    // The blue result requires transformed UV1; UV0 is absent and the original UV hits red.
    asset.materials[0].bindings[0].transform = TextureTransform::IDENTITY;
    asset.images[0].encoded_bytes = common::png(1, 1, &[51, 102, 204, 153]);
    asset.materials[0].bindings[0].strength = TextureStrength::try_new(0.25).unwrap();
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
        PbrMetallicRoughnessFactors::try_new(
            Color {
                r: 0.5,
                g: 0.25,
                b: 0.125,
                a: 0.5,
            },
            0.0,
            1.0,
        )
        .unwrap(),
    ));
    asset.draws[0]
        .vertices
        .iter_mut()
        .for_each(|v| v.color = Some([1.0, 1.0, 1.0, 0.8]));
    region(
        &render(&renderer, &asset, &[], false),
        [0.025, 0.025, 0.025, 1.0],
        0.002,
    );
    // 0.6*0.5*0.8 equals 0.24 within F32. Use an unambiguous adjacent policy boundary.
    asset.materials[0].alpha_mode = AlphaMode::mask(0.239).unwrap();
    region(
        &render(&renderer, &asset, &[], false),
        [0.025, 0.025, 0.025, 1.0],
        0.002,
    );
    asset.materials[0].alpha_mode = AlphaMode::mask(0.241).unwrap();
    region(
        &render(&renderer, &asset, &[], false),
        [0.0, 0.0, 0.0, 1.0],
        0.002,
    );
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
        .encode(&[128, 128, 128], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    asset.images[0].encoded_bytes = jpeg;
    asset.images[0].media_type = Some("image/jpeg".into());
    asset.materials[0].alpha_mode = AlphaMode::Opaque;
    white(&mut asset);
    asset.materials[0].bindings[0].strength = TextureStrength::try_new(1.0).unwrap();
    asset.draws[0]
        .vertices
        .iter_mut()
        .for_each(|v| v.color = None);
    region(
        &render(&renderer, &asset, &[], false),
        [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0],
        0.01,
    );
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn shared_image_color_data_and_distinct_sampler_state_reach_gpu() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    white(&mut asset);
    uv(&mut asset, 1, vec![[0.75, 0.75]; 3]);
    let first = common::image(&mut asset, &[51, 102, 204, 255], 1, 1);
    let sampler = asset.samplers.len();
    asset.samplers.push(Sampler {
        name: None,
        state: SamplerState::new(
            TexelFilter::Linear,
            TexelFilter::Linear,
            MipmapFilter::Linear,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::ClampToEdge,
            TextureAddressMode::Repeat,
        ),
    });
    let second = asset.textures.len();
    asset.textures.push(Texture {
        name: None,
        image: 0,
        sampler,
    });
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
        PbrMetallicRoughnessFactors::try_new(Color::WHITE, 0.5, 0.25).unwrap(),
    ));
    asset.materials[0].bindings = vec![
        common::binding(
            TextureRole::BaseColor,
            first,
            1,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::SrgbColor,
        ),
        common::binding(
            TextureRole::MetallicRoughness,
            second,
            1,
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green]).unwrap(),
            ColorInterpretation::Data,
        ),
    ];
    let base = [
        srgb_to_linear(0.2),
        srgb_to_linear(0.4),
        srgb_to_linear(0.8),
    ];
    let expected = mr_brdf(base, 0.4, 0.1, 1.0, 1.0, 1.0, 1.0);
    let result = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], false);
    for (actual, expected) in center(&result)[..3].iter().zip(expected) {
        assert!((f64::from(*actual) - expected).abs() < 0.003 + expected.abs() * 0.02);
    }
    // Observable sampler difference on the same encoded image, preserving texture identity.
    asset.images[0].encoded_bytes = common::png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
    asset.materials[0].shading_model = ShadingModel::Unlit;
    asset.materials[0].bindings.truncate(1);
    asset.draws[0].uv_streams[0].values.fill([0.5, 0.5]);
    let nearest = render(&renderer, &asset, &[], false);
    asset.materials[0].bindings[0].texture = second;
    let linear = render(&renderer, &asset, &[], false);
    assert!(center(&nearest)[2] > 0.9);
    assert!((center(&linear)[0] - 0.5).abs() < 0.01 && (center(&linear)[2] - 0.5).abs() < 0.01);
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn four_active_uv_sets_preserve_all_packed_varying_lanes() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty()
        .with_pbr_metallic_roughness(Some(
            PbrMetallicRoughnessFactors::try_new(
                Color {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
                0.0,
                1.0,
            )
            .unwrap(),
        ))
        .with_emission(EmissionFactors::try_new(Vec3::new(0.25, 0.5, 0.75), None).unwrap())
        .with_clearcoat(Some(ClearcoatFactors::try_new(0.5, 1.0).unwrap()));
    let specifications = [
        (
            1u16,
            [0.25, 0.25],
            TextureRole::BaseColor,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::LinearColor,
        ),
        (
            3,
            [0.75, 0.25],
            TextureRole::MetallicRoughness,
            TextureChannelSwizzle::try_new(&[TextureChannel::Red, TextureChannel::Green]).unwrap(),
            ColorInterpretation::Data,
        ),
        (
            5,
            [0.25, 0.75],
            TextureRole::Emissive,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::LinearColor,
        ),
        (
            7,
            [0.75, 0.75],
            TextureRole::ClearcoatRoughness,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
        ),
    ];
    for (pixel, (set, coordinate, role, swizzle, transfer)) in
        specifications.into_iter().enumerate()
    {
        uv(&mut asset, u32::from(set), vec![coordinate; 3]);
        // Only this set's own quadrant is white. Substituting another packed
        // lane changes base, roughness, emission, or clearcoat roughness.
        let mut rgba = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
        rgba[pixel * 4..pixel * 4 + 3].fill(255);
        let texture = common::image(&mut asset, &rgba, 2, 2);
        asset.materials[0]
            .bindings
            .push(common::binding(role, texture, set, swizzle, transfer));
    }
    let evaluation = common::evaluated(&asset);
    let frames = evaluated_frames(&evaluation).unwrap();
    let plan = DevicePlan::prepare(
        &asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(plan.materials()[0].active_uv_sets, [1, 3, 5, 7]);
    let uploaded = renderer
        .upload(
            &plan,
            common::camera(),
            RenderSettings::default(),
            &[light([0.0, 0.0, -1.0])],
            Control::default(),
        )
        .unwrap();
    let frame = renderer.render(&uploaded, Control::default()).unwrap();
    // Independent half-coat gray .15444395677637524 plus .98 * emission.
    region(&frame, [0.39944395, 0.6444439, 0.88944393, 1.0], 0.002);
    assert!(frame.diagnostics.iter().all(|&code| code == 0));
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn mirrored_base_and_independent_coat_normal_frames_split_stably() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    white(&mut asset);
    uv(&mut asset, 1, vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
    uv(&mut asset, 2, vec![[0.0, 0.0], [-1.0, 0.0], [-0.5, 1.0]]);
    let normal = common::image(&mut asset, &[255, 128, 128, 255], 1, 1);
    asset.materials[0].bindings.push(common::binding(
        TextureRole::Normal,
        normal,
        1,
        TextureChannelSwizzle::RGB,
        ColorInterpretation::Data,
    ));
    let positive = render(&renderer, &asset, &[light([-1.0, 0.0, 0.0])], false);
    asset.materials[0].bindings[0].uv_set = 2;
    let mirrored = render(&renderer, &asset, &[light([-1.0, 0.0, 0.0])], false);
    assert!(center(&positive)[0] > 0.1);
    assert!(center(&mirrored)[0] < 0.001);
    asset.materials[0].bindings[0].uv_set = 1;
    let neutral = common::image(&mut asset, &[128, 128, 255, 255], 1, 1);
    asset.materials[0].bindings.push(common::binding(
        TextureRole::ClearcoatNormal,
        neutral,
        2,
        TextureChannelSwizzle::RGB,
        ColorInterpretation::Data,
    ));
    asset.materials[0].factors = MaterialFactors::empty()
        .with_pbr_metallic_roughness(Some(
            PbrMetallicRoughnessFactors::try_new(
                Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
                1.0,
                1.0,
            )
            .unwrap(),
        ))
        .with_clearcoat(Some(ClearcoatFactors::try_new(0.5, 1.0).unwrap()));
    let independent = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], false);
    assert!(center(&independent)[0] > 0.0012);
    asset.materials[0].bindings[1].texture = normal;
    let changed_coat = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], false);
    assert!(center(&changed_coat)[0] > center(&independent)[0] * 5.0);
    // Add a mirrored triangle to force a real stable whole-triangle packet boundary.
    let old = asset.draws[0].vertices.clone();
    asset.draws[0].vertices.extend(old);
    asset.draws[0].indices.extend([3, 4, 5]);
    for stream in &mut asset.draws[0].uv_streams {
        let mut values = stream.values.clone();
        values.iter_mut().for_each(|uv| uv[0] = -uv[0]);
        stream.values.extend(values);
    }
    let unsplit = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], false);
    let split = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], true);
    assert_eq!(split.linear_rgba, unsplit.linear_rgba);
}

#[test]
fn static_lit_requires_explicit_evaluator_frames_and_degenerate_uv_fails() {
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    let evaluation = common::evaluated(&asset);
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &[],
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::ExplicitFramesRequired { .. })
    ));
    let frames = evaluated_frames(&evaluation).unwrap();
    assert!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        )
        .is_ok()
    );
    uv(&mut asset, 1, vec![[0.0; 2]; 3]);
    let normal = common::image(&mut asset, &[128, 128, 255, 255], 1, 1);
    asset.materials[0].bindings.push(common::binding(
        TextureRole::Normal,
        normal,
        1,
        TextureChannelSwizzle::RGB,
        ColorInterpretation::Data,
    ));
    let evaluation = common::evaluated(&asset);
    let frames = evaluated_frames(&evaluation).unwrap();
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::DegenerateNormalMapFrame { .. })
    ));
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn mask_equality_opaque_backface_and_instance_reflection() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    white(&mut asset);
    for v in &mut asset.draws[0].vertices {
        v.color = Some([1.0, 0.0, 0.0, 0.375]);
    }
    asset.materials[0].alpha_mode = AlphaMode::mask(0.375).unwrap();
    region(
        &render(&renderer, &asset, &[], false),
        [1.0, 0.0, 0.0, 1.0],
        0.002,
    );
    for v in &mut asset.draws[0].vertices {
        v.color.as_mut().unwrap()[3] = 0.374;
    }
    region(
        &render(&renderer, &asset, &[], false),
        [0.0, 0.0, 0.0, 1.0],
        0.002,
    );
    asset.materials[0].alpha_mode = AlphaMode::Opaque;
    asset.draws[0].indices = vec![0, 2, 1];
    region(
        &render(&renderer, &asset, &[], false),
        [0.0, 0.0, 0.0, 1.0],
        0.002,
    );
    asset.materials[0].double_sided = true;
    region(
        &render(&renderer, &asset, &[], false),
        [1.0, 0.0, 0.0, 1.0],
        0.002,
    );
    asset.materials[0].double_sided = false;
    asset.draws[0].indices = vec![0, 1, 2];
    asset.nodes[0].transform = AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    })
    .unwrap();
    region(
        &render(&renderer, &asset, &[], false),
        [1.0, 0.0, 0.0, 1.0],
        0.002,
    );
}

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn zero_transmission_and_thickness_keep_retained_images_and_uvs_inactive_on_gpu() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty()
        .with_pbr_metallic_roughness(Some(
            PbrMetallicRoughnessFactors::try_new(
                Color {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
                0.0,
                1.0,
            )
            .unwrap(),
        ))
        .with_transmission(Some(TransmissionFactors::try_new(0.0).unwrap()))
        .with_thickness(Some(ThicknessFactors::try_new(0.0).unwrap()));
    let texture = common::image(&mut asset, &[255, 255, 255, 255], 1, 1);
    asset.images[0].encoded_bytes = b"inactive undecodable payload".to_vec();
    asset.materials[0].bindings = [TextureRole::Transmission, TextureRole::Thickness]
        .into_iter()
        .map(|role| {
            common::binding(
                role,
                texture,
                7,
                TextureChannelSwizzle::RED,
                ColorInterpretation::Data,
            )
        })
        .collect();
    let original = asset.materials[0].factors;
    // UV7 is absent and the retained encoded image cannot decode. Neither belongs
    // to the active closure of an explicit-zero transmission/thickness group.
    let frame = render(&renderer, &asset, &[light([0.0, 0.0, -1.0])], false);
    region(&frame, [0.15597184, 0.15597184, 0.15597184, 1.0], 0.002);
    assert!(frame.diagnostics.iter().all(|&code| code == 0));
    assert_eq!(asset.materials[0].factors, original);
    assert!(asset.draws[0].uv_streams.is_empty());
}
