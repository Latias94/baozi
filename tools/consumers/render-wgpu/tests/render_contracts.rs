mod common;

use baozi::*;
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, Light, Node, SampleTime,
    UvStream, evaluate,
};
use baozi_render_wgpu::{
    Control, Error,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    renderer::{GpuAsset, RenderSettings, RenderedFrame, Renderer},
    spatial::LightFrame,
};

fn lit_plane(base: Color, metallic: f32) -> ApplicationAsset {
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
        PbrMetallicRoughnessFactors::try_new(base, metallic, 1.0).unwrap(),
    ));
    asset
}

fn constant_uv(asset: &mut ApplicationAsset) {
    asset.draws[0].uv_streams.push(UvStream {
        set: 7,
        domain: AttributeDomain::Vertex,
        values: vec![[0.25, 0.25]; 3],
    });
}

fn directional() -> LightFrame {
    LightFrame {
        position: [0.0; 3],
        outgoing: [0.0, 0.0, -1.0],
        kind: LightKindCategory::Directional,
        tint: [1.0; 3],
        intensity: 1.0,
        range: None,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit: 1.0,
    }
}

fn upload(
    renderer: &Renderer,
    asset: &ApplicationAsset,
    lights: &[LightFrame],
    side: u32,
) -> GpuAsset {
    let evaluation = evaluate(
        asset,
        None,
        SampleTime::Declared(0.0),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeightNormalsOnly,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )
    .unwrap();
    let frames = evaluated_frames(&evaluation).unwrap();
    let plan = DevicePlan::prepare(
        asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )
    .unwrap();
    renderer
        .upload(
            &plan,
            common::camera(),
            RenderSettings {
                width: side,
                height: side,
                max_lights: 2,
                ..RenderSettings::default()
            },
            lights,
            Control::default(),
        )
        .unwrap()
}

fn render(
    renderer: &Renderer,
    asset: &ApplicationAsset,
    lights: &[LightFrame],
    side: u32,
) -> RenderedFrame {
    renderer
        .render(&upload(renderer, asset, lights, side), Control::default())
        .unwrap()
}

fn pixel(frame: &RenderedFrame, xy: [u32; 2], expected: [f32; 4], tolerance: f32) {
    for (lane, (actual, expected)) in frame
        .pixel(xy[0], xy[1])
        .unwrap()
        .into_iter()
        .zip(expected)
        .enumerate()
    {
        assert!(actual.is_finite() && actual >= 0.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "{xy:?} lane {lane}: {actual} != {expected}, tolerance {tolerance}"
        );
    }
    assert!(frame.diagnostics.iter().all(|&code| code == 0));
}

fn region(frame: &RenderedFrame, expected: [f32; 4], tolerance: f32) {
    assert_eq!((frame.width, frame.height), (16, 16));
    for y in 6..10 {
        for x in 6..10 {
            pixel(frame, [x, y], expected, tolerance);
        }
    }
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn covered_hdr_failure_survives_overdraw_and_a_fresh_render_is_clean() {
    let renderer = common::renderer();
    let black = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let mut asset = lit_plane(black, 1.0);
    for vertex in &mut asset.draws[0].vertices {
        vertex.position[0] *= 0.5;
        vertex.position[1] *= 0.5;
    }
    asset.materials[0].factors = asset.materials[0]
        .factors
        .with_emission(EmissionFactors::try_new(Vec3::new(70_000.0, 0.0, 0.0), None).unwrap());

    let mut cover = common::asset(3);
    let mut draw = cover.draws.remove(0);
    draw.primitive = 1;
    draw.material = MaterialAssignment::Authored(1);
    for vertex in &mut draw.vertices {
        vertex.position[2] = 0.25;
    }
    asset.draws.push(draw);
    asset.materials.push(cover.materials.remove(0));

    // Both packets are opaque and submitted in source order. The larger, closer
    // second triangle covers every failing fragment, including its diagnostic texel.
    // 70000 is valid finite F32 HDR emission, but exceeds the 65504 F16 output limit.
    let uploaded = upload(&renderer, &asset, &[], 16);
    let error = match renderer.render(&uploaded, Control::default()) {
        Err(error) => error,
        Ok(_) => panic!("covered HDR output must retain its numeric failure"),
    };
    assert!(
        matches!(
            error,
            Error::RenderNumericFailure {
                pixel: usize::MAX,
                code: 64
            }
        ),
        "expected the persistent fragment failure word: {error:?}"
    );

    asset.materials[0].factors = asset.materials[0]
        .factors
        .with_emission(EmissionFactors::try_new(Vec3::new(1.0, 0.0, 0.0), None).unwrap());
    let recovered = render(&renderer, &asset, &[], 16);
    region(&recovered, [0.25, 0.5, 0.75, 1.0], 0.0);

    // A legal nonzero rear sentinel and cover alone must produce identical whole
    // frames. This witnesses full coverage rather than checking only the center.
    asset.draws.remove(0);
    let cover_only = render(&renderer, &asset, &[], 16);
    assert_eq!(recovered.linear_rgba, cover_only.linear_rgba);
    assert_eq!(recovered.diagnostics, cover_only.diagnostics);
    // Fresh upload/render recovery is observable here; the public immutable
    // GpuAsset API cannot replace the failing material in the same failure buffer.
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn sampled_emission_swizzle_transfer_strength_and_zero_indirect_ao() {
    let renderer = common::renderer();
    let gray = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };
    let mut asset = lit_plane(gray, 0.0);
    constant_uv(&mut asset);
    asset.materials[0].factors = asset.materials[0]
        .factors
        .with_emission(EmissionFactors::try_new(Vec3::new(0.5, 0.25, 0.75), Some(2.0)).unwrap());
    let texture = common::image(
        &mut asset,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 32, 64, 128, 192,
        ],
        2,
        2,
    );
    let mut emission = common::binding(
        TextureRole::Emissive,
        texture,
        7,
        TextureChannelSwizzle::try_new(&[
            TextureChannel::Blue,
            TextureChannel::Zero,
            TextureChannel::One,
        ])
        .unwrap(),
        ColorInterpretation::SrgbColor,
    );
    emission.transform =
        TextureTransform::try_new(Vec2::new(0.5, 0.5), 0.0, Vec2::new(1.0, 1.0)).unwrap();
    emission.strength = TextureStrength::try_new(0.5).unwrap();
    asset.materials[0].bindings.push(emission);
    let ao_texture = common::image(&mut asset, &[17, 34, 51, 68], 1, 1);
    let mut ao = common::binding(
        TextureRole::Occlusion,
        ao_texture,
        7,
        TextureChannelSwizzle::try_new(&[TextureChannel::Alpha]).unwrap(),
        ColorInterpretation::Data,
    );
    ao.strength = TextureStrength::try_new(0.75).unwrap();
    asset.materials[0].bindings.push(ao);

    // T(128/255)=.21586050011389926. Effective emission is raw * 2,
    // multiplied once by binding strength .5 and the (B,Zero,One) sample.
    region(
        &render(&renderer, &asset, &[], 16),
        [0.10793025, 0.0, 0.75, 1.0],
        0.00075,
    );
    // At N=L=V, gray MR contributes .49/pi=.15597184423005744.
    let dark_ao = render(&renderer, &asset, &[directional()], 16);
    region(&dark_ao, [0.2639021, 0.15597184, 0.9059718, 1.0], 0.00075);
    asset.materials[0].bindings[1].channel_mapping =
        TextureChannelSwizzle::try_new(&[TextureChannel::One]).unwrap();
    let white_ao = render(&renderer, &asset, &[directional()], 16);
    // AO changes from .45 to 1; only indirect light would be affected, and this
    // profile has none. This cannot claim observable AO channel decoding.
    assert_eq!(dark_ao.linear_rgba, white_ao.linear_rgba);
    assert_eq!(dark_ao.diagnostics, white_ao.diagnostics);
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn sampled_clearcoat_factor_and_roughness_use_independent_transforms_and_channels() {
    let renderer = common::renderer();
    let black = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let mut asset = lit_plane(black, 1.0);
    constant_uv(&mut asset);
    asset.materials[0].factors = asset.materials[0]
        .factors
        .with_clearcoat(Some(ClearcoatFactors::try_new(0.5, 0.5).unwrap()));
    let texture = common::image(
        &mut asset,
        &[
            17, 34, 51, 68, 85, 102, 119, 136, 11, 33, 55, 153, 22, 44, 204, 66,
        ],
        2,
        2,
    );
    let mut coat = common::binding(
        TextureRole::Clearcoat,
        texture,
        7,
        TextureChannelSwizzle::try_new(&[TextureChannel::Alpha]).unwrap(),
        ColorInterpretation::Data,
    );
    coat.transform =
        TextureTransform::try_new(Vec2::new(0.0, 0.5), 0.0, Vec2::new(1.0, 1.0)).unwrap();
    let mut roughness = common::binding(
        TextureRole::ClearcoatRoughness,
        texture,
        7,
        TextureChannelSwizzle::try_new(&[TextureChannel::Blue]).unwrap(),
        ColorInterpretation::Data,
    );
    roughness.transform =
        TextureTransform::try_new(Vec2::new(0.5, 0.5), 0.0, Vec2::new(1.0, 1.0)).unwrap();
    asset.materials[0].bindings = vec![coat, roughness];

    // Black metal contributes zero here. Coat=.5*(153/255)=.3, r=.5*(204/255)=.4.
    // N=L=V gives w=.04*.3 and coat radiance w/(4*pi*r^4).
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.03730194, 0.03730194, 0.03730194, 1.0],
        0.0001,
    );
    asset.materials[0].bindings[1].channel_mapping =
        TextureChannelSwizzle::try_new(&[TextureChannel::One]).unwrap();
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.015278875, 0.015278875, 0.015278875, 1.0],
        0.0001,
    );
    asset.materials[0].bindings[0].channel_mapping =
        TextureChannelSwizzle::try_new(&[TextureChannel::Zero]).unwrap();
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.0, 0.0, 0.0, 1.0],
        0.0,
    );
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn sampled_sheen_color_and_roughness_match_independent_charlie_literals() {
    let renderer = common::renderer();
    let black = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let mut asset = lit_plane(black, 1.0);
    constant_uv(&mut asset);
    for vertex in &mut asset.draws[0].vertices {
        vertex.normal = Some([3.0f64.sqrt() * 0.5, 0.0, 0.5]);
    }
    asset.materials[0].factors = asset.materials[0].factors.with_sheen(Some(
        SheenFactors::try_new(Vec3::new(0.5, 0.75, 0.25), 1.0).unwrap(),
    ));
    let texture = common::image(
        &mut asset,
        &[
            255, 34, 0, 255, 64, 128, 192, 51, 17, 153, 85, 238, 221, 68, 119, 170,
        ],
        2,
        2,
    );
    let mut color = common::binding(
        TextureRole::SheenColor,
        texture,
        7,
        TextureChannelSwizzle::try_new(&[
            TextureChannel::Blue,
            TextureChannel::Red,
            TextureChannel::One,
        ])
        .unwrap(),
        ColorInterpretation::SrgbColor,
    );
    color.transform =
        TextureTransform::try_new(Vec2::new(0.5, 0.0), 0.0, Vec2::new(1.0, 1.0)).unwrap();
    color.strength = TextureStrength::try_new(0.5).unwrap();
    let mut roughness = common::binding(
        TextureRole::SheenRoughness,
        texture,
        7,
        TextureChannelSwizzle::try_new(&[TextureChannel::Green]).unwrap(),
        ColorInterpretation::Data,
    );
    roughness.transform =
        TextureTransform::try_new(Vec2::new(0.0, 0.5), 0.0, Vec2::new(1.0, 1.0)).unwrap();
    asset.materials[0].bindings = vec![color, roughness];

    // V=L=+Z gives NoV=NoL=NoH=.5 and VoH=1, so black metal is exactly zero.
    // Effective sRGB sheen=(.13177878142645327,.019226046890266215,.125).
    // Independent Charlie*NoL is .07765892575290882 for r=153/255=.6;
    // raw LUT energies are below one here, so reciprocal normalization is one.
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.010233799, 0.0014930741, 0.009707366, 1.0],
        0.00005,
    );
    asset.materials[0].bindings[1].channel_mapping =
        TextureChannelSwizzle::try_new(&[TextureChannel::One]).unwrap();
    // At r=1, independent Charlie*NoL=.08543123457778733.
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.011258024, 0.0016425049, 0.010678904, 1.0],
        0.00005,
    );
    asset.materials[0].bindings[0].color_interpretation = ColorInterpretation::LinearColor;
    region(
        &render(&renderer, &asset, &[directional()], 16),
        [0.016081173, 0.0080405865, 0.010678904, 1.0],
        0.00005,
    );
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn spot_equal_cone_edges_smooth_cone_and_range_boundaries_reach_the_shader() {
    let renderer = common::renderer();
    let gray = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };
    let mut asset = lit_plane(gray, 0.0);
    asset.space.unit_scale_to_meters = Some(0.5);
    let spot = LightFrame {
        position: [0.0, 0.0, 2.0],
        kind: LightKindCategory::Spot,
        cos_inner: 1.0,
        cos_outer: 0.5,
        meters_per_unit: 0.5,
        ..directional()
    };
    let hard = LightFrame {
        cos_inner: 0.6,
        cos_outer: 0.6,
        ..spot
    };
    let below = 0.6 - 1.0 / 1024.0;
    let cases = [
        ("equal cones, inside", hard, 0.15597184),
        (
            "equal cones, equality",
            LightFrame {
                outgoing: [0.8, 0.0, -0.6],
                ..hard
            },
            0.15597184,
        ),
        (
            "equal cones, outside",
            LightFrame {
                outgoing: [(1.0f64 - below * below).sqrt(), 0.0, -below],
                ..hard
            },
            0.0,
        ),
        ("smooth inner edge", spot, 0.15597184),
        (
            "smooth midpoint",
            LightFrame {
                outgoing: [(1.0f64 - 0.75 * 0.75).sqrt(), 0.0, -0.75],
                ..spot
            },
            0.03899296,
        ),
        (
            "smooth outer edge",
            LightFrame {
                outgoing: [3.0f64.sqrt() * 0.5, 0.0, -0.5],
                ..spot
            },
            0.0,
        ),
        (
            "inside range",
            LightFrame {
                range: Some(4.0),
                ..spot
            },
            0.13708463,
        ),
        (
            "at range",
            LightFrame {
                range: Some(2.0),
                ..spot
            },
            0.0,
        ),
        (
            "beyond range",
            LightFrame {
                range: Some(1.0),
                ..spot
            },
            0.0,
        ),
    ];
    // The sole pixel is exactly at world (0,0,0). Distance=2 scene units=1m,
    // so incident intensity is one. Smooth midpoint multiplies .49/pi by .25;
    // range 4 multiplies it by (1-(2/4)^4)^2=225/256. All black cases are exact.
    for (name, light, expected) in cases {
        let frame = render(&renderer, &asset, &[light], 1);
        let tolerance = if expected == 0.0 { 0.0 } else { 0.0002 };
        let actual = frame.pixel(0, 0).unwrap();
        for &channel in &actual[..3] {
            assert!(
                channel.is_finite() && (channel - expected).abs() <= tolerance,
                "{name}: {channel} != {expected}, tolerance {tolerance}"
            );
        }
        assert_eq!(actual[3], 1.0, "{name}");
        assert!(frame.diagnostics.iter().all(|&code| code == 0), "{name}");
    }
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn shared_spot_definition_uses_reflected_local_axes_placements_and_scene_units() {
    let renderer = common::renderer();
    let gray = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };
    let mut asset = lit_plane(gray, 0.0);
    asset.space.unit_scale_to_meters = Some(0.5);
    let axes = LocalAxes::try_new(Axis::NegativeZ, Axis::PositiveY, Axis::PositiveX).unwrap();
    asset.lights.push(Light {
        name: None,
        definition: LightDefinition::try_from_scaled_linear_srgb(
            LightKind::spot(None, 0.0, std::f32::consts::FRAC_PI_4).unwrap(),
            axes,
            Color::WHITE,
            1.0,
        )
        .unwrap(),
    });
    for z in [2.0, 4.0] {
        let node = asset.nodes.len();
        asset.nodes.push(Node {
            name: None,
            transform: AuthoredTransform::try_matrix(Mat4 {
                cols: [
                    [0.0, 0.0, -1.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, z, 1.0],
                ],
            })
            .unwrap(),
            children: vec![],
            camera: None,
            light: Some(0),
        });
        asset.roots.push(node);
    }
    // Intrinsic forward +X becomes world -Z. Right -Z produces a reflected
    // complete light frame; neither placement changes the shared definition.
    // Physical distances 1m and 2m give (1+1/4)*(.49/pi).
    pixel(
        &render(&renderer, &asset, &[], 1),
        [0, 0],
        [0.19496481, 0.19496481, 0.19496481, 1.0],
        0.0003,
    );
    asset.space.unit_scale_to_meters = Some(1.0);
    // Distances now 2m and 4m: (1/4+1/16)*(.49/pi), exactly one quarter as bright.
    pixel(
        &render(&renderer, &asset, &[], 1),
        [0, 0],
        [0.0487412, 0.0487412, 0.0487412, 1.0],
        0.0003,
    );
}
