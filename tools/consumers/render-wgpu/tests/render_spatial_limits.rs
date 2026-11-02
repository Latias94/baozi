mod common;

use baozi::{
    CameraProjection, Color, LightKindCategory, LocalAxes, MaterialFactors,
    PbrMetallicRoughnessFactors, ShadingModel,
};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, SampleTime, evaluate,
};
use baozi_render_wgpu::{
    Control, Error, Result,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    renderer::{RenderSettings, RenderedFrame, Renderer},
    spatial::{CameraInput, IDENTITY, LightFrame},
};

fn render(
    renderer: &Renderer,
    asset: &ApplicationAsset,
    camera: CameraInput,
    light: LightFrame,
) -> Result<RenderedFrame> {
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
    let uploaded = renderer
        .upload(
            &plan,
            camera,
            RenderSettings::default(),
            &[light],
            Control::default(),
        )
        .unwrap();
    renderer.render(&uploaded, Control::default())
}

fn gray_plane() -> ApplicationAsset {
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
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
    ));
    asset
}

fn point(z: f64, meters_per_unit: f64, range: Option<f64>) -> LightFrame {
    LightFrame {
        position: [0.0, 0.0, z],
        outgoing: [0.0, 0.0, -1.0],
        kind: LightKindCategory::Point,
        tint: [1.0; 3],
        intensity: 1.0,
        range,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit,
    }
}

fn region(frame: &RenderedFrame, expected: f32, tolerance: f32) {
    for y in 30..34 {
        for x in 30..34 {
            let pixel = frame.pixel(x, y).unwrap();
            for &actual in &pixel[..3] {
                assert!(actual.is_finite() && actual >= 0.0);
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "({x},{y}): {actual} != {expected}"
                );
            }
            assert_eq!(pixel[3], 1.0);
        }
    }
    assert!(frame.diagnostics.iter().all(|&code| code == 0));
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn physical_point_distance_is_scaled_before_squaring_and_unrepresentable_distance_fails() {
    let renderer = common::renderer();
    let mut asset = gray_plane();
    let ordinary = render(&renderer, &asset, common::camera(), point(1.0, 1.0, None)).unwrap();
    // Independent normal-incidence gray BRDF: .48/pi + .01/pi = .49/pi.
    // The ordinary point's central pixels vary slightly with direction/distance.
    region(&ordinary, 0.15597184, 0.002);

    asset.space.unit_scale_to_meters = Some(1e-20);
    let scaled = render(
        &renderer,
        &asset,
        common::camera(),
        point(1e20, 1e-20, None),
    )
    .unwrap();
    region(&scaled, 0.15597184, 0.002);
    for y in 30..34 {
        for x in 30..34 {
            for (&a, &b) in ordinary.pixel(x, y).unwrap()[..3]
                .iter()
                .zip(&scaled.pixel(x, y).unwrap()[..3])
            {
                assert!(
                    (a - b).abs() < 0.002,
                    "({x},{y}): one-meter {a} != scaled {b}"
                );
            }
        }
    }

    asset.space.unit_scale_to_meters = Some(1.0);
    let distant = point(f64::from(f32::MAX), 1.0, None);
    let error = match render(&renderer, &asset, common::camera(), distant) {
        Err(error) => error,
        Ok(frame) => panic!(
            "expected RenderNumericFailure bit 512 for max-F32 point distance; got success with center {:?} and {} nonzero diagnostics",
            frame.pixel(31, 31),
            frame.diagnostics.iter().filter(|&&code| code != 0).count()
        ),
    };
    assert!(
        matches!(error,Error::RenderNumericFailure {code,..} if code&512!=0),
        "{error:?}"
    );
    // An explicit finite range is a deliberate zero contribution, so physical
    // inverse-square arithmetic is unnecessary and must not reject this case.
    let outside = render(
        &renderer,
        &asset,
        common::camera(),
        LightFrame {
            range: Some(1.0),
            ..distant
        },
    )
    .unwrap();
    region(&outside, 0.0, 0.0);
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn physical_range_fraction_survives_a_subnormal_reciprocal_denominator() {
    let renderer = common::renderer();
    let mut asset = gray_plane();
    let meters = f64::from(f32::MIN_POSITIVE);
    asset.space.unit_scale_to_meters = Some(meters);
    let light = point(f64::from(f32::MAX) / 4.0, meters, Some(f64::from(f32::MAX)));
    // Exactly normal input lanes: distance is about one meter and d/range=.25.
    // Independent cutoff is (1-(1/4)^4)^2=.9922027587890625, not one.
    match render(&renderer, &asset, common::camera(), light) {
        Ok(frame) => region(&frame, 0.15475573, 0.0002),
        Err(error) => assert!(
            matches!(error, Error::RenderNumericFailure { code, .. } if code & 512 != 0),
            "expected a named derived numeric failure: {error:?}"
        ),
    }
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn perspective_view_normalizes_a_component_scaled_world_delta() {
    let renderer = common::renderer();
    let mut asset = gray_plane();
    for vertex in &mut asset.draws[0].vertices {
        vertex.position = vertex.position.map(|v| v * 1e20);
    }
    asset.materials[0].factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(
        PbrMetallicRoughnessFactors::try_new(Color::WHITE, 1.0, 1.0).unwrap(),
    ));
    let mut world = IDENTITY;
    world[3][2] = 1e20;
    let camera = CameraInput::Explicit {
        projection: CameraProjection::perspective(
            LocalAxes::STANDARD,
            std::f32::consts::FRAC_PI_2,
            Some(1.0),
            1e19,
            Some(2e20),
        )
        .unwrap(),
        world,
    };
    let light = LightFrame {
        kind: LightKindCategory::Directional,
        ..point(0.0, 1.0, None)
    };
    let frame = render(&renderer, &asset, camera, light).unwrap();
    // Independent white-metal roughness-one BRDF: 1/(4*pi). Overflowing
    // normalize(camera-world) used to yield approximately twice this value.
    region(&frame, 0.07957747, 0.002);
}
