use crate::canonical_scene::store::typed_storage::TextRange;
use crate::canonical_scene::store::typed_storage::animation::{
    StoredAnimationChannel, StoredAnimationClip, StoredMeshInstanceIndex,
};

use super::*;

fn empty_storage() -> AnimationStorage {
    AnimationStorage {
        clips: Vec::new(),
        channels: Vec::new(),
        times: Vec::new(),
        values: Vec::new(),
        in_tangents: Vec::new(),
        out_tangents: Vec::new(),
        names: Vec::new(),
    }
}

fn fixture() -> AnimationStorage {
    AnimationStorage {
        clips: vec![
            StoredAnimationClip {
                source_index: 7,
                name: None,
                duration: 1.0000000000000002,
                time_domain: AnimationTimeDomain::Seconds,
                channels: TableRange::new(0, 1),
            },
            StoredAnimationClip {
                source_index: 11,
                name: Some(TextRange(TableRange::new(0, 0))),
                duration: 2400.1250000000005,
                time_domain: AnimationTimeDomain::Frames {
                    rate: 29.97002997002997,
                },
                channels: TableRange::new(1, 2),
            },
            StoredAnimationClip {
                source_index: 19,
                name: Some(TextRange(TableRange::new(0, 3))),
                duration: -0.0,
                time_domain: AnimationTimeDomain::Ticks { rate: None },
                channels: TableRange::new(3, 0),
            },
            StoredAnimationClip {
                source_index: 25,
                name: Some(TextRange(TableRange::new(3, 0))),
                duration: 96000.125,
                time_domain: AnimationTimeDomain::Ticks {
                    rate: Some(48000.25),
                },
                channels: TableRange::new(3, 1),
            },
        ],
        channels: vec![
            StoredAnimationChannel {
                source_index: 3,
                target: StoredAnimationTarget::NodeTranslation(StoredNodeIndex(2)),
                interpolation: AnimationInterpolation::Linear,
                pre_extrapolation: AnimationExtrapolation::Constant,
                post_extrapolation: AnimationExtrapolation::Linear,
                times: TableRange::new(0, 2),
                values: TableRange::new(0, 6),
                in_tangents: None,
                out_tangents: None,
            },
            StoredAnimationChannel {
                source_index: 8,
                target: StoredAnimationTarget::NodeScale(StoredNodeIndex(1)),
                interpolation: AnimationInterpolation::CubicSpline,
                pre_extrapolation: AnimationExtrapolation::Repeat,
                post_extrapolation: AnimationExtrapolation::Constant,
                times: TableRange::new(2, 2),
                values: TableRange::new(6, 6),
                in_tangents: Some(TableRange::new(0, 6)),
                out_tangents: Some(TableRange::new(0, 6)),
            },
            StoredAnimationChannel {
                source_index: 12,
                target: StoredAnimationTarget::NodeRotation(StoredNodeIndex(2)),
                interpolation: AnimationInterpolation::Step,
                pre_extrapolation: AnimationExtrapolation::SourceDefault,
                post_extrapolation: AnimationExtrapolation::Repeat,
                times: TableRange::new(4, 3),
                values: TableRange::new(12, 12),
                in_tangents: None,
                out_tangents: None,
            },
            StoredAnimationChannel {
                source_index: 45,
                target: StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
                interpolation: AnimationInterpolation::Linear,
                pre_extrapolation: AnimationExtrapolation::Linear,
                post_extrapolation: AnimationExtrapolation::SourceDefault,
                times: TableRange::new(7, 2),
                values: TableRange::new(24, 4),
                in_tangents: None,
                out_tangents: None,
            },
        ],
        times: vec![
            -0.0,
            1.0000000000000002,
            12.25,
            24.125,
            0.0,
            30.0,
            120.0,
            48000.125,
            96000.125,
        ],
        values: vec![
            f32::from_bits(1),
            -0.0,
            2.0,
            3.0,
            4.0,
            5.0,
            1.0,
            2.0,
            3.0,
            4.0,
            5.0,
            6.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.25,
            -0.0,
            0.5,
            0.75,
        ],
        in_tangents: vec![-0.0, -2.0, -3.0, -4.0, -5.0, -6.0],
        out_tangents: vec![0.25, 0.5, 0.75, 1.0, 1.25, 1.5],
        names: "歩".as_bytes().to_vec(),
    }
}

// Projection tests deliberately bypass central proof minting. They exercise the reader's
// mechanical boundaries and coverage, not the validator's numeric/target correspondence proof.
fn tables(storage: &AnimationStorage) -> NodeSceneAnimationTables<'_> {
    NodeSceneAnimationTables {
        storage,
        source_node_count: 2,
        mesh_instance_count: 1,
    }
}

fn exhaust(storage: &AnimationStorage) -> NodeSceneSemanticResult<()> {
    let mut cursor = tables(storage).into_cursor();
    while cursor.next_clip()?.is_some() {}
    while cursor.next_channel()?.is_some() {}
    cursor.finish()
}

#[test]
fn borrowed_views_preserve_source_order_precision_enums_and_optional_names_without_allocating() {
    use NodeSceneAnimationTarget as A;
    let storage = fixture();
    take_row_read_count_for_test();
    let allocations = allocation_counter::measure(|| {
        let view = tables(&storage);
        for (row, source) in [7, 11, 19, 25].into_iter().enumerate() {
            assert_eq!(view.clip_at(row).unwrap().source_index(), source);
        }
        assert_eq!(view.clip_at(0).unwrap().name(), None);
        assert_eq!(view.clip_at(1).unwrap().name(), Some(""));
        let named = view.clip_at(2).unwrap();
        assert_eq!(named.name(), Some("歩"));
        assert!(std::ptr::eq(
            named.name().unwrap().as_bytes(),
            storage.names.as_slice()
        ));
        assert_eq!(named.channel_count(), 0);
        assert_eq!(named.channel_offset(), 3);
        assert_eq!(named.duration().to_bits(), (-0.0_f64).to_bits());
        assert_eq!(view.clip_at(3).unwrap().name(), Some(""));
        assert_eq!(
            view.clip_at(0).unwrap().duration().to_bits(),
            1.0000000000000002_f64.to_bits()
        );
        assert_eq!(
            view.clip_at(1).unwrap().duration().to_bits(),
            2400.1250000000005_f64.to_bits()
        );
        assert_eq!(
            view.clip_at(0).unwrap().time_domain(),
            AnimationTimeDomain::Seconds
        );
        let AnimationTimeDomain::Frames { rate } = view.clip_at(1).unwrap().time_domain() else {
            panic!("frame domain changed")
        };
        assert_eq!(rate.to_bits(), 29.97002997002997_f64.to_bits());
        assert_eq!(
            named.time_domain(),
            AnimationTimeDomain::Ticks { rate: None }
        );
        let AnimationTimeDomain::Ticks { rate: Some(rate) } =
            view.clip_at(3).unwrap().time_domain()
        else {
            panic!("tick rate presence changed")
        };
        assert_eq!(rate.to_bits(), 48000.25_f64.to_bits());

        for (row, (source, target, width, interpolation, pre, post)) in [
            (
                3,
                A::NodeTranslation(2),
                3,
                AnimationInterpolation::Linear,
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Linear,
            ),
            (
                8,
                A::NodeScale(1),
                3,
                AnimationInterpolation::CubicSpline,
                AnimationExtrapolation::Repeat,
                AnimationExtrapolation::Constant,
            ),
            (
                12,
                A::NodeRotation(2),
                4,
                AnimationInterpolation::Step,
                AnimationExtrapolation::SourceDefault,
                AnimationExtrapolation::Repeat,
            ),
            (
                45,
                A::MeshInstanceMorphWeights(0),
                2,
                AnimationInterpolation::Linear,
                AnimationExtrapolation::Linear,
                AnimationExtrapolation::SourceDefault,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let channel = view.channel_at(row).unwrap();
            assert_eq!(channel.source_index(), source);
            assert_eq!(channel.target(), target);
            assert_eq!(channel.value_width(), width);
            assert_eq!(channel.interpolation(), interpolation);
            assert_eq!(channel.pre_extrapolation(), pre);
            assert_eq!(channel.post_extrapolation(), post);
            let stored = storage.channels[row];
            assert!(std::ptr::eq(
                channel.times(),
                stored.times.get(&storage.times).unwrap()
            ));
            assert!(std::ptr::eq(
                channel.values(),
                stored.values.get(&storage.values).unwrap()
            ));
            assert_eq!(channel.key_count(), channel.times().len());
        }
        let translation = view.channel_at(0).unwrap();
        assert_eq!(translation.times()[0].to_bits(), (-0.0_f64).to_bits());
        assert_eq!(
            translation.times()[1].to_bits(),
            1.0000000000000002_f64.to_bits()
        );
        assert_eq!(translation.values()[0].to_bits(), 1);
        assert_eq!(translation.values()[1].to_bits(), (-0.0_f32).to_bits());
        assert!(translation.in_tangents().is_none());
        assert!(translation.out_tangents().is_none());
        let scale = view.channel_at(1).unwrap();
        assert!(std::ptr::eq(
            scale.in_tangents().unwrap(),
            storage.in_tangents.as_slice()
        ));
        assert!(std::ptr::eq(
            scale.out_tangents().unwrap(),
            storage.out_tangents.as_slice()
        ));
        assert_eq!(
            scale.in_tangents().unwrap()[0].to_bits(),
            (-0.0_f32).to_bits()
        );
        let mut independent = translation.times().iter();
        assert_eq!(independent.next_back().copied(), Some(1.0000000000000002));
        assert_eq!(
            translation
                .times()
                .iter()
                .next()
                .copied()
                .unwrap()
                .to_bits(),
            (-0.0_f64).to_bits()
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn random_reads_visit_one_row_and_do_not_validate_a_broken_prefix() {
    let mut storage = fixture();
    storage.clips[0].channels = TableRange::new(usize::MAX, 1);
    storage.channels[0].times = TableRange::new(usize::MAX, 2);
    take_row_read_count_for_test();
    assert_eq!(tables(&storage).clip_at(3).unwrap().source_index(), 25);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(tables(&storage).channel_at(3).unwrap().source_index(), 45);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert!(tables(&storage).clip_at(4).is_err());
    assert!(tables(&storage).channel_at(usize::MAX).is_err());
    assert_eq!(take_row_read_count_for_test(), 0);
    assert!(tables(&storage).clip_at(0).is_err());
    assert!(tables(&storage).channel_at(0).is_err());
    assert_eq!(take_row_read_count_for_test(), 2);
}

#[test]
fn selected_rows_reject_unreadable_names_invalid_headers_and_unresolved_targets_or_shapes() {
    type RowCorruption = (usize, fn(&mut AnimationStorage));
    let clip_corruptions: [RowCorruption; 3] = [
        (2, |s| s.names[0] = 0xff),
        (0, |s| s.clips[0].duration = f64::INFINITY),
        (1, |s| {
            s.clips[1].time_domain = AnimationTimeDomain::Frames { rate: 0.0 }
        }),
    ];
    for (row, corrupt) in clip_corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(tables(&storage).clip_at(row).is_err());
    }
    let channel_corruptions: [RowCorruption; 7] = [
        (0, |s| s.channels[0].values = TableRange::new(usize::MAX, 6)),
        (0, |s| {
            s.channels[0].target = StoredAnimationTarget::NodeTranslation(StoredNodeIndex(0))
        }),
        (0, |s| {
            s.channels[0].target = StoredAnimationTarget::NodeTranslation(StoredNodeIndex(3))
        }),
        (3, |s| {
            s.channels[3].target =
                StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(1))
        }),
        (3, |s| s.channels[3].values = TableRange::new(24, 3)),
        (1, |s| s.channels[1].out_tangents = None),
        (2, |s| {
            s.channels[2].in_tangents = Some(TableRange::new(0, 0))
        }),
    ];
    for (row, corrupt) in channel_corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(tables(&storage).channel_at(row).is_err());
    }
}

#[test]
fn exhaustive_traversal_yields_all_rows_and_exact_pool_ranges_without_allocating() {
    let storage = fixture();
    take_row_read_count_for_test();
    let allocations = allocation_counter::measure(|| {
        let mut cursor = tables(&storage).into_cursor();
        assert_eq!(cursor.clip_count(), 4);
        assert_eq!(cursor.channel_count(), 4);
        for source in [7, 11, 19, 25] {
            assert_eq!(cursor.next_clip().unwrap().unwrap().source_index(), source);
        }
        assert!(cursor.next_clip().unwrap().is_none());
        for source in [3, 8, 12, 45] {
            assert_eq!(
                cursor.next_channel().unwrap().unwrap().source_index(),
                source
            );
        }
        assert!(cursor.next_channel().unwrap().is_none());
        cursor.finish().unwrap();
    });
    assert_eq!(take_row_read_count_for_test(), 8);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn collection_phases_require_exhaustion_and_cannot_move_backwards() {
    let storage = fixture();
    assert!(tables(&storage).into_cursor().finish().is_err());
    let mut cursor = tables(&storage).into_cursor();
    assert!(cursor.next_channel().is_err());
    while cursor.next_clip().unwrap().is_some() {}
    assert!(cursor.finish().is_err());
    let mut cursor = tables(&storage).into_cursor();
    while cursor.next_clip().unwrap().is_some() {}
    assert!(cursor.next_channel().unwrap().is_some());
    assert!(cursor.next_clip().is_err());
    assert!(cursor.finish().is_err());
}

#[test]
fn exhaustive_reads_reject_source_reordering_overlap_gaps_and_unowned_payloads() {
    let corruptions: [fn(&mut AnimationStorage); 10] = [
        |s| s.clips[1].source_index = s.clips[0].source_index,
        |s| s.channels[3].source_index = s.channels[2].source_index,
        |s| s.clips[1].channels = TableRange::new(0, 2),
        |s| s.clips[1].name = Some(TextRange(TableRange::new(3, 0))),
        |s| s.channels[1].times = TableRange::new(0, 2),
        |s| s.times.push(0.0),
        |s| s.values.push(0.0),
        |s| s.in_tangents.push(0.0),
        |s| s.out_tangents.push(0.0),
        |s| s.names.push(b'x'),
    ];
    for corrupt in corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(exhaust(&storage).is_err());
    }
}

#[test]
fn empty_storage_and_named_channel_free_clips_complete_without_inventing_payloads() {
    let empty = empty_storage();
    exhaust(&empty).unwrap();
    let mut cursor = tables(&empty).into_cursor();
    assert!(cursor.next_channel().unwrap().is_none());
    assert!(cursor.next_clip().is_err());
    cursor.finish().unwrap();
    let mut storage = empty_storage();
    storage.clips.push(StoredAnimationClip {
        source_index: 500,
        name: Some(TextRange(TableRange::new(0, 0))),
        duration: -0.0,
        time_domain: AnimationTimeDomain::Ticks { rate: None },
        channels: TableRange::new(0, 0),
    });
    assert_eq!(tables(&storage).clip_at(0).unwrap().name(), Some(""));
    exhaust(&storage).unwrap();
    storage.names.extend_from_slice("空".as_bytes());
    storage.clips[0].name = Some(TextRange(TableRange::new(0, storage.names.len())));
    assert_eq!(tables(&storage).clip_at(0).unwrap().name(), Some("空"));
    exhaust(&storage).unwrap();
}
