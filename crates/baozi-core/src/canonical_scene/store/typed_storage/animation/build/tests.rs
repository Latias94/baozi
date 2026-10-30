use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationTargetCatalog, AuthoringRange, CanonicalAuthoringControl, DeformationAuthoringBatch,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LogicalMeshLocalId,
    LogicalMorphAuthoring, MeshInstanceDeformationAuthoring, MorphTargetDefinitionAuthoring,
    MorphTargetIdentity, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring,
    validate_animation_batch_with_control, validate_deformation_batch_with_control,
    validate_geometry_batch_with_control, validate_node_batch_with_control,
};
use crate::entity::AuthoredTransform;
use crate::{SceneValidationLimits, SceneValidationWork};

use super::*;

struct AuthoringControl;

impl CanonicalAuthoringControl for AuthoringControl {
    type Error = Infallible;
    type Reservation = u64;

    fn poll(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        _work: SceneValidationWork,
        mut action: impl FnMut() -> R,
    ) -> Result<R, Self::Error> {
        Ok(action())
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        Ok(bytes)
    }

    fn validate_scratch(&mut self, _reservation: &Self::Reservation) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        *reservation = required_bytes;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(usize);

#[derive(Default)]
struct Control {
    completed: usize,
    string_completed: usize,
    reject_before: Option<usize>,
    panic_before: Option<usize>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, Interrupted> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.panic_before == Some(self.completed) {
            // Skip the global panic hook so the allocation probe observes only the
            // caught payload and partial builder buffers, not test-output capture.
            std::panic::resume_unwind(Box::new(Interrupted(self.completed)));
        }
        let result = action();
        self.completed += 1;
        self.string_completed += usize::from(string);
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(false, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(true, action)
    }
}

fn node(parent: Presence<NodeParent>) -> NodeAuthoringNode<'static> {
    NodeAuthoringNode::new(
        parent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )
}

fn with_fixture<T>(action: impl FnOnce(&ValidatedAnimationBatch<'_, u64>) -> T) -> T {
    let limits = SceneValidationLimits::default();
    let mut authoring = AuthoringControl;
    // Node targets already carry this canonical mapping in ValidatedAnimationBatch.
    let roots = [NodeLocalId::new(2), NodeLocalId::new(1)];
    let source_nodes = [
        node(Presence::Present(NodeParent::new(NodeLocalId::new(2), 0))),
        node(Presence::Absent),
        node(Presence::Absent),
    ];
    let nodes = validate_node_batch_with_control(
        NodeAuthoringBatch::new(Presence::Present(&roots), &source_nodes),
        limits,
        &mut authoring,
    )
    .unwrap();
    assert_eq!(nodes.canonical_index_by_local(), [3, 2, 1]);
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 0),
    )];
    let instances = [
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(2), LogicalMeshLocalId::new(0)),
    ];
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &[], &instances),
        source_nodes.len(),
        0,
        limits,
        &mut authoring,
    )
    .unwrap();
    let definitions = [2, 0, 1].map(|identity| {
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(identity), Presence::Absent)
    });
    let logical = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, 3),
        &[0.0; 3],
    )];
    let morph_targets = [PrimitiveMorphTargetAuthoring::new(&[]); 3];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(
        Presence::Absent,
        &morph_targets,
    )];
    let deformations =
        [MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent); 2];
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::new(
            &[],
            &definitions,
            &logical,
            &primitive_deformations,
            &deformations,
        ),
        &nodes,
        &geometry,
        limits,
        &mut authoring,
    )
    .unwrap();
    let catalog = AnimationTargetCatalog::new(&nodes, &deformation);

    let ordinary_times = [-2.5, f64::from_bits(1.0_f64.to_bits() + 1)];
    let rotation_times = [-0.0, 0.125];
    let morph_times = [0.0, 2.0];
    let frame_times = [16_777_217.0, 16_777_217.5];
    let tick_times = [0.0, 3.5];
    let subnormal_time = [f64::from_bits(1)];
    let translation = [f32::from_bits(1), -0.0, 2.0, 3.0, 4.0, 5.0];
    let rotation = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    let rotation_in = [-5.0, -0.0, f32::from_bits(1), 2.0, 1.0, 3.0, 0.0, 4.0];
    let rotation_out = [9.0, 8.0, 7.0, 6.0, -0.0, 5.0, 4.0, 3.0];
    let morph = [-0.0, 0.25, 1.5, 2.0, -3.0, 4.0];
    let scale = [1.0, 1.0, 1.0, 2.0, 3.0, 4.0];
    let morph_in = [-2.0, -0.0, 3.0, 4.0, 5.0, 6.0];
    let morph_out = [7.0, 8.0, 9.0, 10.0, 11.0, f32::from_bits(1)];
    let one_rotation = [0.0, 0.0, 0.0, 1.0];
    let channels = [
        AnimationChannelAuthoring::new(
            4,
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Constant,
            &ordinary_times,
            &translation,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            8,
            AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(2)),
            AnimationInterpolation::CubicSpline,
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Repeat,
            &rotation_times,
            &rotation,
            Presence::Present(&rotation_in),
            Presence::Present(&rotation_out),
        ),
        AnimationChannelAuthoring::new(
            15,
            AnimationTargetAuthoring::MeshInstanceMorphWeights(1),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::Repeat,
            AnimationExtrapolation::SourceDefault,
            &morph_times,
            &morph,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            42,
            AnimationTargetAuthoring::NodeScale(NodeLocalId::new(1)),
            AnimationInterpolation::Step,
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Linear,
            &frame_times,
            &scale,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            90,
            AnimationTargetAuthoring::MeshInstanceMorphWeights(0),
            AnimationInterpolation::CubicSpline,
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Constant,
            &tick_times,
            &morph,
            Presence::Present(&morph_in),
            Presence::Present(&morph_out),
        ),
        AnimationChannelAuthoring::new(
            125,
            AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(1)),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Repeat,
            &subnormal_time,
            &one_rotation,
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let unicode_name = String::from("步行");
    let tick_name = String::from("α");
    let clips = [
        AnimationClipAuthoring::new(
            3,
            f64::from_bits(2.0_f64.to_bits() + 1),
            AnimationTimeDomain::Seconds,
            AuthoringRange::new(0, 3),
        )
        .with_name(&unicode_name),
        AnimationClipAuthoring::new(
            9,
            16_777_218.0,
            AnimationTimeDomain::Frames {
                rate: f64::from_bits(24.0_f64.to_bits() + 1),
            },
            AuthoringRange::new(3, 1),
        )
        .with_name(""),
        AnimationClipAuthoring::new(
            40,
            3.5,
            AnimationTimeDomain::Ticks {
                rate: Some(f64::from_bits(60.0_f64.to_bits() + 1)),
            },
            AuthoringRange::new(4, 1),
        ),
        AnimationClipAuthoring::new(
            100,
            1.0,
            AnimationTimeDomain::Ticks { rate: None },
            AuthoringRange::new(5, 1),
        )
        .with_name(&tick_name),
        AnimationClipAuthoring::new(
            150,
            -0.0,
            AnimationTimeDomain::Seconds,
            AuthoringRange::new(6, 0),
        ),
    ];
    let batch = validate_animation_batch_with_control(
        AnimationAuthoringBatch::new(&clips, &channels),
        &catalog,
        limits,
        &mut authoring,
    )
    .unwrap();
    action(&batch)
}

#[test]
fn build_owns_bits_names_domains_canonical_targets_and_key_major_payloads() {
    let (mut storage, plan, control) = with_fixture(|batch| {
        let plan = AnimationStoragePlan::new(batch).unwrap();
        assert_eq!(plan.row_counts(), [5, 6]);
        assert_eq!(plan.payload_counts(), [11, 36, 14, 14, 8]);
        assert_eq!(plan.work().unwrap(), 94);
        let mut control = Control::default();
        let storage = plan.build(batch, &mut control).unwrap();
        (storage, plan, control)
    });
    // All source buffers, validated plans, node/geometry/deformation catalogs, and
    // dynamic name strings have left scope. These reads borrow only the new owner.
    assert_eq!(control.completed, 94);
    assert_eq!(control.string_completed, plan.name_bytes());
    assert_eq!(storage.names, "步行α".as_bytes());
    assert_eq!(
        storage.clips[0].name.unwrap().get(&storage.names),
        Some("步行")
    );
    assert_eq!(storage.clips[1].name.unwrap().get(&storage.names), Some(""));
    assert!(storage.clips[2].name.is_none());
    assert_eq!(
        storage.clips[3].name.unwrap().get(&storage.names),
        Some("α")
    );
    assert!(storage.clips[4].name.is_none());
    assert_eq!(
        storage
            .clips
            .iter()
            .map(|row| row.source_index)
            .collect::<Vec<_>>(),
        [3, 9, 40, 100, 150]
    );
    assert_eq!(
        storage
            .channels
            .iter()
            .map(|row| row.source_index)
            .collect::<Vec<_>>(),
        [4, 8, 15, 42, 90, 125]
    );
    assert_eq!(storage.clips[0].channels, TableRange::new(0, 3));
    assert_eq!(storage.clips[1].channels, TableRange::new(3, 1));
    assert_eq!(storage.clips[2].channels, TableRange::new(4, 1));
    assert_eq!(storage.clips[3].channels, TableRange::new(5, 1));
    assert_eq!(storage.clips[4].channels, TableRange::new(6, 0));
    assert_eq!(storage.clips[0].duration.to_bits(), 2.0_f64.to_bits() + 1);
    assert_eq!(storage.clips[4].duration.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(storage.clips[0].time_domain, AnimationTimeDomain::Seconds);
    let AnimationTimeDomain::Frames { rate } = storage.clips[1].time_domain else {
        panic!("frames must remain frames")
    };
    assert_eq!(rate.to_bits(), 24.0_f64.to_bits() + 1);
    let AnimationTimeDomain::Ticks { rate: Some(rate) } = storage.clips[2].time_domain else {
        panic!("known tick rate must remain declared")
    };
    assert_eq!(rate.to_bits(), 60.0_f64.to_bits() + 1);
    assert_eq!(
        storage.clips[3].time_domain,
        AnimationTimeDomain::Ticks { rate: None }
    );
    let targets = [
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(3)),
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(1)),
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(1)),
        StoredAnimationTarget::NodeScale(StoredNodeIndex(2)),
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(2)),
    ];
    let interpolations = [
        AnimationInterpolation::Linear,
        AnimationInterpolation::CubicSpline,
        AnimationInterpolation::Linear,
        AnimationInterpolation::Step,
        AnimationInterpolation::CubicSpline,
        AnimationInterpolation::Linear,
    ];
    let extrapolations = [
        (
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Constant,
        ),
        (
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Repeat,
        ),
        (
            AnimationExtrapolation::Repeat,
            AnimationExtrapolation::SourceDefault,
        ),
        (
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Linear,
        ),
        (
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Constant,
        ),
        (
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Repeat,
        ),
    ];
    for (index, row) in storage.channels.iter().enumerate() {
        assert_eq!(row.target, targets[index]);
        assert_eq!(row.interpolation, interpolations[index]);
        assert_eq!(
            (row.pre_extrapolation, row.post_extrapolation),
            extrapolations[index]
        );
        assert_eq!(row.value_width(), Some([3, 4, 3, 3, 3, 4][index]));
        assert_eq!(row.in_tangents.is_some(), matches!(index, 1 | 4));
        assert_eq!(row.out_tangents.is_some(), matches!(index, 1 | 4));
    }
    assert_eq!(storage.times[0].to_bits(), (-2.5_f64).to_bits());
    assert_eq!(storage.times[1].to_bits(), 1.0_f64.to_bits() + 1);
    assert_eq!(
        storage.channels[3].times.get(&storage.times).unwrap(),
        [16_777_217.0, 16_777_217.5]
    );
    assert_eq!(storage.times[10].to_bits(), 1);
    assert_eq!(storage.values[0].to_bits(), 1);
    assert_eq!(storage.values[1].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(
        storage.channels[2].values.get(&storage.values).unwrap(),
        [-0.0, 0.25, 1.5, 2.0, -3.0, 4.0]
    );
    let rotation_in = storage.channels[1]
        .in_tangents
        .unwrap()
        .get(&storage.in_tangents)
        .unwrap();
    assert_eq!(
        rotation_in,
        [-5.0, -0.0, f32::from_bits(1), 2.0, 1.0, 3.0, 0.0, 4.0]
    );
    assert_eq!(rotation_in[1].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(storage.channels[4].in_tangents, Some(TableRange::new(8, 6)));
    assert_eq!(
        storage.channels[4].out_tangents,
        Some(TableRange::new(8, 6))
    );
    assert_eq!(storage.out_tangents[13].to_bits(), 1);
    assert_eq!(
        storage
            .times
            .iter()
            .copied()
            .map(f64::to_bits)
            .collect::<Vec<_>>(),
        [
            -2.5,
            f64::from_bits(1.0_f64.to_bits() + 1),
            -0.0,
            0.125,
            0.0,
            2.0,
            16_777_217.0,
            16_777_217.5,
            0.0,
            3.5,
            f64::from_bits(1),
        ]
        .map(f64::to_bits),
    );
    assert_eq!(
        storage
            .values
            .iter()
            .copied()
            .map(f32::to_bits)
            .collect::<Vec<_>>(),
        [
            f32::from_bits(1),
            -0.0,
            2.0,
            3.0,
            4.0,
            5.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            1.0,
            0.0,
            0.0,
            -0.0,
            0.25,
            1.5,
            2.0,
            -3.0,
            4.0,
            1.0,
            1.0,
            1.0,
            2.0,
            3.0,
            4.0,
            -0.0,
            0.25,
            1.5,
            2.0,
            -3.0,
            4.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ]
        .map(f32::to_bits),
    );
    assert_eq!(
        storage
            .in_tangents
            .iter()
            .copied()
            .map(f32::to_bits)
            .collect::<Vec<_>>(),
        [
            -5.0,
            -0.0,
            f32::from_bits(1),
            2.0,
            1.0,
            3.0,
            0.0,
            4.0,
            -2.0,
            -0.0,
            3.0,
            4.0,
            5.0,
            6.0,
        ]
        .map(f32::to_bits),
    );
    assert_eq!(
        storage
            .out_tangents
            .iter()
            .copied()
            .map(f32::to_bits)
            .collect::<Vec<_>>(),
        [
            9.0,
            8.0,
            7.0,
            6.0,
            -0.0,
            5.0,
            4.0,
            3.0,
            7.0,
            8.0,
            9.0,
            10.0,
            11.0,
            f32::from_bits(1),
        ]
        .map(f32::to_bits),
    );
    let expected = (std::mem::size_of::<AnimationStorage>()
        + 5 * std::mem::size_of::<StoredAnimationClip>()
        + 6 * std::mem::size_of::<StoredAnimationChannel>()
        + 11 * std::mem::size_of::<f64>()
        + (36 + 14 + 14) * std::mem::size_of::<f32>()
        + 8) as u64;
    assert_eq!(plan.canonical_bytes().unwrap(), expected);
    storage.clips.reserve(20);
    storage.values.reserve(100);
    assert_eq!(
        AnimationStoragePlan::canonical_bytes_for_shape(
            [storage.clips.len() as u64, storage.channels.len() as u64],
            [
                storage.times.len() as u64,
                storage.values.len() as u64,
                storage.in_tangents.len() as u64,
                storage.out_tangents.len() as u64,
                storage.names.len() as u64
            ],
        )
        .unwrap(),
        expected
    );
}

#[test]
fn each_materialization_action_releases_partial_buffers_on_error_and_unwind() {
    with_fixture(|batch| {
        let plan = AnimationStoragePlan::new(batch).unwrap();
        for before in 0..usize::try_from(plan.work().unwrap()).unwrap() {
            let mut control = Control {
                reject_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                assert!(matches!(
                    plan.build(batch, &mut control),
                    Err(TypedStorageBuildError::Control(Interrupted(completed))) if completed == before,
                ));
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);

            let mut control = Control {
                panic_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                let unwind = catch_unwind(AssertUnwindSafe(|| plan.build(batch, &mut control)));
                assert!(unwind.is_err());
                drop(unwind);
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
        }
    });
}

#[test]
fn empty_owner_has_seven_empty_capacities_and_zero_materialization_actions() {
    let limits = SceneValidationLimits::default();
    let mut authoring = AuthoringControl;
    let nodes = validate_node_batch_with_control(
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        limits,
        &mut authoring,
    )
    .unwrap();
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        0,
        0,
        limits,
        &mut authoring,
    )
    .unwrap();
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &nodes,
        &geometry,
        limits,
        &mut authoring,
    )
    .unwrap();
    let catalog = AnimationTargetCatalog::new(&nodes, &deformation);
    let batch = validate_animation_batch_with_control(
        AnimationAuthoringBatch::empty(),
        &catalog,
        limits,
        &mut authoring,
    )
    .unwrap();
    let plan = AnimationStoragePlan::new(&batch).unwrap();
    assert_eq!(
        plan.capacities()
            .unwrap()
            .map(|capacity| capacity.desired_capacity()),
        [0; 7]
    );
    assert_eq!(plan.row_counts(), [0; 2]);
    assert_eq!(plan.payload_counts(), [0; 5]);
    assert_eq!(plan.name_bytes(), 0);
    assert_eq!(plan.work().unwrap(), 0);
    assert_eq!(
        plan.canonical_bytes().unwrap(),
        std::mem::size_of::<AnimationStorage>() as u64
    );
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        let storage = plan.build(&batch, &mut control).unwrap();
        assert!(storage.clips.is_empty() && storage.channels.is_empty());
        assert!(storage.times.is_empty() && storage.values.is_empty());
        assert!(
            storage.in_tangents.is_empty()
                && storage.out_tangents.is_empty()
                && storage.names.is_empty()
        );
    });
    assert_eq!(control.completed, 0);
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn checked_shape_is_lean_and_mismatched_plan_fails_before_allocation() {
    assert!(std::mem::size_of::<AnimationStoragePlan>() < std::mem::size_of::<[CapacityPlan; 7]>());
    assert!(matches!(
        AnimationStoragePlan::for_shape([u64::from(u32::MAX) + 1, 0], [0; 5]),
        Err(StorePlanError::SizeOverflow)
    ));
    assert!(AnimationStoragePlan::for_shape([0; 2], [u64::MAX; 5]).is_err());
    assert!(matches!(
        AnimationStoragePlan::canonical_bytes_for_shape([u64::MAX; 2], [u64::MAX; 5]),
        Err(StorePlanError::SizeOverflow)
    ));
    with_fixture(|batch| {
        let correct = AnimationStoragePlan::new(batch).unwrap();
        let incorrect = AnimationStoragePlan::for_shape([5, 5], correct.payload_counts()).unwrap();
        let mut control = Control::default();
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                incorrect.build(batch, &mut control),
                Err(TypedStorageBuildError::Invariant)
            ));
        });
        assert_eq!(control.completed, 0);
        assert_eq!(allocations.count_total, 0);
    });
}

#[test]
fn derived_width_rejects_empty_misaligned_or_wrong_trs_shape_without_panicking() {
    let mut row = StoredAnimationChannel {
        source_index: 0,
        target: StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
        interpolation: AnimationInterpolation::Linear,
        pre_extrapolation: AnimationExtrapolation::SourceDefault,
        post_extrapolation: AnimationExtrapolation::SourceDefault,
        times: TableRange::new(0, 0),
        values: TableRange::new(0, 0),
        in_tangents: None,
        out_tangents: None,
    };
    assert_eq!(row.value_width(), None);
    row.times = TableRange::new(0, 2);
    assert_eq!(row.value_width(), None);
    row.values = TableRange::new(0, 5);
    assert_eq!(row.value_width(), None);
    row.values = TableRange::new(0, 6);
    assert_eq!(row.value_width(), Some(3));
    row.target = StoredAnimationTarget::NodeRotation(StoredNodeIndex(1));
    assert_eq!(row.value_width(), None);
    row.values = TableRange::new(0, 8);
    assert_eq!(row.value_width(), Some(4));
    row.target = StoredAnimationTarget::NodeScale(StoredNodeIndex(1));
    assert_eq!(row.value_width(), None);
    row.values = TableRange::new(0, 6);
    assert_eq!(row.value_width(), Some(3));
}
