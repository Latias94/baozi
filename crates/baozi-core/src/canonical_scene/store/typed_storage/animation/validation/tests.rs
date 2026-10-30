use super::*;
use crate::canonical_scene::store::production::input::deformation::MorphTargetIdentity;
use crate::canonical_scene::store::typed_storage::deformation::{
    StoredLogicalMorph, StoredMeshInstanceDeformation, StoredMorphDefinition,
};
use crate::canonical_scene::store::typed_storage::{
    StoredLogicalMesh, StoredLogicalMeshIndex, StoredMeshInstance, StoredNode, StoredNodeIndex,
};
use crate::math::{Mat4, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(u64);

#[derive(Default)]
struct Control {
    completed: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
}

impl TypedStorageValidationControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.omit_before == Some(self.completed) {
            return Ok(None);
        }
        let result = action();
        self.completed += 1;
        Ok(Some(result))
    }
}

struct Fixture {
    storage: AnimationStorage,
    nodes: NodeStorage,
    geometry: GeometryStorage,
    deformation: DeformationStorage,
    counts: AnimationMeteredCounts,
}

impl Fixture {
    fn validate(&self, control: &mut Control) -> Result<bool, Interrupted> {
        let mut marks = [u32::MAX; 11];
        let mut scratch = AnimationValidationScratch::new(&mut marks);
        if !scratch.initialize(control)? {
            return Ok(false);
        }
        self.storage.validate(
            control,
            self.counts,
            &self.nodes,
            &self.geometry,
            &self.deformation,
            &mut scratch,
        )
    }
}

fn fixture() -> Fixture {
    let node = |transform, children| StoredNode {
        children,
        transform,
        name: None,
        camera: None,
        light: None,
    };
    let trs = StoredTransform::Trs {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::new(1.0, 1.0, 1.0),
    };
    let nodes = NodeStorage {
        transform_scalar_count: 36,
        nodes: vec![
            node(StoredTransform::Synthetic, TableRange::new(0, 3)),
            node(trs, TableRange::new(3, 0)),
            node(
                StoredTransform::Matrix(Mat4::IDENTITY),
                TableRange::new(3, 0),
            ),
            node(trs, TableRange::new(3, 0)),
        ],
        children: vec![StoredNodeIndex(1), StoredNodeIndex(2), StoredNodeIndex(3)],
        names: Vec::new(),
    };
    let geometry = GeometryStorage {
        logical_meshes: vec![
            StoredLogicalMesh {
                primitives: TableRange::new(0, 0),
                name: None,
            },
            StoredLogicalMesh {
                primitives: TableRange::new(0, 0),
                name: None,
            },
        ],
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: vec![
            StoredMeshInstance {
                owner_node: StoredNodeIndex(1),
                logical_mesh: StoredLogicalMeshIndex(1),
            },
            StoredMeshInstance {
                owner_node: StoredNodeIndex(3),
                logical_mesh: StoredLogicalMeshIndex(0),
            },
        ],
        names: Vec::new(),
        f32_values: Vec::new(),
        f64_values: Vec::new(),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    };
    let instance = StoredMeshInstanceDeformation {
        skin: None,
        morph_weights: None,
    };
    let deformation = DeformationStorage {
        skins: Vec::new(),
        logical_morphs: vec![
            StoredLogicalMorph {
                definitions: TableRange::new(0, 2),
                default_weights: TableRange::new(0, 2),
            },
            StoredLogicalMorph {
                definitions: TableRange::new(2, 0),
                default_weights: TableRange::new(2, 0),
            },
        ],
        morph_definitions: vec![
            StoredMorphDefinition {
                logical_mesh: StoredLogicalMeshIndex(0),
                target_ordinal: 0,
                stable_identity: MorphTargetIdentity::new(1),
                name: None,
            },
            StoredMorphDefinition {
                logical_mesh: StoredLogicalMeshIndex(0),
                target_ordinal: 1,
                stable_identity: MorphTargetIdentity::new(0),
                name: None,
            },
        ],
        primitives: Vec::new(),
        morph_targets: Vec::new(),
        morph_deltas: Vec::new(),
        mesh_instances: vec![instance, instance],
        joints: Vec::new(),
        inverse_bind_matrices: Vec::new(),
        names: Vec::new(),
        influence_vertex_offsets: Vec::new(),
        influence_joints: Vec::new(),
        influence_weights: Vec::new(),
        influence_source_orders: Vec::new(),
        morph_weights: vec![0.0, 0.0],
        morph_values: Vec::new(),
        morph_sparse_indices: Vec::new(),
    };
    let mut storage = empty();
    storage.names = "λtail".as_bytes().to_vec();
    storage.clips = vec![
        StoredAnimationClip {
            source_index: 2,
            name: Some(TextRange(TableRange::new(0, 2))),
            duration: f64::from_bits(1.0_f64.to_bits() + 1),
            time_domain: AnimationTimeDomain::Seconds,
            channels: TableRange::new(0, 3),
        },
        StoredAnimationClip {
            source_index: 6,
            name: Some(TextRange(TableRange::new(2, 0))),
            duration: 2.5,
            time_domain: AnimationTimeDomain::Frames { rate: 24.5 },
            channels: TableRange::new(3, 2),
        },
        StoredAnimationClip {
            source_index: 9,
            name: None,
            duration: -0.0,
            time_domain: AnimationTimeDomain::Ticks { rate: None },
            channels: TableRange::new(5, 0),
        },
        StoredAnimationClip {
            source_index: 17,
            name: Some(TextRange(TableRange::new(2, 4))),
            duration: f64::MAX,
            time_domain: AnimationTimeDomain::Ticks {
                rate: Some(f64::from_bits(1)),
            },
            channels: TableRange::new(5, 1),
        },
    ];
    channel(
        &mut storage,
        3,
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
        AnimationInterpolation::Linear,
        &[1.0, f64::from_bits(1.0_f64.to_bits() + 1)],
        &[1.0, -0.0, 3.0, 4.0, 5.0, 6.0],
        None,
    );
    channel(
        &mut storage,
        8,
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(3)),
        AnimationInterpolation::CubicSpline,
        &[0.0, 2.0_f64.powi(-60)],
        &[0.0, -0.0, 0.0, 1.0, -0.0, 0.0, -0.0, -1.0],
        Some((
            &[2.0, -3.0, 4.0, 5.0, 0.0, 7.0, 0.0, 0.0],
            &[0.0, 0.0, -0.0, 0.0, 2.0, 0.0, 0.0, 0.0],
        )),
    );
    channel(
        &mut storage,
        10,
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(1)),
        AnimationInterpolation::Step,
        &[0.0, 2.0],
        &[-1.0, 2.0, -0.0, 0.5],
        None,
    );
    channel(
        &mut storage,
        13,
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
        AnimationInterpolation::CubicSpline,
        &[0.25],
        &[2.0, 3.0, 4.0],
        Some((&[1.0, 2.0, 3.0], &[-1.0, -2.0, -3.0])),
    );
    channel(
        &mut storage,
        16,
        StoredAnimationTarget::NodeScale(StoredNodeIndex(3)),
        AnimationInterpolation::Step,
        &[0.0, 1.0],
        &[-2.0, -0.0, 0.0, 1.0, 2.0, 3.0],
        None,
    );
    channel(
        &mut storage,
        23,
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(3)),
        AnimationInterpolation::Linear,
        &[f64::from_bits(1)],
        &[0.0, 0.0, 1.0, 0.0],
        None,
    );
    let counts = AnimationMeteredCounts::for_storage(&storage).unwrap();
    Fixture {
        storage,
        nodes,
        geometry,
        deformation,
        counts,
    }
}

#[allow(clippy::too_many_arguments)]
fn channel(
    storage: &mut AnimationStorage,
    source_index: u32,
    target: StoredAnimationTarget,
    interpolation: AnimationInterpolation,
    times: &[f64],
    values: &[f32],
    tangents: Option<(&[f32], &[f32])>,
) {
    let times = append(&mut storage.times, times);
    let values = append(&mut storage.values, values);
    let (in_tangents, out_tangents) = match tangents {
        Some((input, output)) => (
            Some(append(&mut storage.in_tangents, input)),
            Some(append(&mut storage.out_tangents, output)),
        ),
        None => (None, None),
    };
    storage.channels.push(StoredAnimationChannel {
        source_index,
        target,
        interpolation,
        pre_extrapolation: AnimationExtrapolation::Repeat,
        post_extrapolation: AnimationExtrapolation::SourceDefault,
        times,
        values,
        in_tangents,
        out_tangents,
    });
}

fn append<T: Copy>(pool: &mut Vec<T>, values: &[T]) -> TableRange<T> {
    let start = pool.len();
    pool.extend_from_slice(values);
    TableRange::new(start, values.len())
}

fn empty() -> AnimationStorage {
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

#[test]
fn actual_storage_is_borrowed_without_allocation_with_exact_row_scalar_and_name_work() {
    let fixture = fixture();
    let requirements = AnimationValidationScratchRequirements::try_new(
        fixture.counts,
        fixture.nodes.nodes.len(),
        fixture.geometry.mesh_instances.len(),
    )
    .unwrap();
    assert_eq!(requirements.target_marks, 11);
    assert_eq!(requirements.initialization_work_units(), Some(11));
    // 2 fixed + 4 clips + 6 channels + 10 times + 31 values + 22 tangents + 6 UTF-8 bytes.
    assert_eq!(fixture.counts.validation_work(), Some(81));
    let mut control = Control::default();
    let allocation = allocation_counter::measure(|| {
        assert_eq!(
            AnimationMeteredCounts::for_storage(&fixture.storage),
            Some(fixture.counts)
        );
        assert!(fixture.validate(&mut control).unwrap());
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.bytes_total, 0);
    assert_eq!(control.completed, 92);
    assert!(fixture.storage.times[0] < fixture.storage.times[1]);
    assert_eq!(
        fixture.storage.times[0] as f32,
        fixture.storage.times[1] as f32
    );
    assert_eq!(
        fixture.storage.clips[0].duration.to_bits(),
        1.0_f64.to_bits() + 1
    );
    assert_eq!(
        fixture.storage.clips[2].duration.to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(fixture.storage.values[1].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(fixture.storage.values[21], -2.0);
    assert_eq!(fixture.storage.in_tangents[0], 2.0);
    assert_eq!(
        fixture.storage.channels[0].pre_extrapolation,
        AnimationExtrapolation::Repeat
    );
    assert!(fixture.storage.clips[1].name.unwrap().0.is_empty());
    assert!(fixture.storage.clips[2].name.is_none());
}

type Corruption = fn(&mut Fixture);

fn reject_cases(cases: &[(&str, Corruption)]) {
    for &(label, corrupt) in cases {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert!(
            !fixture.validate(&mut Control::default()).unwrap(),
            "{label}"
        );
    }
}

#[test]
fn sealed_shape_compact_ranges_clip_coverage_and_source_orders_are_required() {
    reject_cases(&[
        ("sealed count", |f| f.counts.channel_count -= 1),
        ("clip source duplicate", |f| {
            f.storage.clips[1].source_index = 2
        }),
        ("clip source decreasing", |f| {
            f.storage.clips[3].source_index = 1
        }),
        ("channel source duplicate", |f| {
            f.storage.channels[1].source_index = 3
        }),
        ("source order is global across clips", |f| {
            f.storage.channels[3].source_index = 9
        }),
        ("channel ownership overlap", |f| {
            f.storage.clips[1].channels = TableRange::new(2, 2)
        }),
        ("channel range overflow", |f| {
            f.storage.clips[0].channels = TableRange::new(usize::MAX, 3)
        }),
        ("unowned last channel", |f| {
            f.storage.clips[3].channels = TableRange::new(5, 0)
        }),
        ("time range gap", |f| {
            f.storage.channels[1].times = TableRange::new(3, 2)
        }),
        ("value range overlap", |f| {
            f.storage.channels[1].values = TableRange::new(5, 8)
        }),
        ("unowned time scalar", |f| {
            f.storage.times.push(100.0);
            f.counts = AnimationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
        ("unowned value scalar", |f| {
            f.storage.values.push(0.0);
            f.counts = AnimationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
        ("name gap", |f| {
            f.storage.clips[3].name = Some(TextRange(TableRange::new(3, 3)))
        }),
        ("invalid UTF-8 boundary", |f| {
            f.storage.clips[0].name = Some(TextRange(TableRange::new(0, 1)))
        }),
        ("control character", |f| f.storage.names[3] = b'\n'),
        ("unowned name bytes", |f| {
            f.storage.names.push(b'x');
            f.counts = AnimationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
        ("oversized name", |f| {
            f.storage.names = vec![b'x'; MAX_ENTITY_NAME_BYTES + 1];
            f.storage.clips[0].name = Some(TextRange(TableRange::new(0, f.storage.names.len())));
            f.counts = AnimationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
    ]);
}

#[test]
fn time_values_durations_and_rates_preserve_f64_but_reject_invalid_numbers() {
    reject_cases(&[
        ("negative duration", |f| f.storage.clips[0].duration = -1.0),
        ("nonfinite duration", |f| {
            f.storage.clips[0].duration = f64::NAN
        }),
        ("positive-infinite duration", |f| {
            f.storage.clips[0].duration = f64::INFINITY
        }),
        ("negative-infinite duration", |f| {
            f.storage.clips[0].duration = f64::NEG_INFINITY
        }),
        ("zero frame rate", |f| {
            f.storage.clips[1].time_domain = AnimationTimeDomain::Frames { rate: 0.0 }
        }),
        ("negative frame rate", |f| {
            f.storage.clips[1].time_domain = AnimationTimeDomain::Frames { rate: -1.0 }
        }),
        ("nonfinite frame rate", |f| {
            f.storage.clips[1].time_domain = AnimationTimeDomain::Frames {
                rate: f64::INFINITY,
            }
        }),
        ("negative-zero tick rate", |f| {
            f.storage.clips[3].time_domain = AnimationTimeDomain::Ticks { rate: Some(-0.0) }
        }),
        ("nonfinite tick rate", |f| {
            f.storage.clips[3].time_domain = AnimationTimeDomain::Ticks {
                rate: Some(f64::NAN),
            }
        }),
        ("nonfinite key", |f| f.storage.times[0] = f64::INFINITY),
        ("negative-infinite key", |f| {
            f.storage.times[0] = f64::NEG_INFINITY
        }),
        ("NaN key", |f| f.storage.times[3] = f64::NAN),
        ("repeated key", |f| f.storage.times[1] = f.storage.times[0]),
        ("repeated negative key", |f| {
            f.storage.times[0] = -1.0;
            f.storage.times[1] = -1.0;
        }),
        ("descending negative key", |f| {
            f.storage.times[0] = -1.0;
            f.storage.times[1] = -2.0;
        }),
        ("descending key", |f| {
            f.storage.times[0] = 2.0;
            f.storage.times[1] = 1.0;
        }),
        ("signed zeros are equal keys", |f| {
            f.storage.times[0] = -0.0;
            f.storage.times[1] = 0.0;
        }),
    ]);
    let mut fixture = fixture();
    fixture.storage.times[0] = -0.0;
    assert!(fixture.validate(&mut Control::default()).unwrap());
    assert_eq!(fixture.storage.times[0].to_bits(), (-0.0_f64).to_bits());
}

#[test]
fn authored_key_times_outside_source_duration_preserve_exact_bits() {
    let mut fixture = fixture();
    // Clip 0 has mixed-sign translation, negative-only rotation, and mixed-sign morph keys.
    // Translation ends one f64 ULP beyond duration; morph keys extend farther on both sides.
    let expected: [f64; 10] = [
        -2.0,
        f64::from_bits(0.25_f64.to_bits() + 1),
        -4.0,
        -0.25,
        -1.0,
        1.0,
        0.25,
        0.0,
        1.0,
        f64::from_bits(1),
    ];
    fixture.storage.times.copy_from_slice(&expected);
    fixture.storage.clips[0].duration = 0.25;
    let mut control = Control::default();
    assert!(fixture.validate(&mut control).unwrap());
    assert_eq!(control.completed, 92);
    for (&actual, expected) in fixture.storage.times.iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    assert_eq!(
        fixture.storage.clips[0].duration.to_bits(),
        0.25_f64.to_bits()
    );

    // A clip containing only negative keys may retain an authored negative-zero duration.
    let negative: [f64; 6] = [-4.0, -2.0, -3.0, -1.0, -0.5, -f64::from_bits(1)];
    fixture.storage.times[..6].copy_from_slice(&negative);
    fixture.storage.clips[0].duration = -0.0;
    assert!(fixture.validate(&mut Control::default()).unwrap());
    for (&actual, expected) in fixture.storage.times[..6].iter().zip(negative) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    assert_eq!(
        fixture.storage.clips[0].duration.to_bits(),
        (-0.0_f64).to_bits()
    );

    fixture.storage.times[5] = f64::from_bits(1);
    assert!(fixture.validate(&mut Control::default()).unwrap());
    assert_eq!(fixture.storage.times[5].to_bits(), 1);
    assert_eq!(
        fixture.storage.clips[0].duration.to_bits(),
        (-0.0_f64).to_bits()
    );
}

#[test]
fn targets_resolve_canonical_nodes_and_actual_instance_morph_definitions() {
    reject_cases(&[
        ("synthetic target", |f| {
            f.storage.channels[0].target =
                StoredAnimationTarget::NodeTranslation(StoredNodeIndex(0))
        }),
        ("out-of-range node", |f| {
            f.storage.channels[0].target =
                StoredAnimationTarget::NodeTranslation(StoredNodeIndex(4))
        }),
        ("full-domain node", |f| {
            f.storage.channels[0].target =
                StoredAnimationTarget::NodeTranslation(StoredNodeIndex(u32::MAX))
        }),
        ("matrix-authored target", |f| {
            f.storage.channels[0].target =
                StoredAnimationTarget::NodeTranslation(StoredNodeIndex(2))
        }),
        ("duplicate target within clip", |f| {
            f.storage.channels[4].target =
                StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1))
        }),
        ("instance with zero morph width", |f| {
            f.storage.channels[2].target =
                StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0))
        }),
        ("out-of-range instance", |f| {
            f.storage.channels[2].target =
                StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(2))
        }),
        ("missing deformation instance", |f| {
            f.deformation.mesh_instances.pop();
        }),
        ("missing logical morph", |f| {
            f.deformation.logical_morphs.pop();
        }),
        ("instance refers to absent mesh", |f| {
            f.geometry.mesh_instances[1].logical_mesh = StoredLogicalMeshIndex(2)
        }),
        ("definition range outside storage", |f| {
            f.deformation.logical_morphs[0].definitions = TableRange::new(1, 2)
        }),
        ("actual morph width differs", |f| {
            f.deformation.logical_morphs[0].definitions = TableRange::new(0, 1)
        }),
        ("translation width differs", |f| {
            f.storage.channels[0].values = TableRange::new(0, 4)
        }),
        ("empty channel", |f| {
            f.storage.channels[0].times = TableRange::new(0, 0)
        }),
    ]);
    let mut fixture = fixture();
    // The same target is valid in distinct clips; distinct TRS semantics on one node are distinct.
    fixture.storage.channels[4].target = StoredAnimationTarget::NodeScale(StoredNodeIndex(1));
    assert!(fixture.validate(&mut Control::default()).unwrap());
}

#[test]
fn target_marks_follow_borrowed_node_rows_and_actual_instance_indices() {
    let mut fixture = fixture();
    let additional_node = fixture.nodes.nodes[3];
    fixture.nodes.nodes.push(additional_node);
    fixture.nodes.children.push(StoredNodeIndex(4));
    fixture.nodes.nodes[0].children = TableRange::new(0, 4);
    for node in &mut fixture.nodes.nodes[1..] {
        node.children = TableRange::new(4, 0);
    }
    fixture.nodes.transform_scalar_count += 10;
    fixture.storage.channels[0].target = StoredAnimationTarget::NodeTranslation(StoredNodeIndex(4));

    let requirements = AnimationValidationScratchRequirements::try_new(
        fixture.counts,
        fixture.nodes.nodes.len(),
        fixture.geometry.mesh_instances.len(),
    )
    .unwrap();
    // Four authored nodes have twelve TRS slots, followed by the two actual instances.
    assert_eq!(requirements.target_marks, 14);
    assert_eq!(requirements.initialization_work_units(), Some(14));
    assert!(!fixture.validate(&mut Control::default()).unwrap());

    let mut marks = [u32::MAX; 14];
    let mut scratch = AnimationValidationScratch::new(&mut marks);
    let mut control = Control::default();
    let allocation = allocation_counter::measure(|| {
        assert!(scratch.initialize(&mut control).unwrap());
        assert!(
            fixture
                .storage
                .validate(
                    &mut control,
                    fixture.counts,
                    &fixture.nodes,
                    &fixture.geometry,
                    &fixture.deformation,
                    &mut scratch,
                )
                .unwrap()
        );
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.bytes_total, 0);
    assert_eq!(control.completed, 95);
    assert_eq!(scratch.target_marks[9], 1);
    assert_eq!(scratch.target_marks[12], 0);
    assert_eq!(scratch.target_marks[13], 1);
}

#[test]
fn interpolation_controls_real_tangent_presence_shape_order_and_values() {
    reject_cases(&[
        ("missing cubic input", |f| {
            f.storage.channels[1].in_tangents = None
        }),
        ("missing cubic output", |f| {
            f.storage.channels[1].out_tangents = None
        }),
        ("empty cubic tangents", |f| {
            f.storage.channels[1].in_tangents = Some(TableRange::new(0, 0))
        }),
        ("short cubic tangents", |f| {
            f.storage.channels[1].out_tangents = Some(TableRange::new(0, 7))
        }),
        ("unexpected linear tangents", |f| {
            f.storage.channels[0].in_tangents = Some(TableRange::new(0, 0))
        }),
        ("step cannot carry tangents", |f| {
            f.storage.channels[1].interpolation = AnimationInterpolation::Step
        }),
        ("input tangent overlap", |f| {
            f.storage.channels[3].in_tangents = Some(TableRange::new(7, 3))
        }),
        ("nonfinite input derivative", |f| {
            f.storage.in_tangents[10] = f32::NAN
        }),
        ("nonfinite output derivative", |f| {
            f.storage.out_tangents[0] = f32::INFINITY
        }),
        ("nonfinite translation", |f| f.storage.values[1] = f32::NAN),
        ("nonfinite morph weight", |f| {
            f.storage.values[14] = f32::NEG_INFINITY
        }),
        ("nonfinite scale", |f| f.storage.values[26] = f32::INFINITY),
        ("unowned derivative scalar", |f| {
            f.storage.in_tangents.push(0.0);
            f.storage.out_tangents.push(0.0);
            f.counts = AnimationMeteredCounts::for_storage(&f.storage).unwrap();
        }),
    ]);
}

#[test]
fn each_rotation_key_is_unit_but_its_cubic_derivatives_are_not_normalized() {
    reject_cases(&[
        ("zero quaternion", |f| f.storage.values[9] = 0.0),
        ("second key must also be unit", |f| {
            f.storage.values[13] = -2.0
        }),
        ("last channel nonunit key", |f| f.storage.values[29] = 2.0),
        ("nonfinite quaternion component", |f| {
            f.storage.values[6] = f32::NAN
        }),
    ]);
    let mut fixture = fixture();
    fixture.storage.in_tangents[0] = f32::MAX;
    fixture.storage.out_tangents[0] = -f32::MAX;
    assert!(fixture.validate(&mut Control::default()).unwrap());
    assert_eq!(fixture.storage.in_tangents[0].to_bits(), f32::MAX.to_bits());
    assert_eq!(fixture.storage.values[13], -1.0);
}

#[test]
fn cancellation_and_omitted_actions_stop_at_exact_prefix_including_completion() {
    let fixture = fixture();
    for prefix in [0, 10, 11, 12, 15, 34, 90, 91] {
        let mut rejected = Control {
            reject_before: Some(prefix),
            ..Control::default()
        };
        assert_eq!(fixture.validate(&mut rejected), Err(Interrupted(prefix)));
        assert_eq!(rejected.completed, prefix);
        let mut omitted = Control {
            omit_before: Some(prefix),
            ..Control::default()
        };
        assert!(!fixture.validate(&mut omitted).unwrap());
        assert_eq!(omitted.completed, prefix);
    }
    let mut exact = Control {
        reject_before: Some(92),
        ..Control::default()
    };
    assert!(fixture.validate(&mut exact).unwrap());
    assert_eq!(exact.completed, 92);
}

#[test]
fn scratch_is_exact_and_only_allocated_for_nonempty_channels() {
    let fixture = fixture();
    let mut marks = [0; 12];
    for length in [10, 12] {
        let mut scratch = AnimationValidationScratch::new(&mut marks[..length]);
        let mut control = Control::default();
        assert!(
            !fixture
                .storage
                .validate(
                    &mut control,
                    fixture.counts,
                    &fixture.nodes,
                    &fixture.geometry,
                    &fixture.deformation,
                    &mut scratch
                )
                .unwrap()
        );
        assert_eq!(control.completed, 1);
    }
    let mut marks = [u32::MAX; 11];
    let mut scratch = AnimationValidationScratch::new(&mut marks);
    let mut control = Control::default();
    assert!(scratch.initialize(&mut control).unwrap());
    assert_eq!(control.completed, 11);
    assert!(scratch.target_marks.iter().all(|&mark| mark == 0));
    let empty_counts = AnimationMeteredCounts::for_storage(&empty()).unwrap();
    let requirements =
        AnimationValidationScratchRequirements::try_new(empty_counts, usize::MAX, usize::MAX)
            .unwrap();
    assert_eq!(requirements.target_marks, 0);
    assert_eq!(requirements.initialization_work_units(), Some(0));
    assert!(
        AnimationValidationScratchRequirements::try_new(fixture.counts, usize::MAX, 2).is_none()
    );
    assert!(AnimationValidationScratchRequirements::try_new(fixture.counts, 0, 2).is_none());
}

#[test]
fn empty_storage_and_channel_free_named_clips_use_no_marks() {
    let dependencies = fixture();
    let mut storage = empty();
    let mut control = Control::default();
    let mut scratch = AnimationValidationScratch::new(&mut []);
    let counts = AnimationMeteredCounts::for_storage(&storage).unwrap();
    let allocation = allocation_counter::measure(|| {
        assert!(
            storage
                .validate(
                    &mut control,
                    counts,
                    &dependencies.nodes,
                    &dependencies.geometry,
                    &dependencies.deformation,
                    &mut scratch
                )
                .unwrap()
        );
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(control.completed, 2);
    storage.names.push(b'a');
    storage.clips = vec![
        StoredAnimationClip {
            source_index: 2,
            name: Some(TextRange(TableRange::new(0, 1))),
            duration: 0.0,
            time_domain: AnimationTimeDomain::Seconds,
            channels: TableRange::new(0, 0),
        },
        StoredAnimationClip {
            source_index: 5,
            name: Some(TextRange(TableRange::new(1, 0))),
            duration: -0.0,
            time_domain: AnimationTimeDomain::Ticks { rate: None },
            channels: TableRange::new(0, 0),
        },
    ];
    let counts = AnimationMeteredCounts::for_storage(&storage).unwrap();
    assert_eq!(counts.validation_work(), Some(5));
    let mut control = Control::default();
    assert!(
        storage
            .validate(
                &mut control,
                counts,
                &dependencies.nodes,
                &dependencies.geometry,
                &dependencies.deformation,
                &mut scratch
            )
            .unwrap()
    );
    assert_eq!(control.completed, 5);
}

#[test]
fn count_shape_is_o1_and_rejects_unrepresentable_or_impossible_aggregate_shapes() {
    assert!(AnimationMeteredCounts::try_new([u64::from(u32::MAX) + 1, 0], [0; 5]).is_none());
    assert!(AnimationMeteredCounts::try_new([0, 1], [1, 3, 0, 0, 0]).is_none());
    assert!(AnimationMeteredCounts::try_new([1, 0], [1, 0, 0, 0, 0]).is_none());
    assert!(AnimationMeteredCounts::try_new([1, 1], [1, 3, 3, 0, 0]).is_none());
    let mut fixture = fixture();
    fixture.storage.channels[0].times = TableRange::new(usize::MAX, usize::MAX);
    // Only row/scalar/name lengths belong to this O(1) shape. Validation checks actual ranges.
    assert_eq!(
        AnimationMeteredCounts::for_storage(&fixture.storage),
        Some(fixture.counts)
    );
    assert!(!fixture.validate(&mut Control::default()).unwrap());
    #[cfg(target_pointer_width = "64")]
    {
        let count = u64::from(u32::MAX) + 1;
        let counts = AnimationMeteredCounts::try_new([1, 1], [count, count, 0, 0, count]).unwrap();
        assert_eq!(counts.time_scalar_count, count);
        assert_eq!(counts.value_scalar_count, count);
        assert_eq!(counts.clip_name_byte_count, count);
        let counts =
            AnimationMeteredCounts::try_new([1, 1], [u64::MAX, u64::MAX, 0, 0, 0]).unwrap();
        assert_eq!(counts.validation_work(), None);
    }
}
