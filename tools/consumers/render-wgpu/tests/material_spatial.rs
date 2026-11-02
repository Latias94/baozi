mod common;
use baozi::*;
use baozi_asset_consumer::{Binding, Image, Sampler, Texture};
use baozi_render_wgpu::{Error, material::*, spatial::*};

#[test]
fn independent_numeric_brdf_emission_and_spatial_vectors() {
    assert!((srgb_to_linear(0.5) - 0.21404114048223255).abs() < 1e-15);
    let mr = mr_brdf([0.5; 3], 0.0, 1.0, 1.0, 1.0, 1.0, 1.0);
    assert!((mr[0] - 0.49 / std::f64::consts::PI).abs() < 1e-14);
    let coat = (1.0 - 0.5 * 0.04) * mr[0] + 0.5 * 0.04 / (4.0 * std::f64::consts::PI);
    assert!((coat - 0.15444395677637524).abs() < 1e-14);
    assert!((charlie_brdf(0.5, 0.5, 0.5, 1.0) - 0.17086246915557465).abs() < 1e-12);
    let integrals = [
        (64, 128, 0.35631054879613216),
        (128, 256, 0.3562942485648036),
        (256, 512, 0.356290425353786),
    ];
    let mut previous_error = f64::INFINITY;
    for (no_l_steps, phi_steps, expected) in integrals {
        let actual = sheen_albedo(0.5, 1.0, no_l_steps, phi_steps);
        assert!(
            (actual - expected).abs() < 1e-12,
            "{no_l_steps}x{phi_steps}: {actual} != {expected}"
        );
        let error = (actual - 0.356290425353786).abs();
        assert!(error < previous_error);
        previous_error = error;
    }
    assert_eq!(range_attenuation(2.0, Some(4.0)), 225.0 / 256.0);
    assert_eq!(spot_attenuation(0.75, 1.0, 0.5), 0.25);
    let asset = common::asset(3);
    let evaluation = common::evaluated(&asset);
    let mut world = IDENTITY;
    world[3][2] = 3.0;
    let camera = camera(
        &asset,
        &evaluation,
        CameraInput::Explicit {
            projection: CameraProjection::perspective(
                LocalAxes::STANDARD,
                std::f32::consts::FRAC_PI_2,
                Some(1.0),
                1.0,
                Some(10.0),
            )
            .unwrap(),
            world,
        },
        64,
        64,
    )
    .unwrap();
    let clip = project(camera.view_projection, [0.0, 0.0, 0.0, 1.0]);
    assert!((clip[2] / clip[3] - 20.0 / 27.0).abs() < 1e-12);
    world[0][0] = 2.0;
    assert!(matches!(
        camera_for_test(&asset, &evaluation, world),
        Err(Error::NonRigidSpatialPose { .. })
    ));
}

fn camera_for_test(
    asset: &baozi_asset_consumer::ApplicationAsset,
    evaluation: &baozi_asset_consumer::EvaluatedAsset,
    world: baozi_asset_consumer::EvaluationMatrix,
) -> baozi_render_wgpu::Result<CameraFrame> {
    camera(
        asset,
        evaluation,
        CameraInput::Explicit {
            projection: CameraProjection::orthographic(LocalAxes::STANDARD, 2.0, 1.0, 1.0, 10.0)
                .unwrap(),
            world,
        },
        64,
        64,
    )
}

#[test]
fn uv1_affine_after_origin_mapping_and_unlit_inactive_closure() {
    let mut asset = common::asset(3);
    asset.images.push(Image {
        name: None,
        encoded_bytes: vec![],
        media_type: None,
    });
    asset.textures.push(Texture {
        name: None,
        image: 0,
        sampler: 0,
    });
    asset.samplers.push(Sampler {
        name: None,
        state: SamplerState::CANONICAL_DEFAULT,
    });
    asset.materials[0].bindings.push(Binding {
        role: TextureRole::BaseColor,
        texture: 0,
        uv_set: 1,
        channel_mapping: TextureChannelSwizzle::RGBA,
        color_interpretation: ColorInterpretation::SrgbColor,
        transform: TextureTransform::try_new(
            Vec2::new(0.125, 0.25),
            std::f32::consts::FRAC_PI_2,
            Vec2::new(2.0, 0.5),
        )
        .unwrap(),
        strength: TextureStrength::try_new(0.25).unwrap(),
        blend: TextureBlendOperation::Replace,
        source_key: None,
    });
    asset.materials[0].bindings.push(Binding {
        role: TextureRole::Normal,
        texture: usize::MAX,
        uv_set: 0,
        channel_mapping: TextureChannelSwizzle::RGB,
        color_interpretation: ColorInterpretation::Data,
        transform: TextureTransform::IDENTITY,
        strength: TextureStrength::try_new(1.0).unwrap(),
        blend: TextureBlendOperation::Replace,
        source_key: None,
    });
    let compiled = CompiledMaterial::compile(&asset, Some(0)).unwrap();
    assert_eq!(compiled.active_uv_sets, vec![1]);
    assert!(compiled.slots[NORMAL].is_none());
    let uv = compiled.slots[BASE].unwrap().coordinate([0.25, 0.125]);
    assert!((uv[0] - 0.0625).abs() < 1e-6 && (uv[1] - 0.75).abs() < 1e-6);
    asset.space.uv_origin = UvOrigin::LowerLeft;
    let lower = CompiledMaterial::compile(&asset, Some(0)).unwrap().slots[BASE]
        .unwrap()
        .coordinate([0.25, 0.125]);
    assert!((lower[1] - 0.25).abs() < 1e-6);
    assert_eq!(compiled.bytes.len(), 560);
}

#[test]
fn bounded_sheen_table_agrees_with_independent_converged_reference() {
    let (bytes, work) =
        baozi_render_wgpu::renderer::sheen_table(2_097_152, baozi_render_wgpu::Control::default())
            .unwrap();
    assert_eq!(bytes.len(), 32 * 32 * 2);
    assert_eq!(work, 2_097_152);
    let energy = |x: usize| {
        let offset = (31 * 32 + x) * 2;
        half::f16::from_bits(u16::from_le_bytes([bytes[offset], bytes[offset + 1]])).to_f64()
    };
    let coordinate: f64 = (0.5 - 0.001) / 0.999 * 31.0;
    let left = coordinate.floor() as usize;
    let fraction = coordinate - left as f64;
    let interpolated = energy(left) * (1.0 - fraction) + energy(left + 1) * fraction;
    // Independent preparation's 256x512 midpoint integral, not this table's own sample.
    assert!((interpolated - 0.356290425353786).abs() < 0.003);
    // Independent NoL/azimuth midpoint calculation of the published fit. This
    // intentionally exceeds one; an energy clamp would hide its grazing behavior.
    let grazing = half::f16::from_bits(u16::from_le_bytes([bytes[0], bytes[1]])).to_f64();
    assert!((grazing - 4.497436310601021).abs() < 0.004);
    for pair in bytes.chunks_exact(2) {
        let energy = half::f16::from_bits(u16::from_le_bytes([pair[0], pair[1]])).to_f64();
        assert!(energy.is_finite() && energy >= 0.0);
    }
    for (steps, raw_expected) in [
        (64, 4.480886035754134),
        (128, 4.455474154669953),
        (256, 4.434474762298661),
    ] {
        let raw = sheen_albedo(0.001, 0.045, steps, steps * 2);
        assert!((raw - raw_expected).abs() < 1e-11);
        assert!(raw > 1.0);
    }
}

#[test]
fn bounded_sheen_scaling_has_nonnegative_symmetric_layer_weights() {
    let ev = 4.497436310601021;
    let el = 0.35631054879613216;
    for (color, expected_substrate) in [(0.0, 1.0), (0.25, 0.75), (1.0, 0.0)] {
        let forward = sheen_layer_weights(ev, el, color).unwrap();
        let reverse = sheen_layer_weights(el, ev, color).unwrap();
        assert_eq!(forward, reverse);
        assert!((forward.0 - 0.22234889633520205).abs() < 1e-15);
        assert_eq!(forward.1, expected_substrate);
    }
    let ordinary = sheen_layer_weights(0.25, 0.5, 0.5).unwrap();
    assert_eq!(ordinary, (1.0, 0.75));
    // Independent reciprocal after the raw grazing cell is quantized to R16F.
    let packed = sheen_layer_weights(4.49609375, 4.49609375, 1.0).unwrap();
    assert!((packed.0 - 0.22241529105125976).abs() < 1e-15);
    assert_eq!(packed.1, 0.0);
    assert!(sheen_layer_weights(f64::NAN, el, 1.0).is_err());
    assert!(sheen_layer_weights(ev, -0.1, 1.0).is_err());
    assert!(sheen_layer_weights(ev, el, 1.1).is_err());
    // Independent raw grazing BRDF; bounded scaling is a separate consumer policy.
    assert!((charlie_brdf(0.001, 0.001, 0.001, 0.045) - 5240.180562185577).abs() < 1e-8);
}

#[test]
fn zero_transmission_and_thickness_do_not_activate_bindings_or_image_uv_closure() {
    let mut asset = common::asset(3);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].factors = asset.materials[0]
        .factors
        .with_transmission(Some(TransmissionFactors::try_new(0.0).unwrap()))
        .with_thickness(Some(ThicknessFactors::try_new(0.0).unwrap()));
    asset.images.push(Image {
        name: None,
        encoded_bytes: b"inactive undecodable payload".to_vec(),
        media_type: Some("image/png".into()),
    });
    asset.samplers.push(Sampler {
        name: None,
        state: SamplerState::CANONICAL_DEFAULT,
    });
    asset.textures.push(Texture {
        name: None,
        image: 0,
        sampler: 0,
    });
    asset.materials[0].bindings = [TextureRole::Transmission, TextureRole::Thickness]
        .into_iter()
        .map(|role| Binding {
            role,
            texture: 0,
            uv_set: 7,
            channel_mapping: TextureChannelSwizzle::RED,
            color_interpretation: ColorInterpretation::Data,
            transform: TextureTransform::IDENTITY,
            strength: TextureStrength::try_new(1.0).unwrap(),
            blend: TextureBlendOperation::Replace,
            source_key: None,
        })
        .collect();
    let original = asset.materials[0].factors;
    let compiled = CompiledMaterial::compile(&asset, Some(0)).unwrap();
    assert!(compiled.active_uv_sets.is_empty() && compiled.slots.iter().all(Option::is_none));
    let images = baozi_render_wgpu::images::PreparedImages::prepare(
        &asset,
        &[compiled],
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert!(images.variants.is_empty());
    assert_eq!(asset.materials[0].factors, original);
    assert_eq!(original.transmission().unwrap().factor(), 0.0);
    assert_eq!(original.thickness().unwrap().thickness(), 0.0);
    for factors in [
        original.with_transmission(Some(TransmissionFactors::try_new(0.25).unwrap())),
        original.with_thickness(Some(ThicknessFactors::try_new(0.25).unwrap())),
    ] {
        asset.materials[0].factors = factors;
        assert!(matches!(
            CompiledMaterial::compile(&asset, Some(0)),
            Err(Error::UnsupportedMaterial {
                feature: "transmission or thickness",
                ..
            })
        ));
    }
}
