use std::{mem::size_of, path::PathBuf};

use baozi::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain, AssetPath,
    AttributeDomain, AuthoredTransform, Axis, CancellationToken, FrontFaceWinding, Handedness,
    Importer, Mat4, MaterialAssignment, MorphDeltaSemantic, SceneSpaceRef, SkinMatrixSpaces,
    TopologyKind, UvOrigin,
};
use baozi_asset_consumer::{
    ApplicationAsset, Channel, ChannelTarget, Clip, DrawBuffer, EvaluatedAsset, EvaluatedDraw,
    EvaluationError, EvaluationLimit, EvaluationLimits, EvaluationMatrix, EvaluationRequest,
    FrameFailure, FramePolicy, Influences, Instance, MorphStream, Node, Result, SampleTime, Skin,
    TangentBasis, TangentConventions, TangentEncoding, Vertex, evaluate,
};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};

fn request(frames: FramePolicy) -> EvaluationRequest<'static> {
    EvaluationRequest {
        frames,
        limits: EvaluationLimits::default(),
        cancellation: None,
    }
}

fn near(actual: f64, expected: f64) {
    assert!(actual.is_finite() && expected.is_finite());
    assert!(
        (actual - expected).abs() <= 1.0e-10,
        "{actual} != {expected}"
    );
}

fn vector<const N: usize>(actual: [f64; N], expected: [f64; N]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        near(actual, expected);
    }
}

fn matrix(scale: [f32; 3], translation: [f32; 3]) -> Mat4 {
    let mut matrix = Mat4::IDENTITY;
    for lane in 0..3 {
        matrix.cols[lane][lane] = scale[lane];
        matrix.cols[3][lane] = translation[lane];
    }
    matrix
}

fn node(matrix: Mat4) -> Node {
    Node {
        name: None,
        transform: AuthoredTransform::try_matrix(matrix).unwrap(),
        children: Vec::new(),
        camera: None,
        light: None,
    }
}

fn asset() -> ApplicationAsset {
    ApplicationAsset {
        mesh_count: 1,
        mesh_morph_weights: vec![Vec::new()],
        roots: vec![0],
        nodes: vec![node(Mat4::IDENTITY)],
        cameras: Vec::new(),
        lights: Vec::new(),
        draws: vec![DrawBuffer {
            mesh: 0,
            primitive: 0,
            material: MaterialAssignment::Unspecified,
            topology: TopologyKind::Points,
            vertices: vec![Vertex {
                position: [1.0, 2.0, 3.0],
                normal: Some([1.0, 1.0, 0.0]),
                tangent: Some([1.0, -1.0, 0.0, 1.0]),
                color: None,
            }],
            indices: vec![0],
            influences: None,
            morphs: Vec::new(),
            uv_streams: Vec::new(),
            tangent_conventions: Some(TangentConventions {
                domain: AttributeDomain::Vertex,
                basis: TangentBasis::Unknown,
                encoding: TangentEncoding::SignInW,
            }),
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
        space: SceneSpaceRef {
            handedness: Handedness::Right,
            up_axis: Axis::PositiveY,
            front_axis: Axis::PositiveZ,
            unit_scale_to_meters: Some(1.0),
            front_face_winding: FrontFaceWinding::CounterClockwise,
            uv_origin: UvOrigin::UpperLeft,
        },
        source_json_metadata: Vec::new(),
    }
}

fn sampled(asset: &ApplicationAsset, frames: FramePolicy) -> Result<EvaluatedAsset> {
    Ok(evaluate(
        asset,
        None,
        SampleTime::Declared(0.0),
        request(frames),
    )?)
}

fn frame(draw: &EvaluatedDraw, normal: [f64; 3], tangent: [f64; 4], orientation: i8) {
    vector(draw.world_normals.as_ref().unwrap()[0], normal);
    vector(draw.world_tangents.as_ref().unwrap()[0], tangent);
    assert_eq!(
        draw.world_orientations.as_deref(),
        Some([orientation].as_slice())
    );
}

fn frame_failure(asset: &ApplicationAsset, reason: FrameFailure) {
    assert!(matches!(
        evaluate(asset, None, SampleTime::Declared(0.0), request(FramePolicy::FrozenWeight)),
        Err(EvaluationError::Frame { instance: 0, draw: 0, vertex: 0, reason: actual }) if actual == reason
    ));
}

#[test]
fn rigid_rotation_translates_positions_and_rotates_the_whole_frame() -> Result<()> {
    let mut asset = asset();
    let mut rigid = Mat4::IDENTITY;
    rigid.cols[0] = [0.0, 1.0, 0.0, 0.0];
    rigid.cols[1] = [-1.0, 0.0, 0.0, 0.0];
    rigid.cols[3] = [5.0, 6.0, 7.0, 1.0];
    asset.nodes[0] = node(rigid);
    let output = sampled(&asset, FramePolicy::FrozenWeight)?;
    let s = std::f64::consts::FRAC_1_SQRT_2;
    vector(output.draws[0].positions[0], [1.0, 2.0, 3.0]);
    vector(output.draws[0].world_positions[0], [3.0, 7.0, 10.0]);
    frame(&output.draws[0], [-s, s, 0.0], [s, s, 0.0, 1.0], 1);
    Ok(())
}

#[test]
fn nonuniform_scale_and_reflection_use_inverse_transpose_and_authored_handedness() -> Result<()> {
    let mut asset = asset();
    let s = 13.0_f64.sqrt();
    for reflection in [1.0_f32, -1.0] {
        asset.nodes[0] = node(matrix([2.0 * reflection, 3.0, 4.0], [0.0; 3]));
        for handedness in [1.0, -1.0] {
            asset.draws[0].vertices[0].tangent.as_mut().unwrap()[3] = handedness;
            let output = sampled(&asset, FramePolicy::FrozenWeight)?;
            let r = f64::from(reflection);
            vector(output.draws[0].world_positions[0], [2.0 * r, 6.0, 12.0]);
            frame(
                &output.draws[0],
                [3.0 * r / s, 2.0 / s, 0.0],
                [2.0 * r / s, -3.0 / s, 0.0, handedness * r],
                if r < 0.0 { -1 } else { 1 },
            );
        }
    }
    Ok(())
}

fn one_joint_morph(weight: f32) -> ApplicationAsset {
    let mut asset = asset();
    asset.nodes = vec![
        node(matrix([1.0; 3], [10.0, 0.0, 0.0])),
        node(matrix([2.0, 3.0, 4.0], [12.0, 0.0, 0.0])),
    ];
    asset.roots = vec![0, 1];
    asset.instances[0].skin = Some(0);
    asset.skins.push(Skin {
        joints: vec![1],
        inverse_binds: vec![matrix([1.0; 3], [-1.0, 0.0, 0.0])],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 1],
        joints: vec![0],
        weights: vec![weight],
    });
    asset.mesh_morph_weights[0] = vec![0.5];
    asset.draws[0].vertices[0].normal = Some([1.0, 0.0, 0.0]);
    asset.draws[0].vertices[0].tangent = Some([0.0, -1.0, 0.0, 1.0]);
    for (semantic, delta) in [
        (MorphDeltaSemantic::Position, [2.0, 0.0, 0.0]),
        (MorphDeltaSemantic::Normal, [0.0, 2.0, 0.0]),
        (MorphDeltaSemantic::Tangent, [2.0, 0.0, 2.0]),
    ] {
        asset.draws[0].morphs.push(MorphStream {
            target: 0,
            semantic,
            values: vec![delta],
        });
    }
    asset
}

#[test]
fn one_joint_applies_morph_before_skin_and_preserves_nonunit_homogeneous_weights() -> Result<()> {
    let s = 13.0_f64.sqrt();
    let t = 29.0_f64.sqrt();
    for weight in [1.0_f32, 2.0] {
        let asset = one_joint_morph(weight);
        let before = source_bits(&asset);
        let output = sampled(&asset, FramePolicy::FrozenWeight)?;
        let w = f64::from(weight);
        vector(
            output.draws[0].world_positions[0],
            [14.0 * w, 6.0 * w, 12.0 * w],
        );
        vector(output.draws[0].positions[0], [4.0 * w, 6.0 * w, 12.0 * w]);
        frame(
            &output.draws[0],
            [3.0 / s, 2.0 / s, 0.0],
            [2.0 / t, -3.0 / t, 4.0 / t, 1.0],
            1,
        );
        assert_eq!(source_bits(&asset), before);
    }
    Ok(())
}

#[test]
fn frame_blending_keeps_every_influence_in_its_original_row() -> Result<()> {
    let mut asset = asset();
    asset.nodes = vec![
        node(matrix([1.0; 3], [10.0, 0.0, 0.0])),
        node(matrix([2.0, 1.0, 1.0], [10.0, 0.0, 0.0])),
        node(matrix([1.0, 3.0, 1.0], [14.0, 0.0, 0.0])),
    ];
    asset.roots = vec![0, 1, 2];
    asset.instances[0].skin = Some(0);
    asset.skins.push(Skin {
        joints: vec![1, 2],
        inverse_binds: vec![Mat4::IDENTITY; 2],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 6],
        joints: vec![0, 1, 0, 1, 0, 1],
        weights: vec![0.25, 0.25, 0.25, 0.25, 0.25, 0.75],
    });
    let output = sampled(&asset, FramePolicy::FrozenWeight)?;
    vector(output.draws[0].world_positions[0], [27.75, 9.0, 6.0]);
    vector(output.draws[0].positions[0], [7.75, 9.0, 6.0]);
    let s = 445.0_f64.sqrt();
    frame(
        &output.draws[0],
        [18.0 / s, 11.0 / s, 0.0],
        [11.0 / s, -18.0 / s, 0.0, 1.0],
        1,
    );
    Ok(())
}

#[test]
fn frame_failures_are_named_and_position_only_does_not_claim_shading() -> Result<()> {
    for reason in [
        FrameFailure::DegenerateNormal,
        FrameFailure::DegenerateTangent,
        FrameFailure::MissingNormal,
        FrameFailure::UnsupportedTangentEncoding,
        FrameFailure::InvalidHandedness,
        FrameFailure::SingularLinearMap,
    ] {
        let mut asset = asset();
        match reason {
            FrameFailure::DegenerateNormal => asset.draws[0].vertices[0].normal = Some([0.0; 3]),
            FrameFailure::DegenerateTangent => {
                asset.draws[0].vertices[0].tangent = Some([1.0, 1.0, 0.0, 1.0])
            }
            FrameFailure::MissingNormal => asset.draws[0].vertices[0].normal = None,
            FrameFailure::UnsupportedTangentEncoding => asset.draws[0].tangent_conventions = None,
            FrameFailure::InvalidHandedness => {
                asset.draws[0].vertices[0].tangent.as_mut().unwrap()[3] = 0.0
            }
            FrameFailure::SingularLinearMap => {
                asset.nodes[0] = node(matrix([1.0, 1.0e-20, 1.0], [0.0; 3]))
            }
            _ => unreachable!(),
        }
        frame_failure(&asset, reason);
        let output = sampled(&asset, FramePolicy::PositionOnly)?;
        assert!(output.draws[0].world_normals.is_none());
        assert!(output.draws[0].world_tangents.is_none());
        assert!(output.draws[0].world_orientations.is_none());
    }
    let mut asset = asset();
    asset.draws[0].vertices[0].normal = None;
    asset.draws[0].vertices[0].tangent = None;
    let output = sampled(&asset, FramePolicy::FrozenWeight)?;
    assert!(output.draws[0].world_normals.is_none());
    assert!(output.draws[0].world_tangents.is_none());
    Ok(())
}

#[test]
fn empty_zero_or_cancelling_skin_rows_have_no_successful_lit_frame() -> Result<()> {
    let mut asset = one_joint_morph(0.0);
    frame_failure(&asset, FrameFailure::SingularLinearMap);
    vector(
        sampled(&asset, FramePolicy::PositionOnly)?.draws[0].world_positions[0],
        [0.0; 3],
    );
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 0],
        joints: Vec::new(),
        weights: Vec::new(),
    });
    frame_failure(&asset, FrameFailure::SingularLinearMap);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 2],
        joints: vec![0, 0],
        weights: vec![1.0, -1.0],
    });
    frame_failure(&asset, FrameFailure::SingularLinearMap);
    Ok(())
}

#[test]
fn active_position_morphs_require_explicit_normal_delta_coverage() -> Result<()> {
    let mut asset = one_joint_morph(1.0);
    asset.draws[0]
        .morphs
        .retain(|stream| stream.semantic != MorphDeltaSemantic::Normal);
    frame_failure(&asset, FrameFailure::MissingMorphShadingDeltas);
    sampled(&asset, FramePolicy::PositionOnly)?;
    asset.mesh_morph_weights[0][0] = 0.0;
    sampled(&asset, FramePolicy::FrozenWeight)?;
    asset.mesh_morph_weights[0][0] = 0.5;
    asset.draws[0].morphs.push(MorphStream {
        target: 0,
        semantic: MorphDeltaSemantic::Normal,
        values: vec![[0.0; 3]],
    });
    sampled(&asset, FramePolicy::FrozenWeight)?;
    Ok(())
}

#[test]
fn derivative_profile_rejects_nonzero_tangent_morphs_without_changing_the_position_path()
-> Result<()> {
    let mut asset = one_joint_morph(1.0);
    let frozen = sampled(&asset, FramePolicy::FrozenWeight)?;
    assert!(matches!(
        evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            request(FramePolicy::TriangleUvDerivative)
        ),
        Err(EvaluationError::Frame {
            reason: FrameFailure::UnsupportedTangentMorphNormalMap,
            ..
        })
    ));
    asset.draws[0]
        .morphs
        .iter_mut()
        .find(|stream| stream.semantic == MorphDeltaSemantic::Tangent)
        .unwrap()
        .values[0] = [0.0; 3];
    let derivative = sampled(&asset, FramePolicy::TriangleUvDerivative)?;
    assert!(derivative.draws[0].world_tangents.is_none());
    assert_eq!(
        frozen.draws[0].world_positions,
        derivative.draws[0].world_positions
    );
    asset.mesh_morph_weights[0][0] = 0.0;
    asset.draws[0]
        .morphs
        .iter_mut()
        .find(|stream| stream.semantic == MorphDeltaSemantic::Tangent)
        .unwrap()
        .values[0] = [2.0, 0.0, 0.0];
    sampled(&asset, FramePolicy::TriangleUvDerivative)?;
    Ok(())
}

#[test]
fn normal_only_requests_do_not_certify_unused_authored_tangents() -> Result<()> {
    let mut asset = one_joint_morph(1.0);
    let full = sampled(&asset, FramePolicy::FrozenWeight)?;
    // A=diag(2,3,4), morphed n=(1,1,0): A*(9,4,0) is parallel to A^-T*n.
    asset.draws[0].vertices[0].tangent = Some([9.0, 4.0, 0.0, 1.0]);
    asset.draws[0]
        .morphs
        .iter_mut()
        .find(|stream| stream.semantic == MorphDeltaSemantic::Tangent)
        .unwrap()
        .values[0] = [0.0; 3];
    frame_failure(&asset, FrameFailure::DegenerateTangent);
    let before = source_bits(&asset);
    for policy in [
        FramePolicy::FrozenWeightNormalsOnly,
        FramePolicy::TriangleUvDerivative,
    ] {
        let output = sampled(&asset, policy)?;
        assert_eq!(
            output.draws[0].world_positions,
            full.draws[0].world_positions
        );
        assert_eq!(output.draws[0].world_normals, full.draws[0].world_normals);
        assert_eq!(
            output.draws[0].world_orientations,
            full.draws[0].world_orientations
        );
        assert!(output.draws[0].world_tangents.is_none());
        asset.draws[0].vertices[0].normal = Some([0.0, -1.0, 0.0]);
        assert!(matches!(
            evaluate(&asset, None, SampleTime::Declared(0.0), request(policy)),
            Err(EvaluationError::Frame {
                reason: FrameFailure::DegenerateNormal,
                ..
            })
        ));
        asset.draws[0].vertices[0].normal = Some([1.0, 0.0, 0.0]);
    }
    assert_eq!(before, source_bits(&asset));
    Ok(())
}

#[test]
fn mixed_orientation_is_rejected_within_each_element_but_not_between_points() -> Result<()> {
    let mut asset = asset();
    asset.nodes.push(node(matrix([-1.0, 1.0, 1.0], [0.0; 3])));
    asset.roots.push(1);
    asset.instances[0].skin = Some(0);
    asset.skins.push(Skin {
        joints: vec![0, 1],
        inverse_binds: vec![Mat4::IDENTITY; 2],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    let draw = &mut asset.draws[0];
    draw.vertices = vec![draw.vertices[0].clone(); 3];
    draw.indices = vec![0, 1, 2];
    draw.influences = Some(Influences {
        offsets: vec![0, 1, 2, 3],
        joints: vec![0, 1, 0],
        weights: vec![1.0; 3],
    });
    draw.topology = TopologyKind::TriangleList;
    assert!(matches!(
        evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            request(FramePolicy::FrozenWeight)
        ),
        Err(EvaluationError::Frame {
            vertex: 1,
            reason: FrameFailure::MixedFrameOrientation,
            ..
        })
    ));
    sampled(&asset, FramePolicy::PositionOnly)?;
    asset.draws[0].topology = TopologyKind::Points;
    let output = sampled(&asset, FramePolicy::FrozenWeight)?;
    assert_eq!(
        output.draws[0].world_orientations.as_deref(),
        Some([1, -1, 1].as_slice())
    );
    Ok(())
}

fn output_capacity_bytes(output: &EvaluatedAsset) -> usize {
    let mut bytes = (output.local_transforms.capacity() + output.world_transforms.capacity())
        * size_of::<EvaluationMatrix>()
        + output.morph_weights.capacity() * size_of::<Vec<f64>>()
        + output.draws.capacity() * size_of::<EvaluatedDraw>();
    for weights in &output.morph_weights {
        bytes += weights.capacity() * size_of::<f64>();
    }
    for draw in &output.draws {
        bytes +=
            (draw.positions.capacity() + draw.world_positions.capacity()) * size_of::<[f64; 3]>();
        bytes += draw
            .world_normals
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<[f64; 3]>());
        bytes += draw
            .world_tangents
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<[f64; 4]>());
        bytes += draw
            .world_orientations
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<i8>());
    }
    bytes
}

#[test]
fn admission_covers_all_output_capacity_scratch_and_work_with_exact_boundaries() -> Result<()> {
    let asset = one_joint_morph(1.0);
    for policy in [
        FramePolicy::PositionOnly,
        FramePolicy::FrozenWeight,
        FramePolicy::FrozenWeightNormalsOnly,
    ] {
        let output = sampled(&asset, policy)?;
        let a = output.accounting;
        assert_eq!(a.output_bytes, output_capacity_bytes(&output));
        let limits = EvaluationLimits {
            max_output_vertices: a.output_vertices,
            max_output_draws: a.output_draws,
            max_palette_matrices: 1,
            max_output_bytes: a.output_bytes,
            max_scratch_bytes: a.scratch_bytes,
            max_total_bytes: a.output_bytes + a.scratch_bytes,
            max_work_units: a.work_units,
        };
        evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: policy,
                limits,
                cancellation: None,
            },
        )?;
        for limit in [
            EvaluationLimit::OutputVertices,
            EvaluationLimit::OutputDraws,
            EvaluationLimit::PaletteMatrices,
            EvaluationLimit::OutputBytes,
            EvaluationLimit::ScratchBytes,
            EvaluationLimit::TotalBytes,
            EvaluationLimit::Work,
        ] {
            let mut smaller = limits;
            match limit {
                EvaluationLimit::OutputVertices => smaller.max_output_vertices -= 1,
                EvaluationLimit::OutputDraws => smaller.max_output_draws -= 1,
                EvaluationLimit::PaletteMatrices => smaller.max_palette_matrices -= 1,
                EvaluationLimit::OutputBytes => smaller.max_output_bytes -= 1,
                EvaluationLimit::ScratchBytes => smaller.max_scratch_bytes -= 1,
                EvaluationLimit::TotalBytes => smaller.max_total_bytes -= 1,
                EvaluationLimit::Work => smaller.max_work_units -= 1,
            }
            assert!(
                matches!(
                    evaluate(&asset, None, SampleTime::Declared(0.0), EvaluationRequest {
                        frames: policy, limits: smaller, cancellation: None,
                    }),
                    Err(EvaluationError::LimitExceeded { limit: actual }) if actual == limit
                ),
                "{policy:?} {limit:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn output_counts_include_each_instance_and_cancellation_precedes_admission() -> Result<()> {
    let mut asset = asset();
    asset.instances.push(Instance {
        node: 0,
        mesh: 0,
        skin: None,
        morph_weights: None,
    });
    let mut req = request(FramePolicy::FrozenWeight);
    req.limits.max_output_vertices = 1;
    assert!(matches!(
        evaluate(&asset, None, SampleTime::Declared(0.0), req),
        Err(EvaluationError::LimitExceeded {
            limit: EvaluationLimit::OutputVertices
        })
    ));
    let output = sampled(&asset, FramePolicy::FrozenWeight)?;
    assert_eq!(output.accounting.output_vertices, 2);
    assert_eq!(output.accounting.output_draws, 2);
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        evaluate(
            &asset,
            Some(usize::MAX),
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: FramePolicy::FrozenWeight,
                limits: EvaluationLimits::default(),
                cancellation: Some(&token),
            }
        ),
        Err(EvaluationError::Cancelled)
    ));
    Ok(())
}

#[test]
fn channel_validation_is_charged_before_sampling_large_unchanged_tables() -> Result<()> {
    let mut asset = asset();
    asset.animations.push(Clip {
        name: None,
        duration: 2047.0,
        time_domain: AnimationTimeDomain::Seconds,
        channels: vec![Channel {
            target: ChannelTarget::MorphWeights(0),
            interpolation: AnimationInterpolation::Linear,
            pre_extrapolation: AnimationExtrapolation::Constant,
            post_extrapolation: AnimationExtrapolation::Constant,
            width: 1,
            times: (0..2048).map(f64::from).collect(),
            values: vec![0.0; 2048],
            in_tangents: None,
            out_tangents: None,
        }],
    });
    asset.mesh_morph_weights[0] = vec![0.0];
    let mut req = request(FramePolicy::PositionOnly);
    req.limits.max_work_units = 100;
    assert!(matches!(
        evaluate(&asset, Some(0), SampleTime::Declared(0.0), req),
        Err(EvaluationError::LimitExceeded {
            limit: EvaluationLimit::Work
        })
    ));
    Ok(())
}

fn detached(name: &str) -> Result<ApplicationAsset> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let io = FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(root)?);
    let importer = Importer::new()?;
    let report = importer.read_asset(
        &io,
        AssetPath::new(format!("upstream/Models/{name}/glTF/{name}.gltf"))?,
    )?;
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(evidence);
    drop(importer);
    drop(io);
    Ok(asset)
}

// Observe source floating-point bits, including authored TRS rather than only its matrix.
fn source_bits(asset: &ApplicationAsset) -> Vec<u64> {
    let mut bits = Vec::new();
    for node in &asset.nodes {
        bits.extend(
            node.transform
                .local_matrix()
                .cols
                .into_iter()
                .flatten()
                .map(|v| u64::from(v.to_bits())),
        );
        if let Some(trs) = node.transform.authored_trs() {
            let t = trs.translation();
            let q = trs.rotation();
            let s = trs.scale();
            bits.extend(
                [t.x, t.y, t.z, q.x, q.y, q.z, q.w, s.x, s.y, s.z].map(|v| u64::from(v.to_bits())),
            );
        }
    }
    for weights in asset.mesh_morph_weights.iter().chain(
        asset
            .instances
            .iter()
            .filter_map(|i| i.morph_weights.as_ref()),
    ) {
        bits.extend(weights.iter().map(|v| u64::from(v.to_bits())));
    }
    for skin in &asset.skins {
        for matrix in &skin.inverse_binds {
            bits.extend(
                matrix
                    .cols
                    .into_iter()
                    .flatten()
                    .map(|v| u64::from(v.to_bits())),
            );
        }
    }
    for draw in &asset.draws {
        for vertex in &draw.vertices {
            bits.extend(vertex.position.map(f64::to_bits));
            bits.extend(vertex.normal.into_iter().flatten().map(f64::to_bits));
            bits.extend(vertex.tangent.into_iter().flatten().map(f64::to_bits));
        }
        for stream in &draw.morphs {
            bits.extend(
                stream
                    .values
                    .iter()
                    .flatten()
                    .map(|v| u64::from(v.to_bits())),
            );
        }
        if let Some(rows) = &draw.influences {
            bits.extend(rows.weights.iter().map(|v| u64::from(v.to_bits())));
        }
    }
    for clip in &asset.animations {
        for channel in &clip.channels {
            bits.extend(channel.times.iter().map(|v| v.to_bits()));
            for values in std::iter::once(&channel.values)
                .chain(channel.in_tangents.iter())
                .chain(channel.out_tangents.iter())
            {
                bits.extend(values.iter().map(|v| u64::from(v.to_bits())));
            }
        }
    }
    bits
}

fn corner_vertex(asset: &ApplicationAsset, corner: usize) -> usize {
    // Source index-corners survive extraction; a packed vertex ordinal is not a source ID.
    usize::try_from(asset.draws[0].indices[corner]).unwrap()
}

#[test]
fn pinned_rigged_simple_matches_raw_source_positions_and_normals_at_key_and_midpoint() -> Result<()>
{
    // Khronos glTF-Sample-Assets 8f492390ddc7a2ae221a1369ef560589cf8c70fb.
    // POSITION/NORMAL/JOINTS/WEIGHTS = accessors 3/2/1/4; binds = 9; times = 5.
    // Independent LE U16/F32 decoding and cofactor normal calculation, not Baozi output.
    // Blob SHA-256 identities are recorded in fixtures/SHA256SUMS and the private U10 note.
    let asset = detached("RiggedSimple")?;
    let before = source_bits(&asset);
    let v2 = corner_vertex(&asset, 2);
    let v159 = corner_vertex(&asset, 427);
    for (time, p2, n2, p159, n159) in [
        (
            1.0833330154418945,
            [
                0.09166741870906292,
                -0.009324670632406397,
                0.4799818125035087,
            ],
            [0.2077451121600896, 0.03170394224191382, 0.9776690791980245],
            [2.348135485919336, 3.947126532032868, 0.4158198015543262],
            [
                0.5446350752436382,
                0.8386731394377408,
                2.3421896532356733e-8,
            ],
        ),
        (
            1.0625,
            [
                0.09153523183226238,
                -0.009487634519256053,
                0.47998181249481625,
            ],
            [0.2081027722664984, 0.03114086090208392, 0.977611110287355],
            [2.3946026263363884, 3.919394367212576, 0.4158198032957121],
            [
                0.5545216156306101,
                0.8321693203900379,
                2.3783078037406967e-8,
            ],
        ),
    ] {
        let output = evaluate(
            &asset,
            Some(0),
            SampleTime::Declared(time),
            request(FramePolicy::FrozenWeight),
        )?;
        vector(output.draws[0].world_positions[v2], p2);
        vector(output.draws[0].world_positions[v159], p159);
        vector(output.draws[0].world_normals.as_ref().unwrap()[v2], n2);
        vector(output.draws[0].world_normals.as_ref().unwrap()[v159], n159);
        assert!(output.draws[0].world_tangents.is_none());
    }
    assert_eq!(before, source_bits(&asset));
    Ok(())
}

#[test]
fn pinned_morph_cube_matches_raw_source_positions_and_changing_shading_at_key_and_midpoint()
-> Result<()> {
    // Same upstream pin. POSITION/NORMAL/TANGENT = 2/0/1; target deltas = 3..8;
    // times/weights = 10/11. Static quaternion widens source F32 without normalization.
    let asset = detached("AnimatedMorphCube")?;
    let before = source_bits(&asset);
    let v2 = corner_vertex(&asset, 0);
    let v0 = corner_vertex(&asset, 2);
    let v14 = corner_vertex(&asset, 18);
    for (time, weights, y2, p2, p14, n14) in [
        (
            2.09999942779541,
            [0.7233077883720398, 0.2766921818256378],
            0.0036940489635702844,
            [
                -1.0000000023234414,
                -0.9999999930525295,
                -0.36940468089691364,
            ],
            [-1.0000002817201947, 0.9999998198406057, -0.9197691214789313],
            [0.0, 0.20757639232310912, 0.9782188105685368],
        ),
        (
            2.116666078567505,
            [0.709390789270401, 0.2906091958284378],
            0.003430564951432624,
            [
                -1.0000000023234414,
                -0.9999999895123886,
                -0.34305628058501514,
            ],
            [-1.0000002817201947, 0.999999819661411, -0.9211028238497173],
            [0.0, 0.21845065776748473, 0.9758479953973126],
        ),
    ] {
        // Packed vertex 8 is source vertex 10 (first source corner 12). Its authored
        // normal is -X, tangent is +X and both tangent deltas are zero. Full tangent
        // evaluation must fail, while this untextured request needs deformed normals only.
        let degenerate = corner_vertex(&asset, 12);
        assert_eq!(degenerate, 8);
        vector(
            asset.draws[0].vertices[degenerate].normal.unwrap(),
            [-1.0, 0.0, -0.0],
        );
        vector(
            asset.draws[0].vertices[degenerate].tangent.unwrap(),
            [1.0, 0.0, -0.0, 1.0],
        );
        for stream in &asset.draws[0].morphs {
            if stream.semantic == MorphDeltaSemantic::Tangent {
                assert!(stream.values[degenerate].iter().all(|value| *value == 0.0));
            }
        }
        assert!(matches!(
            evaluate(
                &asset,
                Some(0),
                SampleTime::Declared(time),
                request(FramePolicy::FrozenWeight)
            ),
            Err(EvaluationError::Frame {
                instance: 0,
                draw: 0,
                vertex: 8,
                reason: FrameFailure::DegenerateTangent
            })
        ));
        let output = evaluate(
            &asset,
            Some(0),
            SampleTime::Declared(time),
            request(FramePolicy::FrozenWeightNormalsOnly),
        )?;
        vector(
            [output.morph_weights[0][0], output.morph_weights[0][1]],
            weights,
        );
        vector(
            output.draws[0].positions[v2],
            [0.010000000707805157, y2, 0.009999999776482582],
        );
        vector(
            output.draws[0].positions[v0],
            [
                -0.009999999776482582,
                0.009999998845160007,
                0.009999999776482582,
            ],
        );
        vector(output.draws[0].world_positions[v2], p2);
        vector(output.draws[0].world_positions[v14], p14);
        vector(output.draws[0].world_normals.as_ref().unwrap()[v14], n14);
        vector(
            output.draws[0].world_normals.as_ref().unwrap()[degenerate],
            [1.0, 0.0, 0.0],
        );
        assert!(output.draws[0].world_tangents.is_none());
        assert!(
            output.draws[0]
                .world_orientations
                .as_ref()
                .unwrap()
                .iter()
                .all(|v| *v == 1)
        );
    }
    assert_eq!(before, source_bits(&asset));
    Ok(())
}
