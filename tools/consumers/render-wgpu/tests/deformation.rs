mod common;
use baozi::{AttributeDomain, AuthoredTransform, Mat4, ShadingModel, SkinMatrixSpaces};
use baozi_asset_consumer::{
    ApplicationAsset, EvaluationError, EvaluationLimits, EvaluationRequest, FrameFailure,
    FramePolicy, Influences, Instance, Node, SampleTime, Skin, TangentBasis, TangentConventions,
    TangentEncoding, evaluate,
};
use baozi_render_wgpu::{
    Control, Error,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan, SOURCE_TANGENT, WORLD_TANGENT},
};

fn matrix(x: f32, y: f32, z: f32) -> AuthoredTransform {
    AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, y, z, 1.0],
        ],
    })
    .unwrap()
}
fn lit(asset: &mut ApplicationAsset) {
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
}

#[test]
fn nonunit_complete_rows_upload_evaluator_world_xyz_without_instance_reapplication() {
    let mut asset = common::asset(3);
    lit(&mut asset);
    for vertex in &mut asset.draws[0].vertices {
        vertex.position = [1.0, 0.0, 0.0];
    }
    asset.nodes[0].transform = matrix(10.0, 0.0, 0.0);
    asset.nodes.extend([
        Node {
            name: None,
            transform: matrix(2.0, 0.0, 0.0),
            children: vec![],
            camera: None,
            light: None,
        },
        Node {
            name: None,
            transform: matrix(8.0, 0.0, 0.0),
            children: vec![],
            camera: None,
            light: None,
        },
    ]);
    asset.roots = vec![0, 1, 2];
    asset.skins.push(Skin {
        joints: vec![1, 2],
        inverse_binds: vec![Mat4::IDENTITY; 2],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.instances[0].skin = Some(0);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 2, 4, 6],
        joints: vec![0, 1, 0, 1, 0, 1],
        weights: vec![0.5, 1.5, 0.5, 1.5, 0.5, 1.5],
    });
    let before = asset.draws[0]
        .influences
        .as_ref()
        .unwrap()
        .weights
        .iter()
        .map(|w| w.to_bits())
        .collect::<Vec<_>>();
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
    // Independently: .5*(1+2)+1.5*(1+8)=15, weighted W=2, inverse(T10) -> local -5.
    assert_eq!(evaluation.draws[0].world_positions[0], [15.0, 0.0, 0.0]);
    assert_eq!(evaluation.draws[0].positions[0], [-5.0, 0.0, 0.0]);
    let frames = evaluated_frames(&evaluation).unwrap();
    assert_eq!(frames[0].normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(frames[0].orientations[0], 1);
    let plan = DevicePlan::prepare(
        &asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )
    .unwrap();
    let packed = &plan.chunks()[0].vertices;
    assert_eq!(f32::from_le_bytes(packed[..4].try_into().unwrap()), 15.0);
    assert_ne!(f32::from_le_bytes(packed[..4].try_into().unwrap()), 5.0);
    assert_eq!(
        before,
        asset.draws[0]
            .influences
            .as_ref()
            .unwrap()
            .weights
            .iter()
            .map(|w| w.to_bits())
            .collect::<Vec<_>>()
    );
}

#[test]
fn static_nonuniform_reflection_uses_only_evaluator_frame_authority() {
    let mut asset = common::asset(6);
    lit(&mut asset);
    let unit = 1.0 / 2.0f64.sqrt();
    for vertex in &mut asset.draws[0].vertices {
        vertex.normal = Some([unit, unit, 0.0]);
        vertex.tangent = Some([unit, -unit, 0.0, 1.0]);
    }
    asset.draws[0].tangent_conventions = Some(TangentConventions {
        domain: AttributeDomain::Vertex,
        basis: TangentBasis::Unknown,
        encoding: TangentEncoding::SignInW,
    });
    asset.nodes[0].transform = AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [-2.0, 0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0, 0.0],
            [0.0, 0.0, 4.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    })
    .unwrap();
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
    let root = 13.0f64.sqrt();
    for (actual, expected) in frames[0].normals[0]
        .into_iter()
        .zip([-3.0 / root, 2.0 / root, 0.0])
    {
        assert!((actual - expected).abs() < 1e-12);
    }
    let tangent = frames[0].tangents.unwrap()[0];
    for (actual, expected) in tangent
        .into_iter()
        .zip([-2.0 / root, -3.0 / root, 0.0, -1.0])
    {
        assert!((actual - expected).abs() < 1e-12);
    }
    assert_eq!(frames[0].orientations, &[-1; 6]);
    let limits = DeviceLimits {
        max_vertices_per_chunk: 3,
        ..Default::default()
    };
    let plan =
        DevicePlan::prepare(&asset, &evaluation, &frames, limits, Control::default()).unwrap();
    assert_eq!(plan.chunks().len(), 2);
    for (element, chunk) in plan.chunks().iter().enumerate() {
        assert_eq!(chunk.elements, element..element + 1);
        assert_eq!(
            chunk.vertex_sources,
            asset.draws[0].indices[element * 3..element * 3 + 3]
        );
        assert_eq!(chunk.orientation, -1);
        assert_ne!(chunk.field_flags & WORLD_TANGENT, 0);
        for local in 0..chunk.vertex_sources.len() {
            for (offset, expected) in [
                (16, [-3.0 / root, 2.0 / root, 0.0, 0.0]),
                (32, [-2.0 / root, -3.0 / root, 0.0, -1.0]),
            ] {
                for (lane, value) in expected.into_iter().enumerate() {
                    let start = local * 64 + offset + lane * 4;
                    assert_eq!(
                        f32::from_le_bytes(chunk.vertices[start..start + 4].try_into().unwrap()),
                        value as f32
                    );
                }
            }
        }
    }
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
}

#[test]
fn explicit_normals_only_preserves_unused_parallel_source_tangent_without_world_claim() {
    let mut asset = common::asset(3);
    lit(&mut asset);
    for vertex in &mut asset.draws[0].vertices {
        vertex.tangent = Some([0.0, 0.0, 1.0, 1.0]);
    }
    asset.draws[0].tangent_conventions = Some(TangentConventions {
        domain: AttributeDomain::Vertex,
        basis: TangentBasis::Unknown,
        encoding: TangentEncoding::SignInW,
    });
    assert!(matches!(
        evaluate(
            &asset,
            None,
            SampleTime::Declared(0.0),
            EvaluationRequest {
                frames: FramePolicy::FrozenWeight,
                limits: EvaluationLimits::default(),
                cancellation: None
            }
        ),
        Err(EvaluationError::Frame {
            reason: FrameFailure::DegenerateTangent,
            ..
        })
    ));
    let evaluation = evaluate(
        &asset,
        None,
        SampleTime::Declared(0.0),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeightNormalsOnly,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )
    .unwrap();
    assert!(evaluation.draws[0].world_tangents.is_none());
    let frames = evaluated_frames(&evaluation).unwrap();
    let plan = DevicePlan::prepare(
        &asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )
    .unwrap();
    let chunk = &plan.chunks()[0];
    assert_ne!(chunk.field_flags & SOURCE_TANGENT, 0);
    assert_eq!(chunk.field_flags & WORLD_TANGENT, 0);
    for (local, &source) in chunk.vertex_sources.iter().enumerate() {
        let authored = asset.draws[0].vertices[source as usize].tangent.unwrap();
        for (lane, value) in authored.into_iter().enumerate() {
            let offset = local * 64 + 32 + lane * 4;
            assert_eq!(
                f32::from_le_bytes(chunk.vertices[offset..offset + 4].try_into().unwrap())
                    .to_bits(),
                (value as f32).to_bits()
            );
        }
    }
}

#[test]
fn shared_mesh_uses_each_instances_current_skin_and_evaluator_frames() {
    let mut asset = common::asset(3);
    lit(&mut asset);
    asset.nodes.extend([
        Node {
            name: None,
            transform: matrix(2.0, 0.0, 0.0),
            children: vec![],
            camera: None,
            light: None,
        },
        Node {
            name: None,
            transform: matrix(8.0, 0.0, 0.0),
            children: vec![],
            camera: None,
            light: None,
        },
    ]);
    asset.roots = vec![0, 1, 2];
    asset.skins = vec![
        Skin {
            joints: vec![1],
            inverse_binds: vec![Mat4::IDENTITY],
            matrix_spaces: SkinMatrixSpaces::canonical(),
        },
        Skin {
            joints: vec![2, 1],
            inverse_binds: vec![Mat4::IDENTITY; 2],
            matrix_spaces: SkinMatrixSpaces::canonical(),
        },
    ];
    asset.instances[0].skin = Some(0);
    asset.instances.push(Instance {
        node: 0,
        mesh: 0,
        skin: Some(1),
        morph_weights: None,
    });
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 1, 2, 3],
        joints: vec![0; 3],
        weights: vec![1.0; 3],
    });
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
    assert_eq!(plan.chunks().len(), 2);
    for (instance, chunk) in plan.chunks().iter().enumerate() {
        assert_eq!(chunk.instance, instance);
        assert_eq!(chunk.draw, 0);
        assert_eq!(chunk.palette_source_joints, [0]);
        for (local, &source) in chunk.vertex_sources.iter().enumerate() {
            let x = f32::from_le_bytes(
                chunk.vertices[local * 64..local * 64 + 4]
                    .try_into()
                    .unwrap(),
            );
            let expected =
                asset.draws[0].vertices[source as usize].position[0] + [2.0, 8.0][instance];
            assert_eq!(x, expected as f32);
        }
    }
    drop(plan);
    // Joint 1 exists in skin 1, but not in the first placement's one-joint skin.
    asset.draws[0].influences.as_mut().unwrap().joints[0] = 1;
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("influence joint outside instance skin"))
    ));
}

#[test]
fn post_evaluation_joint_edits_reject_positive_and_zero_weight_references() {
    let mut asset = common::asset(4);
    lit(&mut asset);
    asset.draws[0].indices = vec![0, 1, 2];
    asset.skins.push(Skin {
        joints: vec![0],
        inverse_binds: vec![Mat4::IDENTITY],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.instances[0].skin = Some(0);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 2, 3, 4, 5],
        joints: vec![0; 5],
        weights: vec![1.0, 0.0, 1.0, 1.0, 1.0],
    });
    let evaluation = common::evaluated(&asset);
    let frames = evaluated_frames(&evaluation).unwrap();
    for entry in [0, 1, 4] {
        // Includes the retained, unused fourth vertex.
        asset.draws[0].influences.as_mut().unwrap().joints[entry] = 1;
        assert!(matches!(
            DevicePlan::prepare(
                &asset,
                &evaluation,
                &frames,
                DeviceLimits::default(),
                Control::default()
            ),
            Err(Error::InvalidAsset("influence joint outside instance skin"))
        ));
        asset.draws[0].influences.as_mut().unwrap().joints[entry] = 0;
    }
    asset.skins[0].joints[0] = asset.nodes.len();
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("skin joint node"))
    ));
    asset.skins[0].joints[0] = 0;
    asset.skins[0].inverse_binds.clear();
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("skin inverse-bind correspondence"))
    ));
}

#[test]
fn post_evaluation_removed_rows_reject_current_skinned_placement() {
    let mut asset = common::asset(3);
    lit(&mut asset);
    asset.skins.push(Skin {
        joints: vec![0],
        inverse_binds: vec![Mat4::IDENTITY],
        matrix_spaces: SkinMatrixSpaces::canonical(),
    });
    asset.instances[0].skin = Some(0);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 1, 2, 3],
        joints: vec![0; 3],
        weights: vec![1.0; 3],
    });
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
    assert!(plan.chunks()[0].influences.is_some());
    drop(plan);
    asset.draws[0].influences = None;
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluation,
            &frames,
            DeviceLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("skinned draw has no influences"))
    ));
}
