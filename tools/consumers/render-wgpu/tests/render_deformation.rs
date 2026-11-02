mod common;
use baozi::*;
use baozi_asset_consumer::*;
use baozi_render_wgpu::{
    Control,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    material::mr_brdf,
    renderer::*,
    spatial::LightFrame,
};

#[test]
#[ignore = "requires explicit named adapter; missing adapter fails"]
fn morph_lit_and_normal_map_readback_uses_evaluated_world_frame() {
    let renderer = common::renderer();
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
    asset.mesh_morph_weights[0] = vec![0.0];
    let normal = 1.0 / 2.0f32.sqrt();
    // Position deltas are z=y, so weight one yields plane z=y and N=(0,-1,1)/sqrt(2).
    asset.draws[0].morphs = vec![
        MorphStream {
            target: 0,
            semantic: MorphDeltaSemantic::Position,
            values: asset.draws[0]
                .vertices
                .iter()
                .map(|v| [0.0, 0.0, v.position[1] as f32])
                .collect(),
        },
        MorphStream {
            target: 0,
            semantic: MorphDeltaSemantic::Normal,
            values: vec![[0.0, -normal, normal - 1.0]; 3],
        },
    ];
    asset.draws[0].uv_streams = vec![UvStream {
        set: 1,
        domain: AttributeDomain::Vertex,
        values: vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
    }];
    let texture = common::image(&mut asset, &[128, 128, 255, 255], 1, 1);
    asset.materials[0].bindings.push(common::binding(
        TextureRole::Normal,
        texture,
        1,
        TextureChannelSwizzle::RGB,
        ColorInterpretation::Data,
    ));
    let light = LightFrame {
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
    let mut center_values = Vec::new();
    for weight in [0.0, 0.5, 1.0] {
        asset.mesh_morph_weights[0][0] = weight as f32;
        let evaluation = evaluate(
            &asset,
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
        let raw = [
            0.0,
            -f64::from(normal) * weight,
            1.0 + f64::from(normal - 1.0) * weight,
        ];
        let length = (raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
        let expected_normal = raw.map(|v| v / length);
        for vertex in frames[0].normals {
            for (actual, expected) in vertex.iter().zip(expected_normal) {
                assert!((actual - expected).abs() < 1e-12);
            }
        }
        for (source, world) in asset.draws[0]
            .vertices
            .iter()
            .zip(&evaluation.draws[0].world_positions)
        {
            assert!((world[2] - source.position[1] * weight).abs() < 1e-12);
        }
        let plan = DevicePlan::prepare(
            &asset,
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
        let frame = renderer.render(&uploaded, Control::default()).unwrap();
        // 128 is not an exact neutral encoded X/Y. Independently apply that tangent sample.
        let encoded = 1.0 / 255.0;
        let n = expected_normal;
        let t = [1.0, 0.0, 0.0];
        let bitangent = [0.0, n[2], -n[1]];
        let raw: [f64; 3] = std::array::from_fn(|i| n[i] + encoded * (t[i] + bitangent[i]));
        let length = (raw.iter().map(|v| v * v).sum::<f64>()).sqrt();
        let nz = raw[2] / length;
        let expected = mr_brdf([0.5; 3], 0.0, 1.0, nz, nz, nz, 1.0)[0] * nz;
        for y in 29..33 {
            for x in 29..33 {
                for value in &frame.pixel(x, y).unwrap()[..3] {
                    assert!((f64::from(*value) - expected).abs() < 0.003);
                }
            }
        }
        center_values.push(frame.pixel(31, 31).unwrap()[0]);
    }
    assert!(center_values[0] > center_values[1] && center_values[1] > center_values[2]);
}
