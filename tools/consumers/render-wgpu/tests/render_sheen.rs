mod common;

use baozi::{
    Color, LightKindCategory, MaterialFactors, PbrMetallicRoughnessFactors, ShadingModel,
    SheenFactors, Vec3,
};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationLimits, EvaluationRequest, FramePolicy, SampleTime, evaluate,
};
use baozi_render_wgpu::{
    Control,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    renderer::{RenderSettings, RenderedFrame, Renderer},
    spatial::LightFrame,
};

fn render(renderer: &Renderer, asset: &ApplicationAsset, light: LightFrame) -> RenderedFrame {
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
            common::camera(),
            RenderSettings::default(),
            &[light],
            Control::default(),
        )
        .unwrap();
    renderer.render(&uploaded, Control::default()).unwrap()
}

fn region(frame: &RenderedFrame, expected: f32, tolerance: f32) {
    for y in 29..33 {
        for x in 29..33 {
            let pixel = frame.pixel(x, y).unwrap();
            for &channel in &pixel[..3] {
                assert!(channel.is_finite() && channel >= 0.0);
                assert!(
                    (channel - expected).abs() < tolerance,
                    "({x},{y}): {channel} != {expected}"
                );
            }
            assert_eq!(pixel[3], 1.0);
        }
    }
    assert!(frame.diagnostics.iter().all(|&code| code == 0));
}

#[test]
#[ignore = "requires explicit named Vulkan/DX12 adapter; missing adapter fails"]
fn bounded_scaling_white_grazing_sheen_is_nonnegative_without_clamping_hdr_radiance() {
    let renderer = common::renderer();
    let mut asset = common::asset(3);
    let normal = [(1.0 - 0.001f64.powi(2)).sqrt(), 0.0, 0.001];
    for vertex in &mut asset.draws[0].vertices {
        vertex.normal = Some(normal);
    }
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = MaterialFactors::empty()
        .with_pbr_metallic_roughness(Some(
            PbrMetallicRoughnessFactors::try_new(Color::WHITE, 0.0, 1.0).unwrap(),
        ))
        .with_sheen(Some(
            SheenFactors::try_new(Vec3::new(1.0, 1.0, 1.0), 0.045).unwrap(),
        ));
    let source = asset.materials[0].factors;
    let parallel = LightFrame {
        position: [0.0; 3],
        outgoing: [0.0, 0.0, -1.0],
        kind: LightKindCategory::Directional,
        tint: [1.0; 3],
        intensity: 1.0,
        range: None,
        cos_inner: 1.0,
        cos_outer: 1.0,
        meters_per_unit: 1.0,
    };
    // Independent raw f=5240.180562185577, NoL=0.001. Raw E=4.497436310601021
    // is R16F 4.49609375: the explicit reciprocal normalization gives 1.1654962849.
    // Output RGB itself stays above one; output clamping would fail this literal.
    let bright = render(&renderer, &asset, parallel);
    region(&bright, 1.1654963, 0.005);
    assert!(bright.pixel(31, 31).unwrap()[0] > 1.1);
    let aligned = LightFrame {
        outgoing: normal.map(|v| -v),
        ..parallel
    };
    // NoV=.001, NoL=1 and NoH=sqrt(.5005): raw Charlie is about 3.843e-74.
    // White sheen reserves the full substrate. The finite device result is zero;
    // the old raw 1-E scaling would instead produce negative base radiance here.
    region(&render(&renderer, &asset, aligned), 0.0, 1e-6);
    assert_eq!(asset.materials[0].factors, source);

    // Base changes cannot leak through a layer whose substrate weight is zero.
    asset.materials[0].factors = source.with_pbr_metallic_roughness(Some(
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
    ));
    region(&render(&renderer, &asset, parallel), 1.1654963, 0.005);
    region(&render(&renderer, &asset, aligned), 0.0, 1e-6);
}
