use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;
use crate::animation::AnimationExtrapolation;
use crate::canonical_scene::store::typed_storage::animation::StoredMeshInstanceIndex;
use crate::canonical_scene::store::typed_storage::{StoredNodeIndex, TextRange};
use crate::{Axis, FrontFaceWinding, Handedness, SceneSpace, TargetSpace};

#[derive(Default)]
struct Control {
    completed: u64,
    attempted: u64,
    string_completed: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
    panic_before: Option<u64>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, u64> {
        self.attempted += 1;
        if self.reject_before == Some(self.completed) {
            return Err(self.completed);
        }
        if self.panic_before == Some(self.completed) {
            // Bypass the global panic hook so allocation probes count the caught
            // payload and partial buffers, not test-output capture allocations.
            std::panic::resume_unwind(Box::new(self.completed));
        }
        let result = action();
        self.completed += 1;
        self.string_completed += u64::from(string);
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(false, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(true, action)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.omit_before == Some(self.completed) {
            self.attempted += 1;
            return Ok(None);
        }
        self.execute(false, action).map(Some)
    }
}

fn target_conversion(
    handedness: Handedness,
    up: Axis,
    front: Axis,
    units: f64,
    winding: FrontFaceWinding,
) -> SpaceConversion {
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(handedness, up, front, units, winding).unwrap(),
        None,
    )
    .unwrap()
}

fn mixed_conversion() -> SpaceConversion {
    // From standard source to this basis: Q(x,y,z)=(-z,x,y), det(Q)=-1, s=2.
    // Rotation xyzw uses (z,-x,-y,w); scale uses (z,x,y).
    target_conversion(
        Handedness::Left,
        Axis::PositiveZ,
        Axis::PositiveX,
        0.5,
        FrontFaceWinding::CounterClockwise,
    )
}

fn units_conversion(units: f64) -> SpaceConversion {
    target_conversion(
        Handedness::Right,
        Axis::PositiveY,
        Axis::NegativeZ,
        units,
        FrontFaceWinding::CounterClockwise,
    )
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

fn one_channel(
    target: StoredAnimationTarget,
    interpolation: AnimationInterpolation,
    times: &[f64],
    values: &[f32],
    input: Option<&[f32]>,
    output: Option<&[f32]>,
) -> AnimationStorage {
    AnimationStorage {
        clips: vec![StoredAnimationClip {
            source_index: 17,
            name: None,
            duration: -0.0,
            time_domain: AnimationTimeDomain::Ticks { rate: None },
            channels: TableRange::new(0, 1),
        }],
        channels: vec![StoredAnimationChannel {
            source_index: 29,
            target,
            interpolation,
            pre_extrapolation: AnimationExtrapolation::SourceDefault,
            post_extrapolation: AnimationExtrapolation::Repeat,
            times: TableRange::new(0, times.len()),
            values: TableRange::new(0, values.len()),
            in_tangents: input.map(|values| TableRange::new(0, values.len())),
            out_tangents: output.map(|values| TableRange::new(0, values.len())),
        }],
        times: times.to_vec(),
        values: values.to_vec(),
        in_tangents: input.unwrap_or(&[]).to_vec(),
        out_tangents: output.unwrap_or(&[]).to_vec(),
        names: Vec::new(),
    }
}

fn mixed() -> AnimationStorage {
    let targets = [
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(2)),
        StoredAnimationTarget::NodeScale(StoredNodeIndex(1)),
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(1)),
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(3)),
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(3)),
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
    ];
    let time_ranges = [(0, 2), (2, 2), (4, 2), (6, 2), (8, 1), (9, 2), (11, 1)];
    let value_ranges = [(0, 6), (6, 8), (14, 6), (20, 4), (24, 3), (27, 8), (35, 5)];
    let extrapolations = [
        AnimationExtrapolation::SourceDefault,
        AnimationExtrapolation::Constant,
        AnimationExtrapolation::Linear,
        AnimationExtrapolation::Repeat,
    ];
    let channels = std::array::from_fn::<_, 7, _>(|index| {
        let (time_start, time_count) = time_ranges[index];
        let (value_start, value_count) = value_ranges[index];
        let tangent = (index < 4).then_some(TableRange::new(value_start, value_count));
        StoredAnimationChannel {
            source_index: [4, 8, 15, 42, 90, 125, 150][index],
            target: targets[index],
            interpolation: match index {
                0..=3 => AnimationInterpolation::CubicSpline,
                4 | 6 => AnimationInterpolation::Step,
                _ => AnimationInterpolation::Linear,
            },
            pre_extrapolation: extrapolations[index % 4],
            post_extrapolation: extrapolations[(index + 1) % 4],
            times: TableRange::new(time_start, time_count),
            values: TableRange::new(value_start, value_count),
            in_tangents: tangent,
            out_tangents: tangent,
        }
    })
    .to_vec();
    AnimationStorage {
        clips: vec![
            StoredAnimationClip {
                source_index: 3,
                name: Some(TextRange(TableRange::new(0, 3))),
                duration: f64::from_bits(1.0_f64.to_bits() + 1),
                time_domain: AnimationTimeDomain::Seconds,
                channels: TableRange::new(0, 2),
            },
            StoredAnimationClip {
                source_index: 9,
                name: Some(TextRange(TableRange::new(3, 0))),
                duration: -0.0,
                time_domain: AnimationTimeDomain::Frames {
                    rate: f64::from_bits(24.0_f64.to_bits() + 1),
                },
                channels: TableRange::new(2, 1),
            },
            StoredAnimationClip {
                source_index: 40,
                name: None,
                duration: 0.25,
                time_domain: AnimationTimeDomain::Ticks {
                    rate: Some(f64::from_bits(60.0_f64.to_bits() + 1)),
                },
                channels: TableRange::new(3, 1),
            },
            StoredAnimationClip {
                source_index: 100,
                name: Some(TextRange(TableRange::new(3, 3))),
                duration: 4.0,
                time_domain: AnimationTimeDomain::Ticks { rate: None },
                channels: TableRange::new(4, 3),
            },
            StoredAnimationClip {
                source_index: 150,
                name: None,
                duration: 0.0,
                time_domain: AnimationTimeDomain::Seconds,
                channels: TableRange::new(7, 0),
            },
        ],
        channels,
        times: vec![
            -2.5,
            f64::from_bits(2.0_f64.to_bits() + 1),
            -0.0,
            f64::from_bits(1),
            16_777_217.0,
            16_777_217.5,
            -100.25,
            f64::from_bits(1.0_f64.to_bits() + 1),
            1e100,
            -1e100,
            1e100,
            -f64::from_bits(1),
        ],
        values: vec![
            1.0,
            -2.0,
            3.0,
            -0.0,
            f32::from_bits(1),
            -4.0,
            0.5,
            -0.5,
            -0.5,
            0.5,
            -0.5,
            0.5,
            0.5,
            -0.5,
            -2.0,
            0.0,
            3.0,
            4.0,
            -5.0,
            -0.0,
            -0.0,
            0.25,
            -2.0,
            3.0,
            2.0,
            4.0,
            8.0,
            0.0,
            -0.0,
            0.0,
            1.0,
            -0.0,
            0.0,
            -0.0,
            -1.0,
            f32::MAX,
            f32::from_bits(1),
            -0.0,
            -3.0,
            4.0,
        ],
        in_tangents: vec![
            5.0,
            6.0,
            -7.0,
            0.0,
            -0.0,
            0.0,
            2.0,
            -3.0,
            4.0,
            5.0,
            -0.0,
            0.0,
            -0.0,
            0.0,
            2.0,
            -3.0,
            4.0,
            0.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            -0.0,
            4.0,
            -5.0,
        ],
        out_tangents: vec![
            -8.0, 9.0, 10.0, -1.0, 2.0, -3.0, 0.0, -0.0, 0.0, -0.0, -7.0, 11.0, -13.0, 17.0, -6.0,
            7.0, 8.0, -9.0, 10.0, -11.0, 0.0, -0.0, 6.0, -7.0,
        ],
        names: "mix\u{732b}".as_bytes().to_vec(),
    }
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn time_bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|value| value.to_bits()).collect()
}

#[test]
fn mixed_curves_have_hand_expected_values_derivatives_and_unchanged_time_and_row_bits() {
    let source = mixed();
    let mut preflight = Control::default();
    let plan = AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut preflight).unwrap();
    assert_eq!(preflight.completed, 95); // 7 channels + 40 values + 24 input + 24 output.
    assert!(plan.changes_storage());
    assert_eq!(plan.work(), 118); // 5 clips + 7 channels + 12 times + 40 + 24 + 24 + 6 bytes.
    assert_eq!(plan.correspondence_work(), 118);
    assert_eq!(plan.name_bytes(), 6);
    assert_eq!(
        plan.capacities().unwrap().map(|p| p.desired_capacity()),
        [5, 7, 12, 40, 24, 24, 6]
    );
    let mut build = Control::default();
    let actual = plan.materialize(&mut build).unwrap();
    assert_eq!(build.completed, 118);
    assert_eq!(build.attempted, 118);
    assert_eq!(build.string_completed, 6);
    assert_eq!(
        bits(&actual.values),
        bits(&[
            -6.0,
            2.0,
            -4.0,
            8.0,
            -0.0,
            f32::from_bits(2),
            -0.5,
            -0.5,
            0.5,
            0.5,
            0.5,
            0.5,
            -0.5,
            -0.5,
            3.0,
            -2.0,
            0.0,
            -0.0,
            4.0,
            -5.0,
            -0.0,
            0.25,
            -2.0,
            3.0,
            -16.0,
            4.0,
            8.0,
            0.0,
            -0.0,
            0.0,
            1.0,
            -0.0,
            0.0,
            -0.0,
            -1.0,
            f32::MAX,
            f32::from_bits(1),
            -0.0,
            -3.0,
            4.0,
        ])
    );
    assert_eq!(
        bits(&actual.in_tangents),
        bits(&[
            14.0,
            10.0,
            12.0,
            -0.0,
            0.0,
            -0.0,
            4.0,
            -2.0,
            3.0,
            5.0,
            -0.0,
            0.0,
            -0.0,
            0.0,
            4.0,
            2.0,
            -3.0,
            0.0,
            0.0,
            -0.0,
            f32::from_bits(1),
            -0.0,
            4.0,
            -5.0,
        ])
    );
    assert_eq!(
        bits(&actual.out_tangents),
        bits(&[
            -20.0, -16.0, 18.0, 6.0, -2.0, 4.0, 0.0, -0.0, 0.0, -0.0, -13.0, 7.0, -11.0, 17.0, 8.0,
            -6.0, 7.0, -11.0, -9.0, 10.0, 0.0, -0.0, 6.0, -7.0,
        ])
    );
    assert_eq!(time_bits(&actual.times), time_bits(&source.times));
    assert_eq!(actual.times[1].to_bits(), 2.0_f64.to_bits() + 1);
    assert_eq!(actual.times[2].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(actual.times[3].to_bits(), 1);
    assert_eq!(&actual.times[4..6], &[16_777_217.0, 16_777_217.5]);
    assert_eq!(actual.times[8], 1e100);
    assert_eq!(actual.times[9], -1e100);
    assert_eq!(actual.times[11].to_bits(), (-f64::from_bits(1)).to_bits());
    assert_eq!(actual.clips[0].duration.to_bits(), 1.0_f64.to_bits() + 1);
    assert_eq!(actual.clips[1].duration.to_bits(), (-0.0_f64).to_bits());
    let AnimationTimeDomain::Frames { rate } = actual.clips[1].time_domain else {
        panic!("frame domain was changed");
    };
    assert_eq!(rate.to_bits(), 24.0_f64.to_bits() + 1);
    let AnimationTimeDomain::Ticks { rate: Some(rate) } = actual.clips[2].time_domain else {
        panic!("declared tick rate was changed");
    };
    assert_eq!(rate.to_bits(), 60.0_f64.to_bits() + 1);
    assert_eq!(
        actual.clips[3].time_domain,
        AnimationTimeDomain::Ticks { rate: None }
    );
    assert_eq!(actual.names, "mix\u{732b}".as_bytes());
    assert_eq!(actual.clips[1].name.unwrap().get(&actual.names), Some(""));
    assert!(actual.clips[2].name.is_none());
    for (source, actual) in source.clips.iter().zip(&actual.clips) {
        assert_eq!(source.source_index, actual.source_index);
        assert_eq!(source.name, actual.name);
        assert_eq!(source.channels, actual.channels);
        assert_eq!(source.duration.to_bits(), actual.duration.to_bits());
        assert_eq!(source.time_domain, actual.time_domain);
    }
    for (source, actual) in source.channels.iter().zip(&actual.channels) {
        assert_eq!(source.source_index, actual.source_index);
        assert_eq!(source.target, actual.target);
        assert_eq!(source.interpolation, actual.interpolation);
        assert_eq!(source.pre_extrapolation, actual.pre_extrapolation);
        assert_eq!(source.post_extrapolation, actual.post_extrapolation);
        assert_eq!(source.times, actual.times);
        assert_eq!(source.values, actual.values);
        assert_eq!(source.in_tangents, actual.in_tangents);
        assert_eq!(source.out_tangents, actual.out_tangents);
    }
    let mut compared = Control::default();
    assert_eq!(
        plan.validate_correspondence(&actual, &mut compared),
        Ok(true)
    );
    assert_eq!(compared.completed, 118);
}

#[test]
fn reflection_maps_quaternion_values_and_arbitrary_vec4_derivatives_without_hemisphere_fixes() {
    let source = one_channel(
        StoredAnimationTarget::NodeRotation(StoredNodeIndex(1)),
        AnimationInterpolation::CubicSpline,
        &[-10.0, 10.0],
        &[0.5, 0.5, -0.5, 0.5, -0.5, -0.5, 0.5, -0.5],
        Some(&[2.0, -3.0, 4.0, -5.0, 0.0, -0.0, 0.0, -0.0]),
        Some(&[-7.0, 11.0, 13.0, 17.0, -0.0, 0.0, -0.0, 0.0]),
    );
    // Q=(-x,y,z); det(Q)Q=(x,-y,-z). The unit factor is deliberately enormous.
    let conversion = target_conversion(
        Handedness::Left,
        Axis::PositiveY,
        Axis::NegativeZ,
        1e-100,
        FrontFaceWinding::CounterClockwise,
    );
    let plan = AnimationSpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    assert_eq!(
        bits(&actual.values),
        bits(&[0.5, -0.5, 0.5, 0.5, -0.5, 0.5, -0.5, -0.5,])
    );
    assert_eq!(
        bits(&actual.in_tangents),
        bits(&[2.0, 3.0, -4.0, -5.0, 0.0, 0.0, -0.0, -0.0,])
    );
    assert_eq!(
        bits(&actual.out_tangents),
        bits(&[-7.0, -11.0, -13.0, 17.0, -0.0, -0.0, 0.0, 0.0,])
    );
    assert_eq!(actual.values[7], -0.5);
    assert_eq!(actual.in_tangents[3], -5.0);
    assert_eq!(actual.out_tangents[3], 17.0);
    assert_eq!(
        plan.validate_correspondence(&actual, &mut Control::default()),
        Ok(true)
    );
}

#[test]
fn unchanged_storage_depends_on_actual_bits_and_identity_or_winding_only_preserves_every_stream() {
    let source = mixed();
    let winding_only = target_conversion(
        Handedness::Right,
        Axis::PositiveY,
        Axis::NegativeZ,
        1.0,
        FrontFaceWinding::Clockwise,
    );
    for conversion in [units_conversion(1.0), winding_only] {
        let plan =
            AnimationSpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
        assert!(!plan.changes_storage());
        let actual = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(bits(&actual.values), bits(&source.values));
        assert_eq!(bits(&actual.in_tangents), bits(&source.in_tangents));
        assert_eq!(bits(&actual.out_tangents), bits(&source.out_tangents));
        assert_eq!(time_bits(&actual.times), time_bits(&source.times));
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(true)
        );
    }
    for target in [
        StoredAnimationTarget::NodeScale(StoredNodeIndex(1)),
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
    ] {
        let source = one_channel(
            target,
            AnimationInterpolation::CubicSpline,
            &[100.0],
            &[2.0, 2.0, 2.0],
            Some(&[-3.0, -3.0, -3.0]),
            Some(&[4.0, 4.0, 4.0]),
        );
        let plan =
            AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default())
                .unwrap();
        assert!(!plan.changes_storage());
    }
    for (target, values) in [
        (
            StoredAnimationTarget::NodeRotation(StoredNodeIndex(1)),
            &[0.0, -0.0, 0.0, 1.0][..],
        ),
        (
            StoredAnimationTarget::NodeScale(StoredNodeIndex(1)),
            &[-2.0, 0.0, f32::MAX][..],
        ),
        (
            StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
            &[f32::MAX, f32::from_bits(1), -0.0][..],
        ),
    ] {
        let source = one_channel(
            target,
            AnimationInterpolation::Step,
            &[0.0],
            values,
            None,
            None,
        );
        let plan =
            AnimationSpacePlan::try_new(&source, units_conversion(1e-100), &mut Control::default())
                .unwrap();
        assert!(!plan.changes_storage());
        let actual = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(bits(&actual.values), bits(values));
    }
    let source = one_channel(
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
        AnimationInterpolation::Step,
        &[0.0],
        &[0.0, -0.0, 0.0],
        None,
        None,
    );
    let units =
        AnimationSpacePlan::try_new(&source, units_conversion(0.5), &mut Control::default())
            .unwrap();
    assert!(!units.changes_storage());
    let reflected =
        AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()).unwrap();
    assert!(
        reflected.changes_storage(),
        "zero sign changes still require rewriting bits"
    );
}

#[test]
fn numeric_failures_locate_channel_stream_and_key_in_values_and_both_derivative_pools() {
    for part in [
        SceneAnimationSamplePart::Values,
        SceneAnimationSamplePart::InTangents,
        SceneAnimationSamplePart::OutTangents,
    ] {
        for (invalid, units, expected) in [
            (f32::MAX, 0.5, NumericConversionError::NonFiniteOutput),
            (
                f32::from_bits(1),
                2.0,
                NumericConversionError::UnderflowToZero,
            ),
        ] {
            let mut source = mixed();
            source.values[4] = 1.0; // Avoid an earlier subnormal translation underflow.
            match part {
                SceneAnimationSamplePart::Values => source.values[3] = invalid,
                SceneAnimationSamplePart::InTangents => source.in_tangents[3] = invalid,
                SceneAnimationSamplePart::OutTangents => source.out_tangents[3] = invalid,
            }
            assert!(matches!(
                AnimationSpacePlan::try_new(&source, units_conversion(units), &mut Control::default()),
                Err(AnimationSpacePlanError::Numeric { channel: 0, part: actual, key: 1, source })
                    if actual == part && source == expected
            ));
        }
    }
    let mut source = mixed();
    source.out_tangents[13] = f32::INFINITY; // Rotation derivative w at key 1.
    assert!(matches!(
        AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()),
        Err(AnimationSpacePlanError::Numeric {
            channel: 1,
            part: SceneAnimationSamplePart::OutTangents,
            key: 1,
            source: NumericConversionError::NonFiniteOutput,
        })
    ));
}

#[test]
fn malformed_stream_shape_or_coverage_fails_without_uncontrolled_numeric_scans() {
    for fault in 0..7 {
        let mut source = mixed();
        match fault {
            0 => source.channels[0].values = TableRange::new(0, 5),
            1 => source.channels[0].out_tangents = None,
            2 => source.channels[0].in_tangents = Some(TableRange::new(0, 3)),
            3 => source.channels[1].times = TableRange::new(0, 2),
            4 => source.channels[4].in_tangents = Some(TableRange::new(24, 0)),
            5 => source.values.push(1.0),
            _ => source.in_tangents.push(1.0),
        }
        assert!(
            matches!(
                AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()),
                Err(AnimationSpacePlanError::Invariant)
            ),
            "fault {fault}"
        );
    }
}

#[test]
fn preflight_and_correspondence_allocate_nothing_and_shape_queries_are_fixed_size() {
    fn require_copy<T: Copy>() {}
    require_copy::<AnimationSpacePlan<'_>>();
    assert!(
        std::mem::size_of::<AnimationSpacePlan<'_>>() < std::mem::size_of::<[CapacityPlan; 7]>()
    );
    let source = mixed();
    let conversion = mixed_conversion();
    let mut planned = None;
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        planned = Some(AnimationSpacePlan::try_new(&source, conversion, &mut control).unwrap());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 95);
    let plan = planned.unwrap();
    let allocations = allocation_counter::measure(|| {
        for _ in 0..16 {
            assert_eq!(plan.work(), 118);
            assert_eq!(plan.correspondence_work(), 118);
            assert_eq!(plan.name_bytes(), 6);
            assert_eq!(
                plan.capacities().unwrap().map(|p| p.desired_capacity()),
                [5, 7, 12, 40, 24, 24, 6]
            );
        }
    });
    assert_eq!(allocations.count_total, 0);
    let actual = plan.materialize(&mut Control::default()).unwrap();
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(true)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 118);

    let weights = vec![-0.0; 4_096];
    let source = one_channel(
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
        AnimationInterpolation::Linear,
        &[-1e100],
        &weights,
        None,
        None,
    );
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        let plan = AnimationSpacePlan::try_new(&source, conversion, &mut control).unwrap();
        assert!(!plan.changes_storage());
        assert_eq!(plan.work(), 4_099);
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(control.completed, 4_097);
    let mut control = Control {
        reject_before: Some(1),
        ..Control::default()
    };
    assert!(matches!(
        AnimationSpacePlan::try_new(&source, conversion, &mut control),
        Err(AnimationSpacePlanError::Control(1))
    ));
}

#[test]
fn every_preflight_and_correspondence_action_is_required_and_control_errors_keep_the_prefix() {
    let source = mixed();
    let conversion = mixed_conversion();
    for stop in 0..95 {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                AnimationSpacePlan::try_new(&source, conversion, &mut control),
                Err(AnimationSpacePlanError::Control(actual)) if actual == stop
            ));
        });
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(allocations.count_total, 0);
    }
    let plan = AnimationSpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    for stop in 0..118 {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Err(stop)
        );
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        let mut control = Control {
            omit_before: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(false)
        );
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
    }
}

#[test]
fn every_materialization_action_releases_all_seven_buffers_on_error_and_unwind() {
    let source = mixed();
    let before_values = bits(&source.values);
    let before_input = bits(&source.in_tangents);
    let before_output = bits(&source.out_tangents);
    let plan =
        AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()).unwrap();
    for stop in 0..118 {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                plan.materialize(&mut control),
                Err(TypedStorageBuildError::Control(actual)) if actual == stop
            ));
        });
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
        let mut control = Control {
            panic_before: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            let unwind = catch_unwind(AssertUnwindSafe(|| plan.materialize(&mut control)));
            assert!(unwind.is_err());
            drop(unwind);
        });
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
    }
    assert_eq!(bits(&source.values), before_values);
    assert_eq!(bits(&source.in_tangents), before_input);
    assert_eq!(bits(&source.out_tangents), before_output);
}

#[test]
fn correspondence_rejects_each_unchanged_field_presence_range_and_bit_mutation() {
    let source = mixed();
    let plan =
        AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()).unwrap();
    for fault in 0..31 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        match fault {
            0 => actual.clips[0].source_index += 1,
            1 => actual.clips[0].name = None,
            2 => actual.clips[1].name = None, // Present empty name is not absent.
            3 => actual.clips[0].name = Some(TextRange(TableRange::new(1, 3))),
            4 => actual.clips[1].duration = 0.0, // -0 versus +0.
            5 => actual.clips[0].time_domain = AnimationTimeDomain::Ticks { rate: None },
            6 => actual.clips[1].time_domain = AnimationTimeDomain::Frames { rate: 24.0 },
            7 => actual.clips[2].time_domain = AnimationTimeDomain::Ticks { rate: Some(60.0) },
            8 => actual.clips[3].time_domain = AnimationTimeDomain::Ticks { rate: Some(1.0) },
            9 => actual.clips[0].channels = TableRange::new(1, 2),
            10 => actual.channels[0].source_index += 1,
            11 => actual.channels[0].target = StoredAnimationTarget::NodeScale(StoredNodeIndex(1)),
            12 => {
                actual.channels[0].target =
                    StoredAnimationTarget::NodeTranslation(StoredNodeIndex(2))
            }
            13 => actual.channels[0].interpolation = AnimationInterpolation::Linear,
            14 => actual.channels[0].pre_extrapolation = AnimationExtrapolation::Repeat,
            15 => actual.channels[0].post_extrapolation = AnimationExtrapolation::Linear,
            16 => actual.channels[0].times = TableRange::new(1, 2),
            17 => actual.channels[0].values = TableRange::new(1, 6),
            18 => actual.channels[0].in_tangents = None,
            19 => actual.channels[0].out_tangents = None,
            20 => actual.channels[0].in_tangents = Some(TableRange::new(1, 6)),
            21 => actual.channels[0].out_tangents = Some(TableRange::new(1, 6)),
            22 => actual.times[2] = 0.0,
            23 => actual.values[5] = f32::from_bits(3),
            24 => actual.in_tangents[0] = f32::from_bits(actual.in_tangents[0].to_bits() + 1),
            25 => actual.out_tangents[0] = f32::from_bits(actual.out_tangents[0].to_bits() + 1),
            26 => actual.values[20] = 0.0, // Unchanged morph-weight signed zero.
            27 => actual.in_tangents[20] = f32::from_bits(2), // Unchanged morph derivative.
            28 => actual.names[0] ^= 1,
            29 => actual.values[34] = 1.0, // Same orientation, wrong q/-q representative.
            _ => actual.times[1] = 2.0,
        }
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false),
            "fault {fault}"
        );
    }
    for pool in 0..7 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        match pool {
            0 => {
                actual.clips.pop();
            }
            1 => {
                actual.channels.pop();
            }
            2 => {
                actual.times.pop();
            }
            3 => {
                actual.values.pop();
            }
            4 => {
                actual.in_tangents.pop();
            }
            5 => {
                actual.out_tangents.pop();
            }
            _ => {
                actual.names.pop();
            }
        }
        let mut control = Control::default();
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(false)
        );
        assert_eq!(control.completed, 0);
    }
    let mut foreign = mixed();
    foreign.values[0] = 19.0;
    let other =
        AnimationSpacePlan::try_new(&foreign, mixed_conversion(), &mut Control::default()).unwrap();
    let actual = other.materialize(&mut Control::default()).unwrap();
    assert_eq!(
        plan.validate_correspondence(&actual, &mut Control::default()),
        Ok(false)
    );
}

#[test]
fn channel_selection_has_exact_work_for_mixed_targets_and_unchanged_morph_scalars() {
    let source = mixed();
    let conversion = mixed_conversion();
    let expected_work = [19, 25, 19, 13, 4, 9, 6];
    let expected_changed = [true, true, true, false, true, false, false];
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        // A later row must not scan the prefix or depend on an earlier query.
        for index in [6, 2, 5, 0, 4, 1, 3] {
            let before = control.completed;
            assert_eq!(
                channel_changes(&source, index, conversion, &mut control).unwrap(),
                expected_changed[index]
            );
            assert_eq!(control.completed - before, expected_work[index]);
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    let aggregate = source.channels.len() as u64
        + source.values.len() as u64
        + source.in_tangents.len() as u64
        + source.out_tangents.len() as u64;
    assert_eq!(aggregate, 95);
    assert_eq!(control.completed, aggregate);
    assert_eq!(control.attempted, aggregate);
    assert_eq!(control.string_completed, 0);

    // Identity still compares every scalar; integration bypasses the selector
    // entirely for pure winding/declaration operations instead of changing cost.
    let identity = units_conversion(1.0);
    let mut control = Control::default();
    for index in 0..source.channels.len() {
        assert!(!channel_changes(&source, index, identity, &mut control).unwrap());
    }
    assert_eq!(control.completed, aggregate);

    let weights = vec![-0.0; 4_096];
    let input = vec![f32::from_bits(1); 4_096];
    let output = vec![f32::MAX; 4_096];
    let source = one_channel(
        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(0)),
        AnimationInterpolation::CubicSpline,
        &[-1e100],
        &weights,
        Some(&input),
        Some(&output),
    );
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert!(!channel_changes(&source, 0, conversion, &mut control).unwrap());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 12_289);
    assert_eq!(control.attempted, 12_289);
}

#[test]
fn channel_selection_finds_late_changes_and_finishes_every_stream_after_early_changes() {
    let conversion = units_conversion(0.5); // Q is identity and s=2.
    for change in 0..5 {
        let mut source = one_channel(
            StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
            AnimationInterpolation::CubicSpline,
            &[-100.0, 100.0],
            &[0.0; 6],
            Some(&[0.0; 6]),
            Some(&[0.0; 6]),
        );
        match change {
            0 => {} // Every scalar is unchanged.
            1 => source.values[5] = 7.0,
            2 => source.in_tangents[5] = 7.0,
            3 => source.out_tangents[5] = 7.0, // First difference is the final scalar.
            _ => source.values[0] = 7.0,       // Still visit all 17 later scalars.
        }
        let mut control = Control::default();
        assert_eq!(
            channel_changes(&source, 0, conversion, &mut control).unwrap(),
            change != 0,
            "change {change}"
        );
        assert_eq!(control.completed, 19);
        assert_eq!(control.attempted, 19);
        assert_eq!(control.string_completed, 0);
    }
}

#[test]
fn channel_selection_requires_every_control_action_even_after_finding_a_change() {
    let source = mixed();
    let conversion = mixed_conversion();
    // Channel 1 changes at its first value. Its final scalar action is still
    // required, with no short circuit after the changed bit becomes true.
    for stop in 0..25 {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                channel_changes(&source, 1, conversion, &mut control),
                Err(actual) if actual == stop
            ));
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
    }
    let mut control = Control::default();
    assert_eq!(
        channel_changes(&source, source.channels.len(), conversion, &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 1);
    assert_eq!(control.attempted, 1);
}

#[test]
fn channel_selection_does_not_hide_late_numeric_failure_behind_an_earlier_change() {
    let source = one_channel(
        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(1)),
        AnimationInterpolation::CubicSpline,
        &[0.0, 1.0],
        &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        Some(&[0.0; 6]),
        Some(&[0.0, 0.0, 0.0, 0.0, 0.0, f32::MAX]),
    );
    let conversion = units_conversion(0.5);
    assert!(matches!(
        AnimationSpacePlan::try_new(&source, conversion, &mut Control::default()),
        Err(AnimationSpacePlanError::Numeric {
            channel: 0,
            part: SceneAnimationSamplePart::OutTangents,
            key: 1,
            ..
        })
    ));
    let mut control = Control::default();
    assert_eq!(
        channel_changes(&source, 0, conversion, &mut control),
        Ok(false)
    );
    assert_eq!(control.completed, 19);
    assert_eq!(control.attempted, 19);
}

#[test]
fn empty_and_channel_free_owners_preserve_presence_without_inventing_payload_work() {
    let source = empty();
    let mut control = Control::default();
    let plan = AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut control).unwrap();
    assert_eq!(control.completed, 0);
    assert_eq!(plan.work(), 0);
    assert_eq!(plan.correspondence_work(), 0);
    assert_eq!(plan.name_bytes(), 0);
    assert!(!plan.changes_storage());
    assert_eq!(
        plan.capacities().unwrap().map(|p| p.desired_capacity()),
        [0; 7]
    );
    let allocations = allocation_counter::measure(|| {
        let actual = plan.materialize(&mut control).unwrap();
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(true)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(control.completed, 0);

    let mut source = empty();
    source.names = b"empty".to_vec();
    source.clips.push(StoredAnimationClip {
        source_index: 9,
        name: Some(TextRange(TableRange::new(0, 5))),
        duration: -0.0,
        time_domain: AnimationTimeDomain::Ticks { rate: None },
        channels: TableRange::new(0, 0),
    });
    let plan =
        AnimationSpacePlan::try_new(&source, mixed_conversion(), &mut Control::default()).unwrap();
    assert!(!plan.changes_storage());
    assert_eq!(plan.work(), 6);
    let mut control = Control::default();
    let actual = plan.materialize(&mut control).unwrap();
    assert_eq!(control.completed, 6);
    assert_eq!(control.string_completed, 5);
    assert_eq!(actual.clips[0].duration.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(actual.names, b"empty");
    assert_eq!(
        plan.validate_correspondence(&actual, &mut Control::default()),
        Ok(true)
    );
}
