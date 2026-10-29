use std::sync::atomic::AtomicU64;

use baozi_operation::PhysicalStorageAccount;

use super::gltf_capability::build_gltf_capability_scene_for_probe;
use super::{profile_fixture, successful_parts, test_account, test_factory, test_limits};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneSuccessParts, NodeSceneTestTransaction,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring as Extrapolation,
    AnimationInterpolationAuthoring as Interpolation, AnimationTargetAuthoring as Target,
    AnimationTimeDomainAuthoring as TimeDomain, AppearanceAuthoringBatch, AuthoringRange,
    DeformationAuthoringBatch, GeometryAuthoringBatch, NodeAuthoringBatch, NodeAuthoringNode,
    NodeLocalId, Presence, SceneAuthoringBatch, SpatialAuthoringBatch,
};
use crate::{
    AnimationClipId, AnimationExtrapolation, AnimationInterpolation, AnimationTarget,
    AnimationTimeDomain, AuthoredTransform, SceneEntityKind, SceneLookupError,
    SceneValidationLimits, ValidatedScene,
};

fn build_animation_scene(account: &PhysicalStorageAccount) -> NodeSceneSuccessParts {
    build_animation_scene_with_names(account, [Some("步行"), Some(""), None, Some("Run")])
}

fn build_animation_scene_with_names(
    account: &PhysicalStorageAccount,
    names: [Option<&str>; 4],
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let node_rows = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &node_rows);
    let channels = [
        AnimationChannelAuthoring::new(
            3,
            Target::NodeTranslation(NodeLocalId::new(1)),
            Interpolation::Linear,
            Extrapolation::Constant,
            Extrapolation::Linear,
            &[0.12500000000000003, 1.0000000000000002],
            &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            8,
            Target::NodeScale(NodeLocalId::new(0)),
            Interpolation::CubicSpline,
            Extrapolation::Repeat,
            Extrapolation::Constant,
            &[12.25, 24.125],
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            Presence::Present(&[-1.0, -2.0, -3.0, -4.0, -5.0, -6.0]),
            Presence::Present(&[0.25, 0.5, 0.75, 1.0, 1.25, 1.5]),
        ),
        AnimationChannelAuthoring::new(
            12,
            Target::NodeRotation(NodeLocalId::new(1)),
            Interpolation::Step,
            Extrapolation::SourceDefault,
            Extrapolation::Repeat,
            &[0.0, 30.0, 120.0],
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            45,
            Target::NodeTranslation(NodeLocalId::new(0)),
            Interpolation::Linear,
            Extrapolation::Linear,
            Extrapolation::SourceDefault,
            &[48000.125, 96000.125],
            &[9.0, 8.0, 7.0, 6.0, 5.0, 4.0],
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let mut clips = [
        AnimationClipAuthoring::new(
            7,
            1.0000000000000002,
            TimeDomain::Seconds,
            AuthoringRange::new(0, 1),
        ),
        AnimationClipAuthoring::new(
            11,
            2400.1250000000005,
            TimeDomain::Frames {
                rate: 29.97002997002997,
            },
            AuthoringRange::new(1, 2),
        ),
        AnimationClipAuthoring::new(
            19,
            4294967296.125,
            TimeDomain::Ticks { rate: None },
            AuthoringRange::new(3, 0),
        ),
        AnimationClipAuthoring::new(
            25,
            96000.125,
            TimeDomain::Ticks {
                rate: Some(48000.25),
            },
            AuthoringRange::new(3, 1),
        ),
    ];
    for (clip, name) in clips.iter_mut().zip(names) {
        if let Some(name) = name {
            *clip = clip.with_name(name);
        }
    }
    let animation = AnimationAuthoringBatch::new(&clips, &channels);
    let spatial = SpatialAuthoringBatch::new(&crate::SceneSpace::UNKNOWN, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let profile = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    successful_parts(
        NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            SceneAuthoringBatch::new(
                spatial,
                nodes,
                appearance,
                geometry,
                deformation,
                animation,
                profile.batch(),
            ),
            SceneValidationLimits::default(),
        )
        .expect("animation fixture admission")
        .finish_draft()
        .expect("animation fixture activation")
        .publish_for_test()
        .expect("animation fixture publication"),
    )
}

#[test]
fn animation_readers_preserve_names_time_domains_and_exact_payloads_without_allocation() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let parts = build_animation_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        assert_eq!(scene.animation_clip_count(), 4);
        let node_zero = scene.nodes().next().unwrap().id();
        let node_one = scene.nodes().nth(1).unwrap().id();

        // Read the last clip first: preceding cubic and non-cubic rows have different offsets.
        let last = scene.animation_clips().next_back().unwrap();
        assert_eq!(last.name(), Some("Run"));
        assert_eq!(last.duration(), 96000.125);
        assert_eq!(
            last.time_domain(),
            AnimationTimeDomain::Ticks {
                rate: Some(48000.25)
            }
        );
        let channel = last.channel(0).unwrap();
        assert_eq!(
            channel.target(),
            AnimationTarget::NodeTranslation(node_zero)
        );
        assert_eq!(channel.key_times(), &[48000.125, 96000.125]);
        assert_eq!(channel.values(), &[9.0, 8.0, 7.0, 6.0, 5.0, 4.0]);
        assert_eq!(channel.in_tangents(), None);
        assert_eq!(channel.out_tangents(), None);
        assert_eq!(channel.pre_extrapolation(), AnimationExtrapolation::Linear);
        assert_eq!(
            channel.post_extrapolation(),
            AnimationExtrapolation::SourceDefault
        );

        let first = scene.animation_clips().next().unwrap();
        assert_eq!(first.id().index(), 0);
        assert_eq!(first.name(), Some("步行"));
        assert_eq!(first.duration(), 1.0000000000000002);
        assert_eq!(first.time_domain(), AnimationTimeDomain::Seconds);
        assert_eq!(first.channel_count(), 1);
        let first_channel = first.channel(0).unwrap();
        assert_eq!(
            first_channel.target(),
            AnimationTarget::NodeTranslation(node_one)
        );
        assert_eq!(
            first_channel.interpolation(),
            AnimationInterpolation::Linear
        );
        assert_eq!(first_channel.key_count(), 2);
        assert_eq!(first_channel.value_width(), 3);
        assert_eq!(
            first_channel.key_times(),
            &[0.12500000000000003, 1.0000000000000002]
        );
        assert_eq!(first_channel.values(), &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(
            first_channel.pre_extrapolation(),
            AnimationExtrapolation::Constant
        );
        assert_eq!(
            first_channel.post_extrapolation(),
            AnimationExtrapolation::Linear
        );

        let frames = scene.animation_clips().nth(1).unwrap();
        assert_eq!(frames.name(), Some(""));
        assert_eq!(frames.duration(), 2400.1250000000005);
        assert_eq!(
            frames.time_domain(),
            AnimationTimeDomain::Frames {
                rate: 29.97002997002997
            }
        );
        let cubic = frames.channel(0).unwrap();
        assert_eq!(cubic.target(), AnimationTarget::NodeScale(node_zero));
        assert_eq!(cubic.interpolation(), AnimationInterpolation::CubicSpline);
        assert_eq!(cubic.key_times(), &[12.25, 24.125]);
        assert_eq!(cubic.value_width(), 3);
        assert_eq!(cubic.values(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(
            cubic.in_tangents(),
            Some(&[-1.0, -2.0, -3.0, -4.0, -5.0, -6.0][..])
        );
        assert_eq!(
            cubic.out_tangents(),
            Some(&[0.25, 0.5, 0.75, 1.0, 1.25, 1.5][..])
        );
        assert_eq!(cubic.pre_extrapolation(), AnimationExtrapolation::Repeat);
        assert_eq!(cubic.post_extrapolation(), AnimationExtrapolation::Constant);
        let rotation = frames.channel(1).unwrap();
        assert_eq!(rotation.target(), AnimationTarget::NodeRotation(node_one));
        assert_eq!(rotation.interpolation(), AnimationInterpolation::Step);
        assert_eq!(rotation.value_width(), 4);
        assert_eq!(rotation.key_count(), 3);
        assert_eq!(rotation.key_times(), &[0.0, 30.0, 120.0]);
        assert_eq!(
            rotation.values(),
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(rotation.in_tangents(), None);
        assert_eq!(rotation.out_tangents(), None);
        assert_eq!(
            rotation.pre_extrapolation(),
            AnimationExtrapolation::SourceDefault
        );
        assert_eq!(
            rotation.post_extrapolation(),
            AnimationExtrapolation::Repeat
        );
        assert!(frames.channel(2).is_none());
        assert!(frames.channel(usize::MAX).is_none());

        let empty = scene.animation_clips().nth(2).unwrap();
        assert_eq!(empty.name(), None);
        assert_eq!(empty.duration(), 4294967296.125);
        assert_eq!(
            empty.time_domain(),
            AnimationTimeDomain::Ticks { rate: None }
        );
        assert_eq!(empty.channel_count(), 0);
        assert_eq!(empty.channels().len(), 0);
        assert!(empty.channel(0).is_none());

        let mut one = frames.channels();
        let mut two = frames.channels();
        assert_eq!(one.len(), 2);
        assert_eq!(one.next_back().unwrap().target(), rotation.target());
        assert_eq!(two.next().unwrap().target(), cubic.target());
        assert_eq!(one.next().unwrap().target(), cubic.target());
        assert!(one.next().is_none());
        assert_eq!(two.len(), 1);
        assert!(std::ptr::eq(
            cubic.values().as_ptr(),
            frames.channels().next().unwrap().values().as_ptr()
        ));
        assert!(std::ptr::eq(
            cubic.key_times().as_ptr(),
            frames.channel(0).unwrap().key_times().as_ptr()
        ));
        assert_eq!(scene.animation_clip(last.id()).unwrap().id(), last.id());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn animation_clip_lookup_checks_generation_and_range() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let parts = build_animation_scene(&account);
    let other_parts = build_animation_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let other = ValidatedScene::from_node_scene(other_parts.scene);
    let clip = scene.animation_clips().next().unwrap();
    assert!(matches!(
        other.animation_clip(clip.id()),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::AnimationClip,
            ..
        })
    ));
    assert!(matches!(
        scene.animation_clip(AnimationClipId::new(scene.generation(), 4)),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::AnimationClip,
            length: 4
        })
    ));
    assert!(matches!(
        scene.animation_clip(AnimationClipId::new(scene.generation(), u32::MAX)),
        Err(SceneLookupError::OutOfRange { .. })
    ));
    drop(scene);
    drop(other);
    drop(parts.report);
    drop(other_parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn animation_name_digest_distinguishes_absence_empty_and_utf8_contents() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let names = [None, Some(""), Some("步行"), Some("步跑")];
    let digests: [[u8; 32]; 4] = std::array::from_fn(|index| {
        let parts = build_animation_scene_with_names(&account, [names[index], None, None, None]);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let allocations = allocation_counter::measure(|| {
            assert_eq!(scene.animation_clips().next().unwrap().name(), names[index]);
        });
        assert_eq!(allocations.count_total, 0);
        let snapshot = scene
            .capture_snapshot(crate::SceneSnapshotLimits::try_new(16384, 1024, 1_000_000).unwrap())
            .unwrap();
        let digest = snapshot.digest();
        drop(scene);
        drop(parts.report);
        digest
    });
    for index in 0..digests.len() {
        for previous in 0..index {
            assert_ne!(digests[index], digests[previous]);
        }
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn animation_morph_targets_resolve_mesh_instance_ids_and_borrow_cubic_payloads() {
    let factory = test_factory();
    let account = test_account(&factory, 4096);
    let parts = build_gltf_capability_scene_for_probe(&account, &AtomicU64::new(0));
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.id().index(), 0);
    assert_eq!(clip.name(), None);
    assert_eq!(clip.channel_count(), 3);
    let morph = clip.channel(2).unwrap();
    let AnimationTarget::MeshInstanceMorphWeights(id) = morph.target() else {
        panic!("expected morph weights")
    };
    assert_eq!(id, scene.mesh_instances().next().unwrap().id());
    assert_eq!(
        scene.mesh_instance(id).unwrap().node().id(),
        scene.nodes().next().unwrap().id()
    );
    assert_eq!(morph.interpolation(), AnimationInterpolation::CubicSpline);
    assert_eq!(morph.value_width(), 2);
    assert_eq!(morph.key_count(), 2);
    assert_eq!(morph.key_times(), &[0.0, 1.0]);
    assert_eq!(morph.values(), &[0.0, 0.0, 1.0, 0.5]);
    assert_eq!(morph.in_tangents(), Some(&[0.0; 4][..]));
    assert_eq!(morph.out_tangents(), Some(&[0.0; 4][..]));
    assert_eq!(
        morph.pre_extrapolation(),
        AnimationExtrapolation::SourceDefault
    );
    assert_eq!(
        morph.post_extrapolation(),
        AnimationExtrapolation::SourceDefault
    );
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
