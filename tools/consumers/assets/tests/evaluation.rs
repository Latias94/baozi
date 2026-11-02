use std::sync::Arc;

use baozi::{
    AnimationExtrapolation as Extrapolation, AnimationInterpolation as Interpolation,
    AnimationTimeDomain, AssetPath, AuthoredTransform, Axis, FrontFaceWinding, Handedness,
    Importer, Mat4, MaterialAssignment, MemoryAssetIo, MorphDeltaSemantic, Quat,
    SkinMatrixCorrection, SkinMatrixSpace, SkinMatrixSpaces, TargetSpace, TopologyKind, UvOrigin,
    Vec3,
    postprocess::{PostProcessControl, SpaceConversionOptions, convert_scene_space},
};
use baozi_asset_consumer::{
    ApplicationAsset, Channel, ChannelTarget, Clip, DrawBuffer, EvaluatedAsset, EvaluationError,
    Influences, Instance, MorphStream, Node, Result, SampleTime, Skin, Vertex, evaluate,
};
use serde_json::{Value, json};

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-11,
        "{actual} != {expected}"
    );
}

fn point(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        near(actual, expected);
    }
}

fn translation(x: f32) -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(x, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .unwrap()
}

fn bind_translation(x: f32) -> Mat4 {
    let mut matrix = Mat4::IDENTITY;
    matrix.cols[3][0] = x;
    matrix
}

fn node(x: f32) -> Node {
    Node {
        name: None,
        transform: translation(x),
        children: Vec::new(),
    }
}

fn asset() -> ApplicationAsset {
    ApplicationAsset {
        mesh_count: 1,
        mesh_morph_weights: vec![Vec::new()],
        roots: vec![0],
        nodes: vec![node(0.0)],
        draws: vec![DrawBuffer {
            mesh: 0,
            primitive: 0,
            material: MaterialAssignment::Unspecified,
            topology: TopologyKind::Points,
            vertices: vec![Vertex {
                position: [1.0, 0.0, 0.0],
                normal: None,
                tangent: None,
                color: None,
            }],
            indices: vec![0],
            influences: None,
            morphs: Vec::new(),
            uv_streams: Vec::new(),
            tangent_conventions: None,
        }],
        instances: vec![Instance {
            node: 0,
            mesh: 0,
            skin: None,
            morph_weights: None,
        }],
        skins: Vec::new(),
        animations: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        uv_origin: UvOrigin::Unknown,
        source_json_metadata: Vec::new(),
    }
}

fn channel(target: ChannelTarget, interpolation: Interpolation, values: &[f32]) -> Channel {
    let width = match target {
        ChannelTarget::Rotation(_) => 4,
        ChannelTarget::MorphWeights(_) => 1,
        _ => 3,
    };
    Channel {
        target,
        interpolation,
        pre_extrapolation: Extrapolation::Constant,
        post_extrapolation: Extrapolation::Constant,
        width,
        times: vec![0.0, 2.0],
        values: values.to_vec(),
        in_tangents: None,
        out_tangents: None,
    }
}

fn set_channel(asset: &mut ApplicationAsset, channel: Channel) {
    asset.animations = vec![Clip {
        name: None,
        duration: 2.0,
        time_domain: AnimationTimeDomain::Seconds,
        channels: vec![channel],
    }];
}

fn at(asset: &ApplicationAsset, time: f64) -> Result<EvaluatedAsset> {
    Ok(evaluate(asset, Some(0), SampleTime::Declared(time))?)
}

fn add_morph(asset: &mut ApplicationAsset, default: f32) {
    asset.mesh_morph_weights[0] = vec![default];
    asset.draws[0].morphs.push(MorphStream {
        target: 0,
        semantic: MorphDeltaSemantic::Position,
        values: vec![[2.0, 0.0, 0.0]],
    });
}

#[test]
fn unanimated_owned_positions_keep_f64_values() -> Result<()> {
    let mut asset = asset();
    let position = [16_777_217.0, -0.0, f64::from_bits(1)];
    asset.draws[0].vertices[0].position = position;
    let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    assert_eq!(
        sampled.draws[0].positions[0].map(f64::to_bits),
        position.map(f64::to_bits)
    );
    drop(asset);
    assert_eq!(sampled.draws[0].world_positions[0][0], 16_777_217.0);
    Ok(())
}

#[test]
fn step_linear_and_cubic_translation_use_independent_interval_vectors() -> Result<()> {
    let mut asset = asset();
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Translation(0),
            Interpolation::Step,
            &[1.0, 0.0, 0.0, 5.0, 0.0, 0.0],
        ),
    );
    near(at(&asset, 1.0)?.local_transforms[0][3][0], 1.0);
    near(at(&asset, 2.0)?.local_transforms[0][3][0], 5.0);
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Translation(0),
            Interpolation::Linear,
            &[0.0, 0.0, 0.0, 4.0, 0.0, 0.0],
        ),
    );
    near(at(&asset, 1.0)?.local_transforms[0][3][0], 2.0);
    let mut cubic = channel(
        ChannelTarget::Translation(0),
        Interpolation::CubicSpline,
        &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0],
    );
    cubic.in_tangents = Some(vec![0.0; 6]);
    cubic.out_tangents = Some(vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    set_channel(&mut asset, cubic);
    near(at(&asset, 1.0)?.local_transforms[0][3][0], 1.5);
    Ok(())
}

#[test]
fn scale_and_morph_curves_cover_each_interpolation_without_clamping() -> Result<()> {
    let mut asset = asset();
    add_morph(&mut asset, 0.25);
    for (interpolation, expected) in [
        (Interpolation::Step, 0.0),
        (Interpolation::Linear, 1.0),
        (Interpolation::CubicSpline, 1.5),
    ] {
        let mut scale = channel(
            ChannelTarget::Scale(0),
            interpolation,
            &[0.0, 1.0, 1.0, 2.0, 1.0, 1.0],
        );
        let mut morph = channel(ChannelTarget::MorphWeights(0), interpolation, &[0.0, 2.0]);
        if interpolation == Interpolation::CubicSpline {
            scale.in_tangents = Some(vec![0.0; 6]);
            scale.out_tangents = Some(vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            morph.in_tangents = Some(vec![0.0; 2]);
            morph.out_tangents = Some(vec![2.0, 0.0]);
        }
        set_channel(&mut asset, scale);
        asset.animations[0].channels.push(morph);
        let sampled = at(&asset, 1.0)?;
        near(sampled.local_transforms[0][0][0], expected);
        near(sampled.morph_weights[0][0], expected);
        point(
            sampled.draws[0].positions[0],
            [1.0 + 2.0 * expected, 0.0, 0.0],
        );
    }
    Ok(())
}

#[test]
fn linear_rotations_slerp_along_the_shortest_arc_without_rewriting_keys() -> Result<()> {
    let mut asset = asset();
    for sign in [1.0, -1.0] {
        let half = std::f32::consts::FRAC_1_SQRT_2 * sign;
        set_channel(
            &mut asset,
            channel(
                ChannelTarget::Rotation(0),
                Interpolation::Linear,
                &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, half, half],
            ),
        );
        let bits: Vec<_> = asset.animations[0].channels[0]
            .values
            .iter()
            .map(|v| v.to_bits())
            .collect();
        let sampled = at(&asset, 1.0)?;
        point(
            sampled.draws[0].world_positions[0],
            [
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
                0.0,
            ],
        );
        point(
            at(&asset, 0.5)?.draws[0].world_positions[0],
            [
                (std::f64::consts::PI / 8.0).cos(),
                (std::f64::consts::PI / 8.0).sin(),
                0.0,
            ],
        );
        assert_eq!(
            asset.animations[0].channels[0]
                .values
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            bits
        );
    }
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Rotation(0),
            Interpolation::Linear,
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0],
        ),
    );
    point(
        at(&asset, 1.0)?.draws[0].world_positions[0],
        [1.0, 0.0, 0.0],
    );
    Ok(())
}

#[test]
fn cubic_rotations_normalize_only_the_result_and_preserve_key_signs() -> Result<()> {
    let mut asset = asset();
    let mut cubic = channel(
        ChannelTarget::Rotation(0),
        Interpolation::CubicSpline,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
    );
    cubic.in_tangents = Some(vec![0.0; 8]);
    cubic.out_tangents = Some(vec![0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    set_channel(&mut asset, cubic);
    // dt=2 and h10=1/8 give raw q=(0,0,1/2,1). Its normalized rotation sends
    // X to (3/5,4/5,0). Unit-normalizing the derivative would give a different point.
    point(
        at(&asset, 1.0)?.draws[0].world_positions[0],
        [0.6, 0.8, 0.0],
    );
    assert_eq!(
        asset.animations[0].channels[0]
            .out_tangents
            .as_ref()
            .unwrap()[2],
        2.0
    );
    asset.animations[0].channels[0].out_tangents =
        Some(vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    asset.animations[0].channels[0].in_tangents =
        Some(vec![0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    point(
        at(&asset, 1.0)?.draws[0].world_positions[0],
        [2.0 / 3.0, -1.0 / 3.0, 2.0 / 3.0],
    );
    let cubic = &mut asset.animations[0].channels[0];
    cubic.values[7] = -1.0;
    cubic.in_tangents = Some(vec![0.0; 8]);
    cubic.out_tangents = Some(vec![0.0; 8]);
    assert!(matches!(
        evaluate(&asset, Some(0), SampleTime::Declared(1.0)),
        Err(EvaluationError::ZeroLengthQuaternion { channel: 0 })
    ));
    Ok(())
}

#[test]
fn one_key_and_step_rotations_remain_defined_at_endpoints() -> Result<()> {
    let mut asset = asset();
    for interpolation in [Interpolation::Step, Interpolation::Linear] {
        let mut single = channel(
            ChannelTarget::Rotation(0),
            interpolation,
            &[0.0, 0.0, 1.0, 0.0],
        );
        single.times = vec![10.0];
        set_channel(&mut asset, single);
        for time in [-1.0, 10.0, 20.0] {
            point(
                at(&asset, time)?.draws[0].world_positions[0],
                [-1.0, 0.0, 0.0],
            );
        }
    }
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Rotation(0),
            Interpolation::Step,
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0],
        ),
    );
    point(
        at(&asset, 1.0)?.draws[0].world_positions[0],
        [1.0, 0.0, 0.0],
    );
    point(
        at(&asset, 2.0)?.draws[0].world_positions[0],
        [-1.0, 0.0, 0.0],
    );
    Ok(())
}

#[test]
fn effective_endpoint_rotation_normalizes_a_tolerated_key_without_rewriting_it() -> Result<()> {
    let mut asset = asset();
    for interpolation in [Interpolation::Step, Interpolation::Linear] {
        for sign in [1.0, -1.0] {
            let a = f32::from_bits(0x3f35_0744) * sign;
            let key = [-0.0_f32, 0.0, a, a];
            let mut single = channel(ChannelTarget::Rotation(0), interpolation, &key);
            single.times = vec![10.0];
            set_channel(&mut asset, single);
            for time in [0.0, 10.0, 20.0] {
                point(
                    at(&asset, time)?.draws[0].world_positions[0],
                    [0.0, 1.0, 0.0],
                );
                assert_eq!(
                    asset.animations[0].channels[0]
                        .values
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    key.map(f32::to_bits),
                );
            }
        }
    }
    Ok(())
}

#[test]
fn constant_and_source_default_are_distinct_on_both_sides() -> Result<()> {
    let mut asset = asset();
    let mut translated = channel(
        ChannelTarget::Translation(0),
        Interpolation::Linear,
        &[7.0, 0.0, 0.0, 9.0, 0.0, 0.0],
    );
    translated.times = vec![10.0, 12.0];
    set_channel(&mut asset, translated);
    near(at(&asset, 0.0)?.local_transforms[0][3][0], 7.0);
    near(at(&asset, 13.0)?.local_transforms[0][3][0], 9.0);
    asset.animations[0].channels[0].pre_extrapolation = Extrapolation::SourceDefault;
    asset.animations[0].channels[0].post_extrapolation = Extrapolation::SourceDefault;
    near(at(&asset, 11.0)?.local_transforms[0][3][0], 8.0);
    near(at(&asset, 0.0)?.local_transforms[0][3][0], 0.0);
    near(at(&asset, 13.0)?.local_transforms[0][3][0], 0.0);
    near(at(&asset, 10.0)?.local_transforms[0][3][0], 7.0);
    for mode in [Extrapolation::Linear, Extrapolation::Repeat] {
        for before in [false, true] {
            let c = &mut asset.animations[0].channels[0];
            c.pre_extrapolation = if before {
                mode
            } else {
                Extrapolation::Constant
            };
            c.post_extrapolation = if before {
                Extrapolation::Constant
            } else {
                mode
            };
            assert!(
                matches!(evaluate(&asset, Some(0), SampleTime::Declared(11.0)),
                Err(EvaluationError::UnsupportedExtrapolation { channel: 0, mode: actual }) if actual == mode)
            );
        }
    }
    Ok(())
}

#[test]
fn f64_time_and_explicit_rates_preserve_declared_domain_derivatives() -> Result<()> {
    let mut asset = asset();
    let mut translated = channel(
        ChannelTarget::Translation(0),
        Interpolation::Linear,
        &[0.0, 0.0, 0.0, 4.0, 0.0, 0.0],
    );
    translated.times = vec![1_000_000_000.0, 1_000_000_000.125];
    set_channel(&mut asset, translated);
    near(
        at(&asset, 1_000_000_000.062_5)?.local_transforms[0][3][0],
        2.0,
    );
    asset.animations[0].channels[0].times = vec![0.0, 2.0];
    asset.animations[0].channels[0].interpolation = Interpolation::CubicSpline;
    asset.animations[0].channels[0].values[3] = 2.0;
    asset.animations[0].channels[0].in_tangents = Some(vec![0.0; 6]);
    asset.animations[0].channels[0].out_tangents = Some(vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    for (domain, rate) in [
        (AnimationTimeDomain::Frames { rate: 24.0 }, 24.0),
        (AnimationTimeDomain::Ticks { rate: Some(48.0) }, 48.0),
    ] {
        asset.animations[0].time_domain = domain;
        near(
            evaluate(&asset, Some(0), SampleTime::Seconds(1.0 / rate))?.local_transforms[0][3][0],
            1.5,
        );
    }
    asset.animations[0].time_domain = AnimationTimeDomain::Ticks { rate: None };
    near(at(&asset, 1.0)?.local_transforms[0][3][0], 1.5);
    assert!(matches!(
        evaluate(&asset, Some(0), SampleTime::Seconds(1.0)),
        Err(EvaluationError::UnknownTimeRate)
    ));
    for rate in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        asset.animations[0].time_domain = AnimationTimeDomain::Frames { rate };
        assert!(matches!(
            evaluate(&asset, Some(0), SampleTime::Seconds(1.0)),
            Err(EvaluationError::InvalidTimeRate)
        ));
    }
    for time in [f64::NAN, f64::INFINITY] {
        assert!(matches!(
            evaluate(&asset, None, SampleTime::Declared(time)),
            Err(EvaluationError::NonFiniteTime)
        ));
    }
    Ok(())
}

#[test]
fn morph_precedence_is_per_instance_and_source_default_is_immutable() -> Result<()> {
    let mut asset = asset();
    add_morph(&mut asset, 0.25);
    asset.nodes.push(node(0.0));
    asset.roots.push(1);
    asset.instances.push(Instance {
        node: 1,
        mesh: 0,
        skin: None,
        morph_weights: Some(vec![0.75]),
    });
    let base = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    point(base.draws[0].positions[0], [1.5, 0.0, 0.0]);
    point(base.draws[1].positions[0], [2.5, 0.0, 0.0]);
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::MorphWeights(1),
            Interpolation::Linear,
            &[0.0, 1.0],
        ),
    );
    asset.animations[0].channels[0].pre_extrapolation = Extrapolation::SourceDefault;
    asset.animations[0].channels[0].post_extrapolation = Extrapolation::SourceDefault;
    let sampled = at(&asset, 1.0)?;
    assert_eq!(sampled.morph_weights, [vec![0.25], vec![0.5]]);
    point(sampled.draws[0].positions[0], [1.5, 0.0, 0.0]);
    point(sampled.draws[1].positions[0], [2.0, 0.0, 0.0]);
    assert_eq!(at(&asset, -1.0)?.morph_weights, [vec![0.25], vec![0.75]]);
    assert_eq!(at(&asset, 3.0)?.morph_weights, [vec![0.25], vec![0.75]]);
    asset.animations[0].channels[0].target = ChannelTarget::MorphWeights(0);
    assert_eq!(at(&asset, -1.0)?.morph_weights, [vec![0.25], vec![0.75]]);
    assert_eq!(asset.mesh_morph_weights, [vec![0.25]]);
    assert_eq!(
        asset.instances[1].morph_weights.as_deref(),
        Some([0.75].as_slice())
    );
    asset.mesh_morph_weights[0] = vec![1.5, -0.5];
    asset.instances[1].morph_weights = None;
    asset.draws[0].morphs.push(MorphStream {
        target: 1,
        semantic: MorphDeltaSemantic::Position,
        values: vec![[4.0, 0.0, 0.0]],
    });
    point(
        evaluate(&asset, None, SampleTime::Declared(0.0))?.draws[0].positions[0],
        [2.0, 0.0, 0.0],
    );
    Ok(())
}

#[test]
fn world_composition_respects_parent_order_and_nonuniform_trs() -> Result<()> {
    let mut asset = asset();
    asset.nodes[0].transform = translation(1.0);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    asset.nodes.push(Node {
        name: None,
        transform: AuthoredTransform::try_trs(
            Vec3::new(10.0, 0.0, 0.0),
            Quat::from_xyzw(0.0, 0.0, half, half),
            Vec3::new(2.0, 3.0, 4.0),
        )?,
        children: vec![0],
    });
    asset.roots = vec![1];
    let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    // Authored F32 quaternion components are preserved, so allow their squared-length error.
    for (actual, expected) in sampled.draws[0].world_positions[0]
        .into_iter()
        .zip([10.0, 4.0, 0.0])
    {
        assert!((actual - expected).abs() < 3.0e-7);
    }
    let mut shear = Mat4::IDENTITY;
    shear.cols[1][0] = 2.0;
    asset.nodes[0].transform = AuthoredTransform::try_matrix(shear)?;
    assert_eq!(
        evaluate(&asset, None, SampleTime::Declared(0.0))?.local_transforms[0][1][0],
        2.0
    );
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Translation(0),
            Interpolation::Linear,
            &[0.0; 6],
        ),
    );
    assert!(matches!(
        evaluate(&asset, Some(0), SampleTime::Declared(1.0)),
        Err(EvaluationError::MatrixAuthoredAnimation { node: 0 })
    ));
    Ok(())
}

fn skinned_asset() -> ApplicationAsset {
    let mut asset = asset();
    add_morph(&mut asset, 0.5);
    asset.nodes = vec![
        node(10.0),
        node(10.0),
        node(14.0),
        node(20.0),
        node(20.0),
        node(24.0),
    ];
    asset.roots = vec![0, 1, 2, 3, 4, 5];
    asset.instances[0].skin = Some(0);
    asset.instances.push(Instance {
        node: 3,
        mesh: 0,
        skin: Some(1),
        morph_weights: None,
    });
    for joints in [vec![1, 2], vec![4, 5]] {
        asset.skins.push(Skin {
            joints,
            inverse_binds: vec![Mat4::IDENTITY, bind_translation(-2.0)],
            matrix_spaces: SkinMatrixSpaces::canonical(),
        });
    }
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 2],
        joints: vec![0, 1],
        weights: vec![0.25, 0.75],
    });
    asset
}

#[test]
fn two_joint_positions_morph_before_skin_with_each_instances_palette() -> Result<()> {
    let asset = skinned_asset();
    let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    // p=1 + 0.5*2=2; first world x=0.25*12 + 0.75*14=13.5, then inverse(T10).
    point(sampled.draws[0].positions[0], [3.5, 0.0, 0.0]);
    point(sampled.draws[0].world_positions[0], [13.5, 0.0, 0.0]);
    point(sampled.draws[1].positions[0], [3.5, 0.0, 0.0]);
    point(sampled.draws[1].world_positions[0], [23.5, 0.0, 0.0]);
    assert_eq!((sampled.draws[1].instance, sampled.draws[1].draw), (1, 0));
    drop(asset);
    point(sampled.draws[0].positions[0], [3.5, 0.0, 0.0]);
    Ok(())
}

#[test]
fn skinning_retains_all_influences_and_unnormalized_homogeneous_weights() -> Result<()> {
    let mut asset = skinned_asset();
    asset.draws[0].influences.as_mut().unwrap().weights = vec![0.5, 1.5];
    let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    point(sampled.draws[0].positions[0], [7.0, 0.0, 0.0]);
    point(sampled.draws[0].world_positions[0], [27.0, 0.0, 0.0]);
    point(sampled.draws[1].positions[0], [7.0, 0.0, 0.0]);
    point(sampled.draws[1].world_positions[0], [47.0, 0.0, 0.0]);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 6],
        joints: vec![0, 1, 0, 1, 0, 1],
        weights: vec![0.125, 0.125, 0.125, 0.125, 0.125, 0.375],
    });
    point(
        evaluate(&asset, None, SampleTime::Declared(0.0))?.draws[0].positions[0],
        [3.25, 0.0, 0.0],
    );
    Ok(())
}

#[test]
fn unsupported_correction_and_singular_instance_are_named_failures() -> Result<()> {
    let mut asset = skinned_asset();
    let correction = SkinMatrixCorrection::new(
        SkinMatrixSpace::LogicalMeshLocalBind,
        SkinMatrixSpace::MeshInstanceLocal,
        bind_translation(3.0),
    )?;
    asset.skins[0].matrix_spaces = SkinMatrixSpaces::canonical().with_correction(correction);
    assert!(matches!(
        evaluate(&asset, None, SampleTime::Declared(0.0)),
        Err(EvaluationError::UnsupportedSkinCorrection { skin: 0 })
    ));
    assert_eq!(asset.skins[0].matrix_spaces.correction(), Some(correction));
    asset.skins[0].matrix_spaces = SkinMatrixSpaces::canonical();
    asset.nodes[0].transform =
        AuthoredTransform::try_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.0, 1.0, 1.0))?;
    assert!(matches!(
        evaluate(&asset, None, SampleTime::Declared(0.0)),
        Err(EvaluationError::SingularInstanceTransform { instance: 0 })
    ));
    Ok(())
}

fn matrix_skin_asset(matrix: Mat4) -> Result<ApplicationAsset> {
    let mut asset = asset();
    asset.nodes[0].transform = AuthoredTransform::try_matrix(matrix)?;
    asset.nodes.push(node(0.0));
    asset.roots.push(1);
    asset.instances[0].skin = Some(0);
    asset.skins.push(Skin {
        joints: vec![1],
        inverse_binds: vec![Mat4::IDENTITY],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 1],
        joints: vec![0],
        weights: vec![1.0],
    });
    Ok(asset)
}

#[test]
fn rounded_nonzero_pivot_cannot_accept_an_owned_singular_instance() -> Result<()> {
    for exponent in [-120, 0, 120] {
        let scale = 2.0_f32.powi(exponent);
        // The exactly representable null vector [1,-2,1,0] proves rank deficiency.
        // At unit scale the old elimination left a nonzero pivot near -7.77e-16.
        let matrix = Mat4 {
            cols: [
                [scale, 4.0 * scale, 7.0 * scale, 0.0],
                [2.0 * scale, 5.0 * scale, 8.0 * scale, 0.0],
                [3.0 * scale, 6.0 * scale, 9.0 * scale, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let asset = matrix_skin_asset(matrix)?;
        assert!(
            matches!(
                evaluate(&asset, None, SampleTime::Declared(0.0)),
                Err(EvaluationError::SingularInstanceTransform { instance: 0 })
            ),
            "singular matrix at scale 2^{exponent} must not publish positions"
        );
    }
    Ok(())
}

#[test]
fn inaccurate_candidate_inverse_is_rejected_by_its_two_sided_residual() -> Result<()> {
    // The 2x2 block determinant is n*n - (n-1)*(n+1) = 1 exactly, so this is
    // invertible. Its conditioning amplifies rounded elimination enough that the
    // candidate's identity residual exceeds the reference profile's accuracy limit.
    for n in [16_384.0_f32, 1_048_576.0] {
        let matrix = Mat4 {
            cols: [
                [n, n + 1.0, 0.0, 0.0],
                [n - 1.0, n, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let asset = matrix_skin_asset(matrix)?;
        // n=1048576 makes the old rounded residual zero even though the exact residual
        // of its retained candidate is 2^-17, above the 2^-26 accuracy cap.
        assert!(
            matches!(
                evaluate(&asset, None, SampleTime::Declared(0.0)),
                Err(EvaluationError::NumericFailure {
                    operation: "instance inverse residual"
                })
            ),
            "the determinant-one block at n={n} needs a certified inverse"
        );
    }
    Ok(())
}

#[test]
fn general_affine_translation_cancellation_uses_the_full_vector_scale() -> Result<()> {
    for exponent in [-120, 0, 120] {
        let scale = 2.0_f32.powi(exponent);
        // det(A)=23, cond_inf(A)=133/23; the exact inverse translation is [0,-1,0].
        // A single zero row of A*(-inverse(A)*t)+t may retain only roundoff.
        let matrix = Mat4 {
            cols: [
                [-2.0 * scale, 3.0 * scale, 0.0, 0.0],
                [0.0, scale, 3.0 * scale, 0.0],
                [scale, 3.0 * scale, 2.0 * scale, 0.0],
                [0.0, scale, 3.0 * scale, 1.0],
            ],
        };
        let mut asset = matrix_skin_asset(matrix)?;
        asset.draws[0].vertices[0].position = [f64::from(scale), 0.0, 0.0];
        let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
        point(
            sampled.draws[0].positions[0],
            [-7.0 / 23.0, -29.0 / 23.0, 9.0 / 23.0],
        );
        point(
            sampled.draws[0].world_positions[0].map(|value| value / f64::from(scale)),
            [1.0, 0.0, 0.0],
        );
    }
    Ok(())
}

#[test]
fn small_and_large_uniform_scales_and_affine_shear_remain_invertible() -> Result<()> {
    for exponent in [-120, 0, 120] {
        for shear in [false, true] {
            let scale = 2.0_f32.powi(exponent);
            let matrix = Mat4 {
                cols: [
                    [scale, 0.0, 0.0, 0.0],
                    [if shear { 2.0 * scale } else { 0.0 }, scale, 0.0, 0.0],
                    [0.0, if shear { -scale } else { 0.0 }, scale, 0.0],
                    [4.0 * scale, -2.0 * scale, 6.0 * scale, 1.0],
                ],
            };
            let mut asset = matrix_skin_asset(matrix)?;
            asset.draws[0].vertices[0].position =
                [17.0, 2.0, 3.0].map(|value| value * f64::from(scale));
            let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
            drop(asset);
            // After removing translation, (13,4,-3) is sent through inverse(S).
            // For shear S=[[1,2,0],[0,1,-1],[0,0,1]], inverse(S)*d=(11,1,-3).
            point(
                sampled.draws[0].positions[0],
                if shear {
                    [11.0, 1.0, -3.0]
                } else {
                    [13.0, 4.0, -3.0]
                },
            );
            point(
                sampled.draws[0].world_positions[0].map(|value| value / f64::from(scale)),
                [17.0, 2.0, 3.0],
            );
        }
    }
    Ok(())
}

#[test]
fn hierarchical_uniform_scales_remain_invertible_beyond_f32_range() -> Result<()> {
    for exponent in [-100, 100] {
        let step = 2.0_f32.powi(exponent);
        let authored =
            AuthoredTransform::try_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(step, step, step))?;
        let mut asset = matrix_skin_asset(Mat4::IDENTITY)?;
        asset.nodes[0].transform = authored;
        // Six local transforms produce a finite, well-conditioned world scale 2^(+/-600).
        // A direct determinant would underflow or overflow despite the invertible transform.
        let mut child = 0;
        for _ in 0..5 {
            let parent = asset.nodes.len();
            asset.nodes.push(Node {
                name: None,
                transform: authored,
                children: vec![child],
            });
            child = parent;
        }
        asset.roots[0] = child;
        let scale = 2.0_f64.powi(exponent * 6);
        asset.draws[0].vertices[0].position = [1.0, -2.0, 3.0].map(|value| value * scale);
        let sampled = evaluate(&asset, None, SampleTime::Declared(0.0))?;
        point(sampled.draws[0].positions[0], [1.0, -2.0, 3.0]);
        point(
            sampled.draws[0].world_positions[0].map(|value| value / scale),
            [1.0, -2.0, 3.0],
        );
    }
    Ok(())
}

#[test]
fn malformed_owned_tables_fail_without_partial_output_or_panics() -> Result<()> {
    let mut asset = asset();
    set_channel(
        &mut asset,
        channel(
            ChannelTarget::Translation(0),
            Interpolation::Linear,
            &[0.0; 6],
        ),
    );
    asset.animations[0].channels[0].times[1] = 0.0;
    assert!(matches!(
        evaluate(&asset, Some(0), SampleTime::Declared(1.0)),
        Err(EvaluationError::InvalidChannel { .. })
    ));
    asset.nodes[0].children.push(0);
    assert!(matches!(
        evaluate(&asset, None, SampleTime::Declared(0.0)),
        Err(EvaluationError::InvalidHierarchy)
    ));
    let mut skinned = skinned_asset();
    skinned.draws[0].influences.as_mut().unwrap().joints[0] = 99;
    assert!(matches!(
        evaluate(&skinned, None, SampleTime::Declared(0.0)),
        Err(EvaluationError::InvalidAsset(_))
    ));
    Ok(())
}

#[derive(Default)]
struct Payload {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Payload {
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let index = self.views.len();
        self.views
            .push(json!({"buffer": 0, "byteOffset": self.bytes.len(), "byteLength": bytes.len()}));
        self.bytes.extend_from_slice(bytes);
        index
    }

    fn floats(&mut self, values: &[f32], shape: &str, lanes: usize) -> usize {
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let view = self.view(&bytes);
        let accessor = self.accessors.len();
        self.accessors
            .push(json!({"bufferView": view, "componentType": 5126,
            "type": shape, "count": values.len() / lanes}));
        accessor
    }
}

fn fixture_io() -> Result<MemoryAssetIo> {
    let mut payload = Payload::default();
    let positions = payload.floats(&[1.0, 0.5, 1.0, 2.0, -0.5, -1.0], "VEC3", 3);
    payload.accessors[positions]["min"] = json!([1.0, -0.5, -1.0]);
    payload.accessors[positions]["max"] = json!([2.0, 0.5, 1.0]);
    let sparse_indices = payload.view(&[0]);
    let delta_bytes: Vec<_> = [2.0_f32, 1.0, 0.5]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let sparse_values = payload.view(&delta_bytes);
    let morph = payload.accessors.len();
    payload
        .accessors
        .push(json!({"componentType": 5126, "type": "VEC3", "count": 2,
        "min": [0.0, 0.0, 0.0], "max": [2.0, 1.0, 0.5],
        "sparse": {"count": 1, "indices": {"bufferView": sparse_indices, "componentType": 5121},
            "values": {"bufferView": sparse_values}}}));
    let joints_view = payload.view(&[0, 1, 0, 0, 0, 1, 0, 0]);
    let joints = payload.accessors.len();
    payload.accessors.push(
        json!({"bufferView": joints_view, "componentType": 5121, "type": "VEC4", "count": 2}),
    );
    let weights = payload.floats(&[0.25, 0.75, 0.0, 0.0, 0.25, 0.75, 0.0, 0.0], "VEC4", 4);
    let binds: Vec<_> = [Mat4::IDENTITY, bind_translation(-2.0)]
        .into_iter()
        .flat_map(|matrix| matrix.cols.into_iter().flatten())
        .collect();
    let binds = payload.floats(&binds, "MAT4", 16);
    let times = payload.floats(&[0.0, 2.0], "SCALAR", 1);
    payload.accessors[times]["min"] = json!([0.0]);
    payload.accessors[times]["max"] = json!([2.0]);
    let morph_values = payload.floats(&[0.25, 0.75], "SCALAR", 1);
    let rotations = payload.floats(
        &[
            0.0, 0.0, 0.0, 0.0, // key 0 incoming derivative
            0.0, 0.0, 0.0, 1.0, // key 0 quaternion
            2.0, 0.0, 0.0, 0.0, // key 0 outgoing derivative (reflection must change its sign)
            0.0, 0.0, 0.0, 0.0, // key 1 incoming derivative
            0.0, 0.0, 0.0, 1.0, // key 1 quaternion
            0.0, 0.0, 0.0, 0.0, // key 1 outgoing derivative
        ],
        "VEC4",
        4,
    );
    let late_times = payload.floats(&[10.0, 12.0], "SCALAR", 1);
    payload.accessors[late_times]["min"] = json!([10.0]);
    payload.accessors[late_times]["max"] = json!([12.0]);
    let late_translation = payload.floats(&[7.0, 0.0, 0.0, 9.0, 0.0, 0.0], "VEC3", 3);
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0, 1, 3, 4, 6]}],
        "nodes": [
            {"name": "first", "mesh": 0, "skin": 0, "translation": [10.0, 0.0, 0.0],
                "scale": [2.0, 3.0, 4.0], "weights": [0.75]},
            {"name": "joint-a0", "translation": [10.0, 0.0, 0.0], "scale": [1.0, 2.0, 3.0], "children": [2]},
            {"name": "joint-a1", "translation": [4.0, 0.0, 0.0]},
            {"name": "second", "mesh": 0, "skin": 1, "translation": [20.0, 0.0, 0.0], "scale": [1.0, 2.0, 3.0]},
            {"name": "joint-b0", "translation": [20.0, 0.0, 0.0], "scale": [1.0, 2.0, 3.0], "children": [5]},
            {"name": "joint-b1", "translation": [4.0, 0.0, 0.0]},
            {"name": "late-base"}
        ],
        "skins": [
            {"joints": [1, 2], "skeleton": 1, "inverseBindMatrices": binds},
            {"joints": [4, 5], "skeleton": 4, "inverseBindMatrices": binds}
        ],
        "meshes": [{"weights": [0.25], "primitives": [{"mode": 0,
            "attributes": {"POSITION": positions, "JOINTS_0": joints, "WEIGHTS_0": weights},
            "targets": [{"POSITION": morph}]}]}],
        "animations": [
            {"name": "deform", "samplers": [
                {"input": times, "output": morph_values, "interpolation": "LINEAR"},
                {"input": times, "output": rotations, "interpolation": "CUBICSPLINE"}
            ], "channels": [
                {"sampler": 0, "target": {"node": 0, "path": "weights"}},
                {"sampler": 1, "target": {"node": 1, "path": "rotation"}}
            ]},
            {"name": "late", "samplers": [{"input": late_times, "output": late_translation, "interpolation": "LINEAR"}],
                "channels": [{"sampler": 0, "target": {"node": 6, "path": "translation"}}]}
        ],
        "buffers": [{"uri": "evaluation.bin", "byteLength": payload.bytes.len()}],
        "bufferViews": payload.views, "accessors": payload.accessors
    });
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("evaluation.gltf")?,
        Arc::from(serde_json::to_vec(&document)?),
    )?;
    io.try_insert(AssetPath::new("evaluation.bin")?, Arc::from(payload.bytes))?;
    Ok(io)
}

fn copied_pair() -> Result<(ApplicationAsset, ApplicationAsset)> {
    let io = fixture_io()?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("evaluation.gltf")?)?;
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    let source = ApplicationAsset::from_scene(&scene)?;
    let (scene, conversion) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::try_new(
                Handedness::Left,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.01,
                FrontFaceWinding::CounterClockwise,
            )?,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    let converted = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(conversion);
    drop(evidence);
    drop(importer);
    drop(io);
    Ok((source, converted))
}

#[test]
fn public_import_copy_drop_preserves_animation_skin_and_weight_contracts() -> Result<()> {
    let (source, converted) = copied_pair()?;
    drop(converted);
    assert_eq!(source.mesh_morph_weights, [vec![0.25]]);
    assert_eq!(
        source.instances[0].morph_weights.as_deref(),
        Some([0.75].as_slice())
    );
    assert!(source.instances[1].morph_weights.is_none());
    assert_eq!(source.skins[0].matrix_spaces, SkinMatrixSpaces::canonical());
    assert_eq!(source.skins[0].inverse_binds[1], bind_translation(-2.0));
    assert_eq!(
        source.animations[0].time_domain,
        AnimationTimeDomain::Seconds
    );
    for clip in &source.animations {
        for channel in &clip.channels {
            assert_eq!(channel.pre_extrapolation, Extrapolation::Constant);
            assert_eq!(channel.post_extrapolation, Extrapolation::Constant);
        }
    }
    assert_eq!(source.draws[0].morphs[0].values[1], [0.0; 3]);
    let sampled = evaluate(&source, Some(0), SampleTime::Declared(1.0))?;
    assert_eq!(sampled.morph_weights, [vec![0.5], vec![0.25]]);
    point(sampled.draws[0].world_positions[0], [13.5, -1.8, 3.85]);
    point(sampled.draws[0].positions[0], [1.75, -0.6, 0.9625]);
    point(sampled.draws[1].world_positions[0], [23.0, 1.5, 3.375]);
    point(sampled.draws[1].positions[0], [3.0, 0.75, 1.125]);
    assert_eq!(
        evaluate(&source, Some(0), SampleTime::Declared(-1.0))?.morph_weights,
        [vec![0.25], vec![0.25]]
    );
    assert_eq!(
        evaluate(&source, Some(0), SampleTime::Declared(3.0))?.morph_weights,
        [vec![0.75], vec![0.25]]
    );
    let late_node = source
        .nodes
        .iter()
        .position(|node| node.name.as_deref() == Some("late-base"))
        .unwrap();
    near(
        evaluate(&source, None, SampleTime::Declared(0.0))?.local_transforms[late_node][3][0],
        0.0,
    );
    let late = evaluate(&source, Some(1), SampleTime::Declared(0.0))?;
    near(late.local_transforms[late_node][3][0], 7.0);
    near(
        evaluate(&source, Some(1), SampleTime::Declared(13.0))?.local_transforms[late_node][3][0],
        9.0,
    );
    drop(source);
    point(sampled.draws[0].world_positions[0], [13.5, -1.8, 3.85]);
    Ok(())
}

#[test]
fn reflected_centimeter_conversion_commutes_with_evaluation() -> Result<()> {
    let (source, converted) = copied_pair()?;
    // H = diag(100,100,-100,1). Source and converted owned copies are evaluated after
    // all Scenes, import evidence, importer and sidecar IO have been destroyed.
    let derivative = converted.animations[0]
        .channels
        .iter()
        .find(|channel| matches!(channel.target, ChannelTarget::Rotation(_)))
        .unwrap();
    assert_eq!(derivative.out_tangents.as_ref().unwrap()[0], -2.0);
    for time in [-1.0, 0.0, 0.25, 1.0, 1.75, 2.0, 3.0] {
        let original = evaluate(&source, Some(0), SampleTime::Declared(time))?;
        let changed = evaluate(&converted, Some(0), SampleTime::Declared(time))?;
        assert_eq!(original.morph_weights, changed.morph_weights);
        assert_eq!(original.draws.len(), changed.draws.len());
        for (original, changed) in original.draws.iter().zip(&changed.draws) {
            assert_eq!(
                (original.instance, original.draw),
                (changed.instance, changed.draw)
            );
            for (original, changed) in original.positions.iter().zip(&changed.positions).chain(
                original
                    .world_positions
                    .iter()
                    .zip(&changed.world_positions),
            ) {
                let expected = [
                    100.0 * original[0],
                    100.0 * original[1],
                    -100.0 * original[2],
                ];
                for (actual, expected) in changed.iter().copied().zip(expected) {
                    assert!(
                        (actual - expected).abs() <= 1.0e-8,
                        "H conversion at t={time}: {actual} != {expected}"
                    );
                }
            }
        }
    }
    Ok(())
}
