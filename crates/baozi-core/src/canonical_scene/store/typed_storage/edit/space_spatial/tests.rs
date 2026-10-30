use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::extension::ExtensionSchemaId;
use crate::math::{Color, Vec3};
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, TangentBasisConvention, TangentHandednessEncoding, UvOrigin,
};
use crate::{LocalAxes, TargetSpace};

use super::*;

#[derive(Default)]
struct Control {
    completed: u64,
    attempted: u64,
    strings: u64,
    stop: Option<u64>,
    omit: Option<u64>,
    panic_before: Option<u64>,
    panic_after: Option<u64>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, u64> {
        let index = self.completed;
        self.attempted += 1;
        if self.stop == Some(index) {
            return Err(index);
        }
        if self.panic_before == Some(index) {
            // Bypass the global panic hook so allocation probes only measure local storage
            // and the caught payload, without test-output capture allocations.
            std::panic::resume_unwind(Box::new(index));
        }
        let result = action();
        self.completed += 1;
        self.strings += u64::from(string);
        if self.panic_after == Some(index) {
            std::panic::resume_unwind(Box::new(index));
        }
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
        if self.omit == Some(self.completed) {
            self.attempted += 1;
            return Ok(None);
        }
        self.execute(false, action).map(Some)
    }
}

fn target(units: f64, winding: FrontFaceWinding) -> TargetSpace {
    TargetSpace::try_new(
        Handedness::Right,
        Axis::PositiveY,
        Axis::NegativeZ,
        units,
        winding,
    )
    .unwrap()
}

fn units(units: f64) -> SpaceConversion {
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        target(units, FrontFaceWinding::CounterClockwise),
        None,
    )
    .unwrap()
}

fn reflected() -> SpaceConversion {
    // Hand-calculated: Q(x,y,z)=(-x,-z,y), det(Q)=-1, s=2.
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveZ,
            Axis::PositiveY,
            0.5,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn unusual_axes() -> LocalAxes {
    LocalAxes::try_new(Axis::NegativeZ, Axis::NegativeX, Axis::PositiveY).unwrap()
}

fn reflected_standard_axes() -> LocalAxes {
    LocalAxes::try_new(Axis::NegativeX, Axis::PositiveZ, Axis::PositiveY).unwrap()
}

fn reflected_unusual_axes() -> LocalAxes {
    LocalAxes::try_new(Axis::PositiveY, Axis::PositiveX, Axis::PositiveZ).unwrap()
}

fn named_space() -> SceneSpace {
    SceneSpace {
        uv_origin: UvOrigin::UpperLeft,
        tangent_basis: TangentBasisConvention::Namespaced {
            schema_id: ExtensionSchemaId::new("test:basis", 17).unwrap(),
        },
        tangent_handedness_encoding: TangentHandednessEncoding::Namespaced {
            schema_id: ExtensionSchemaId::new("test:sign", 29).unwrap(),
        },
        ..SceneSpace::STANDARD_TARGET
    }
}

fn light(kind: LightKind, axes: LocalAxes, intensity: f32) -> LightDefinition {
    LightDefinition::try_from_scaled_linear_srgb(
        kind,
        axes,
        Color::linear_rgba(2.0, 0.0, 0.5, 1.0),
        intensity,
    )
    .unwrap()
}

fn fixture() -> SpatialStorage {
    SpatialStorage {
        space: named_space(),
        cameras: vec![
            CameraProjection::perspective(
                LocalAxes::STANDARD,
                f32::from_bits(0x3f90_0001),
                Some(f32::from_bits(0x3fe0_0001)),
                0.125,
                Some(128.0),
            )
            .unwrap(),
            CameraProjection::perspective(unusual_axes(), 0.75, None, 0.25, None).unwrap(),
            CameraProjection::orthographic(unusual_axes(), 2.0, 3.0, 0.0, 64.0).unwrap(),
        ],
        lights: vec![
            light(LightKind::directional(), LocalAxes::STANDARD, 7.0),
            light(
                LightKind::point(Some(8.0)).unwrap(),
                unusual_axes(),
                f32::from_bits(0x4080_0001),
            ),
            light(LightKind::point(None).unwrap(), LocalAxes::STANDARD, 1.0),
            light(
                LightKind::spot(Some(32.0), 0.25, 0.75).unwrap(),
                unusual_axes(),
                11.0,
            ),
            light(
                LightKind::spot(None, -0.0, std::f32::consts::FRAC_PI_2).unwrap(),
                LocalAxes::STANDARD,
                0.0,
            ),
        ],
    }
}

fn just_camera(camera: CameraProjection) -> SpatialStorage {
    SpatialStorage {
        space: SceneSpace::STANDARD_TARGET,
        cameras: vec![camera],
        lights: Vec::new(),
    }
}

fn just_light(light: LightDefinition) -> SpatialStorage {
    SpatialStorage {
        space: SceneSpace::STANDARD_TARGET,
        cameras: Vec::new(),
        lights: vec![light],
    }
}

#[test]
fn reflection_permutation_and_units_convert_complete_definitions_in_source_order() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    assert_eq!(actual.cameras.len(), 3);
    assert_eq!(actual.lights.len(), 5);
    let CameraProjection::Perspective(first) = actual.cameras[0] else {
        panic!("perspective source row");
    };
    assert_eq!(first.axes(), reflected_standard_axes());
    assert_eq!(first.vertical_fov_radians().to_bits(), 0x3f90_0001);
    assert_eq!(first.aspect_ratio().map(f32::to_bits), Some(0x3fe0_0001));
    assert_eq!((first.near(), first.far()), (0.25, Some(256.0)));
    let CameraProjection::Perspective(second) = actual.cameras[1] else {
        panic!("unbounded perspective source row");
    };
    assert_eq!(second.axes(), reflected_unusual_axes());
    assert_eq!(second.vertical_fov_radians(), 0.75);
    assert_eq!(second.aspect_ratio(), None);
    assert_eq!((second.near(), second.far()), (0.5, None));
    let CameraProjection::Orthographic(ortho) = actual.cameras[2] else {
        panic!("orthographic source row");
    };
    assert_eq!(ortho.axes(), reflected_unusual_axes());
    assert_eq!(
        (
            ortho.horizontal_magnification(),
            ortho.vertical_magnification(),
            ortho.near(),
            ortho.far()
        ),
        (4.0, 6.0, 0.0, 128.0)
    );
    assert_eq!(ortho.near().to_bits(), 0);
    assert_eq!(actual.lights[0].axes(), reflected_standard_axes());
    assert_eq!(actual.lights[1].axes(), reflected_unusual_axes());
    assert_eq!(actual.lights[2].axes(), reflected_standard_axes());
    assert_eq!(actual.lights[3].axes(), reflected_unusual_axes());
    assert_eq!(actual.lights[4].axes(), reflected_standard_axes());
    assert_eq!(
        actual.lights[0].kind().category(),
        LightKindCategory::Directional
    );
    assert_eq!(actual.lights[1].kind().range(), Some(16.0));
    assert_eq!(actual.lights[2].kind().range(), None);
    assert_eq!(actual.lights[3].kind().range(), Some(64.0));
    assert_eq!(actual.lights[4].kind().range(), None);
    for (&source, &actual) in source.lights.iter().zip(&actual.lights) {
        assert_eq!(source.kind().category(), actual.kind().category());
        assert!(same_optional_float(
            source.kind().inner_cone_radians(),
            actual.kind().inner_cone_radians()
        ));
        assert!(same_optional_float(
            source.kind().outer_cone_radians(),
            actual.kind().outer_cone_radians()
        ));
        assert!(same_emission(source, actual));
    }
    assert_eq!(actual.space.handedness, Handedness::Left);
    assert_eq!(actual.space.up_axis, Axis::PositiveZ);
    assert_eq!(actual.space.front_axis, Axis::PositiveY);
    assert_eq!(
        actual.space.unit_scale_to_meters.map(f64::to_bits),
        Some(0.5_f64.to_bits())
    );
    assert_eq!(
        actual.space.front_face_winding,
        FrontFaceWinding::CounterClockwise
    );
    assert_eq!(actual.space.uv_origin, source.space.uv_origin);
    assert_eq!(actual.space.tangent_basis, source.space.tangent_basis);
    assert_eq!(
        actual.space.tangent_handedness_encoding,
        source.space.tangent_handedness_encoding
    );
    assert_eq!(
        plan.validate_correspondence(&actual, &mut Control::default()),
        Ok(true)
    );
    assert_eq!(actual.validate(&mut Control::default(), 3, 5, 19), Ok(true));
}

#[test]
fn all_48_intrinsic_frames_round_trip_reflections_without_rebuilding_a_missing_axis() {
    let axes = [
        Axis::PositiveX,
        Axis::NegativeX,
        Axis::PositiveY,
        Axis::NegativeY,
        Axis::PositiveZ,
        Axis::NegativeZ,
    ];
    let conversion = reflected();
    let converted = converted_space(&SceneSpace::STANDARD_TARGET, conversion);
    let inverse = SpaceConversion::try_new(&converted, TargetSpace::STANDARD, None).unwrap();
    let mut count = 0;
    for right in axes {
        for up in axes {
            for forward in axes {
                let Ok(frame) = LocalAxes::try_new(right, up, forward) else {
                    continue;
                };
                let camera =
                    CameraProjection::perspective(frame, 1.0, None, 0.125, Some(64.0)).unwrap();
                let converted = convert_camera(camera, conversion).unwrap();
                assert!(same_camera(
                    camera,
                    convert_camera(converted, inverse).unwrap()
                ));
                let light = light(LightKind::spot(Some(8.0), 0.25, 0.5).unwrap(), frame, 3.0);
                let converted = convert_light(light, conversion).unwrap();
                assert!(same_light(
                    light,
                    convert_light(converted, inverse).unwrap()
                ));
                count += 1;
            }
        }
    }
    assert_eq!(count, 48);
    // A reflected right axis cannot be recovered using the original handedness.
    assert_eq!(reflected_standard_axes().right(), Axis::NegativeX);
}

#[test]
fn optional_camera_fields_and_unscaled_angles_keep_their_exact_bits() {
    for aspect in [None, Some(f32::MAX)] {
        for far in [None, Some(4.0)] {
            let source = CameraProjection::perspective(
                LocalAxes::STANDARD,
                f32::from_bits(1),
                aspect,
                1.0,
                far,
            )
            .unwrap();
            let CameraProjection::Perspective(actual) = convert_camera(source, units(0.5)).unwrap()
            else {
                panic!("perspective remains perspective");
            };
            assert_eq!(actual.vertical_fov_radians().to_bits(), 1);
            assert_eq!(
                actual.aspect_ratio().map(f32::to_bits),
                aspect.map(f32::to_bits)
            );
            assert_eq!(actual.near(), 2.0);
            assert_eq!(actual.far(), far.map(|far| far * 2.0));
        }
    }
}

#[test]
fn camera_numeric_failures_identify_the_row_and_precede_materialization() {
    let tiny = f32::from_bits(1);
    for (camera, conversion, expected) in [
        (
            CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, tiny, None).unwrap(),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
        (
            CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, f32::MAX, None).unwrap(),
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, 1.0, Some(f32::MAX))
                .unwrap(),
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, tiny, 1.0, 0.0, 4.0).unwrap(),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, tiny, 0.0, 4.0).unwrap(),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, 1.0, tiny, 4.0).unwrap(),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, 1.0, 0.0, tiny).unwrap(),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, f32::MAX, 1.0, 0.0, 4.0).unwrap(),
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, f32::MAX, 0.0, 4.0).unwrap(),
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, 1.0, 0.0, f32::MAX).unwrap(),
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
    ] {
        let mut source = fixture();
        source.cameras[2] = camera;
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                SpatialSpacePlan::try_new(&source, conversion, &mut Control::default()),
                Err(SpatialSpacePlanError::NumericCamera { camera: 2, source })
                    if source == expected
            ));
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
}

#[test]
fn nonzero_planes_that_round_to_the_same_f32_are_rejected_as_projection_errors() {
    // Halving adjacent values at the normal/subnormal boundary rounds both to 0x00400000.
    let near = f32::MIN_POSITIVE;
    let far = f32::from_bits(near.to_bits() + 1);
    for camera in [
        CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, near, Some(far)).unwrap(),
        CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, 2.0, near, far).unwrap(),
    ] {
        let source = just_camera(camera);
        assert!(matches!(
            SpatialSpacePlan::try_new(&source, units(2.0), &mut Control::default()),
            Err(SpatialSpacePlanError::CameraProjection {
                camera: 0,
                source: CameraProjectionError::FarPlane
            })
        ));
    }
    let source = just_camera(
        CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, near, None).unwrap(),
    );
    let plan = SpatialSpacePlan::try_new(&source, units(2.0), &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    let CameraProjection::Perspective(actual) = actual.cameras[0] else {
        panic!("unbounded projection");
    };
    assert_eq!(actual.near().to_bits(), 0x0040_0000);
    assert_eq!(actual.far(), None);
}

#[test]
fn orthographic_zero_near_and_representable_subnormal_extents_remain_valid() {
    let source = just_camera(
        CameraProjection::orthographic(
            LocalAxes::STANDARD,
            f32::from_bits(2),
            f32::from_bits(4),
            -0.0,
            f32::from_bits(6),
        )
        .unwrap(),
    );
    let plan = SpatialSpacePlan::try_new(&source, units(2.0), &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    let CameraProjection::Orthographic(actual) = actual.cameras[0] else {
        panic!("orthographic projection");
    };
    assert_eq!(actual.horizontal_magnification().to_bits(), 1);
    assert_eq!(actual.vertical_magnification().to_bits(), 2);
    assert_eq!(actual.near().to_bits(), 0);
    assert_eq!(actual.far().to_bits(), 3);
}

#[test]
fn light_range_failures_are_numeric_and_do_not_scale_photometric_intensity_or_cones() {
    for (range, conversion, expected) in [
        (
            f32::MAX,
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            f32::from_bits(1),
            units(4.0),
            NumericConversionError::UnderflowToZero,
        ),
    ] {
        for kind in [
            LightKind::point(Some(range)).unwrap(),
            LightKind::spot(Some(range), 0.125, 0.75).unwrap(),
        ] {
            let mut source = fixture();
            source.lights[4] = light(kind, unusual_axes(), 2.0);
            assert!(matches!(
                SpatialSpacePlan::try_new(&source, conversion, &mut Control::default()),
                Err(SpatialSpacePlanError::NumericLight { light: 4, source })
                    if source == expected
            ));
        }
    }
    for kind in [
        LightKind::directional(),
        LightKind::point(None).unwrap(),
        LightKind::spot(None, f32::from_bits(1), std::f32::consts::FRAC_PI_2).unwrap(),
    ] {
        let source = LightDefinition::try_from_scaled_linear_srgb(
            kind,
            unusual_axes(),
            Color::WHITE,
            f32::MAX,
        )
        .unwrap();
        let owner = just_light(source);
        let plan = SpatialSpacePlan::try_new(&owner, units(0.5), &mut Control::default()).unwrap();
        let actual = plan.materialize(&mut Control::default()).unwrap();
        assert!(same_light(source, actual.lights[0]));
    }
}

#[test]
fn preflight_counts_two_capacities_inline_names_and_every_row_without_allocating() {
    let source = fixture();
    let mut control = Control::default();
    let mut result = None;
    let allocations = allocation_counter::measure(|| {
        result = Some(SpatialSpacePlan::try_new(&source, reflected(), &mut control).unwrap());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 28);
    assert_eq!(control.attempted, 28);
    assert_eq!(control.strings, 19);
    let plan = result.unwrap();
    assert_eq!(
        plan.capacities()
            .map(|capacity| capacity.desired_capacity()),
        [3, 5]
    );
    assert_eq!(plan.work(), 28);
    assert_eq!(plan.correspondence_work(), 28);
    assert_eq!(plan.name_bytes(), 19);
    let mut control = Control::default();
    let actual = plan.materialize(&mut control).unwrap();
    assert_eq!(control.completed, 28);
    assert_eq!(control.strings, 19);
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(true)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 28);
    assert_eq!(control.attempted, 28);
}

#[test]
fn every_preflight_and_correspondence_action_preserves_control_failures() {
    let source = fixture();
    let conversion = reflected();
    let plan = SpatialSpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
    let actual = plan.materialize(&mut Control::default()).unwrap();
    for stop in 0..28 {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                SpatialSpacePlan::try_new(&source, conversion, &mut control),
                Err(SpatialSpacePlanError::Control(actual)) if actual == stop
            ));
        });
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(allocations.count_total, 0);
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Err(stop)
        );
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        let mut control = Control {
            omit: Some(stop),
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
fn every_materialization_action_releases_both_buffers_on_interruption_and_unwind() {
    let source = fixture();
    let original_cameras = source.cameras.clone();
    let original_lights = source.lights.clone();
    let original_space = source.space.clone();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for stop in 0..28 {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                plan.materialize(&mut control),
                Err(TypedStorageBuildError::Control(actual)) if actual == stop
            ));
        });
        assert_eq!(control.completed, stop);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
        for after in [false, true] {
            let mut control = Control {
                panic_before: (!after).then_some(stop),
                panic_after: after.then_some(stop),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                let unwind = catch_unwind(AssertUnwindSafe(|| plan.materialize(&mut control)));
                assert!(unwind.is_err());
                drop(unwind);
            });
            assert_eq!(control.completed, stop + u64::from(after));
            assert_eq!(control.attempted, stop + 1);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
        }
    }
    assert!(same_space(&source.space, &original_space));
    assert!(
        source
            .cameras
            .iter()
            .zip(original_cameras)
            .all(|(&a, b)| same_camera(a, b))
    );
    assert!(
        source
            .lights
            .iter()
            .zip(original_lights)
            .all(|(&a, b)| same_light(a, b))
    );
}

#[test]
fn a_materialized_owner_also_releases_both_buffers_on_later_host_unwind() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let allocations = allocation_counter::measure(|| {
        let unwind = catch_unwind(AssertUnwindSafe(|| {
            let actual = plan.materialize(&mut Control::default()).unwrap();
            std::hint::black_box(&actual);
            std::panic::resume_unwind(Box::new(1_u64));
        }));
        assert!(unwind.is_err());
        drop(unwind);
    });
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}

fn change_perspective(camera: CameraProjection, fault: usize) -> CameraProjection {
    let CameraProjection::Perspective(source) = camera else {
        panic!("perspective fixture")
    };
    CameraProjection::perspective(
        if fault == 0 {
            LocalAxes::STANDARD
        } else {
            source.axes()
        },
        if fault == 1 {
            f32::from_bits(source.vertical_fov_radians().to_bits() + 1)
        } else {
            source.vertical_fov_radians()
        },
        if fault == 2 {
            None
        } else if fault == 3 {
            Some(2.0)
        } else {
            source.aspect_ratio()
        },
        if fault == 4 {
            source.near() * 2.0
        } else {
            source.near()
        },
        if fault == 5 {
            None
        } else if fault == 6 {
            Some(512.0)
        } else {
            source.far()
        },
    )
    .unwrap()
}

fn change_orthographic(camera: CameraProjection, fault: usize) -> CameraProjection {
    let CameraProjection::Orthographic(source) = camera else {
        panic!("orthographic fixture")
    };
    CameraProjection::orthographic(
        if fault == 0 {
            LocalAxes::STANDARD
        } else {
            source.axes()
        },
        source.horizontal_magnification() + if fault == 1 { 1.0 } else { 0.0 },
        source.vertical_magnification() + if fault == 2 { 1.0 } else { 0.0 },
        if fault == 3 { 1.0 } else { source.near() },
        source.far() + if fault == 4 { 1.0 } else { 0.0 },
    )
    .unwrap()
}

#[test]
fn correspondence_rejects_projection_variants_all_parameters_presence_and_row_order() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for fault in 0..7 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        actual.cameras[0] = change_perspective(actual.cameras[0], fault);
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false)
        );
    }
    for fault in 0..5 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        actual.cameras[2] = change_orthographic(actual.cameras[2], fault);
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false)
        );
    }
    for fault in 0..4 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        match fault {
            0 => actual.cameras.swap(0, 1),
            1 => actual.cameras[0] = actual.cameras[2],
            2 => {
                actual.cameras[1] = CameraProjection::perspective(
                    reflected_unusual_axes(),
                    0.75,
                    Some(1.0),
                    0.5,
                    None,
                )
                .unwrap()
            }
            _ => {
                actual.cameras[1] = CameraProjection::perspective(
                    reflected_unusual_axes(),
                    0.75,
                    None,
                    0.5,
                    Some(256.0),
                )
                .unwrap()
            }
        }
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false)
        );
    }
}

#[test]
fn correspondence_rejects_light_kind_axes_ranges_cones_emission_bits_and_order() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for fault in 0..12 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        let spot = actual.lights[3];
        match fault {
            0 => {
                actual.lights[3] = LightDefinition::new(
                    LightKind::point(spot.kind().range()).unwrap(),
                    spot.axes(),
                    spot.emission(),
                )
            }
            1 => {
                actual.lights[3] =
                    LightDefinition::new(spot.kind(), LocalAxes::STANDARD, spot.emission())
            }
            2 => {
                actual.lights[3] = LightDefinition::new(
                    LightKind::spot(None, 0.25, 0.75).unwrap(),
                    spot.axes(),
                    spot.emission(),
                )
            }
            3 => {
                actual.lights[3] = LightDefinition::new(
                    LightKind::spot(Some(128.0), 0.25, 0.75).unwrap(),
                    spot.axes(),
                    spot.emission(),
                )
            }
            4 => {
                actual.lights[3] = LightDefinition::new(
                    LightKind::spot(Some(64.0), 0.125, 0.75).unwrap(),
                    spot.axes(),
                    spot.emission(),
                )
            }
            5 => {
                actual.lights[3] = LightDefinition::new(
                    LightKind::spot(Some(64.0), 0.25, 1.0).unwrap(),
                    spot.axes(),
                    spot.emission(),
                )
            }
            6 => {
                actual.lights[3] = spot.with_emission_parts_for_test(
                    Vec3 {
                        x: f32::from_bits(spot.tint().x.to_bits() + 1),
                        ..spot.tint()
                    },
                    spot.intensity(),
                )
            }
            7 => {
                actual.lights[3] = spot.with_emission_parts_for_test(
                    Vec3 {
                        y: -0.0,
                        ..spot.tint()
                    },
                    spot.intensity(),
                )
            }
            8 => {
                actual.lights[3] = spot.with_emission_parts_for_test(
                    spot.tint(),
                    f32::from_bits(spot.intensity().to_bits() + 1),
                )
            }
            9 => {
                actual.lights[4] =
                    actual.lights[4].with_emission_parts_for_test(actual.lights[4].tint(), -0.0)
            }
            10 => {
                actual.lights[3] = spot.with_emission_parts_for_test(
                    Vec3 {
                        z: f32::from_bits(spot.tint().z.to_bits() + 1),
                        ..spot.tint()
                    },
                    spot.intensity(),
                )
            }
            _ => actual.lights.swap(1, 2),
        }
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false),
            "fault {fault}"
        );
    }
}

#[test]
fn correspondence_checks_all_space_facts_uv_schema_versions_and_names() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for fault in 0..11 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        match fault {
            0 => actual.space.handedness = Handedness::Right,
            1 => actual.space.up_axis = Axis::NegativeZ,
            2 => actual.space.front_axis = Axis::NegativeY,
            3 => actual.space.unit_scale_to_meters = Some(1.0),
            4 => actual.space.front_face_winding = FrontFaceWinding::Clockwise,
            5 => actual.space.uv_origin = UvOrigin::LowerLeft,
            6 => {
                actual.space.tangent_basis = TangentBasisConvention::Namespaced {
                    schema_id: ExtensionSchemaId::new("test:basis", 18).unwrap(),
                }
            }
            7 => {
                actual.space.tangent_handedness_encoding = TangentHandednessEncoding::Namespaced {
                    schema_id: ExtensionSchemaId::new("test:sign", 30).unwrap(),
                }
            }
            8 => {
                actual.space.tangent_basis = TangentBasisConvention::Namespaced {
                    schema_id: ExtensionSchemaId::new("test:other", 17).unwrap(),
                }
            }
            9 => {
                actual.space.tangent_handedness_encoding = TangentHandednessEncoding::Namespaced {
                    schema_id: ExtensionSchemaId::new("test:else", 29).unwrap(),
                }
            }
            _ => actual.space.unit_scale_to_meters = None,
        }
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(false)
        );
    }
}

#[test]
fn correspondence_rejects_changed_shape_and_same_shape_foreign_source_values() {
    let source = fixture();
    let plan = SpatialSpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for fault in 0..3 {
        let mut actual = plan.materialize(&mut Control::default()).unwrap();
        match fault {
            0 => {
                actual.cameras.pop();
            }
            1 => {
                actual.lights.pop();
            }
            _ => {
                actual.space.tangent_basis = TangentBasisConvention::Namespaced {
                    schema_id: ExtensionSchemaId::new("test:x", 17).unwrap(),
                }
            }
        }
        let mut control = Control::default();
        assert_eq!(
            plan.validate_correspondence(&actual, &mut control),
            Ok(false)
        );
        assert_eq!(control.attempted, 0);
    }
    let mut foreign = fixture();
    foreign.lights[1] = light(LightKind::point(Some(16.0)).unwrap(), unusual_axes(), 7.0);
    let other = SpatialSpacePlan::try_new(&foreign, reflected(), &mut Control::default()).unwrap();
    let actual = other.materialize(&mut Control::default()).unwrap();
    assert_eq!(
        plan.validate_correspondence(&actual, &mut Control::default()),
        Ok(false)
    );
}

#[test]
fn profile_selection_uses_actual_row_bits_and_needs_no_extra_traversal_or_allocation() {
    let source = fixture();
    let winding = SpaceConversion::try_new(
        &source.space,
        target(1.0, FrontFaceWinding::Clockwise),
        None,
    )
    .unwrap();
    let almost_identity = units(f64::from_bits(1.0_f64.to_bits() + 1));
    for (conversion, expected_cameras, expected_lights) in [
        (units(1.0), [false; 3], [false; 5]),
        (winding, [false; 3], [false; 5]),
        (almost_identity, [false; 3], [false; 5]),
        (units(0.5), [true; 3], [false, true, false, true, false]),
        (reflected(), [true; 3], [true; 5]),
    ] {
        // The selectors are only called after this complete numeric preflight succeeds.
        let _plan =
            SpatialSpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
        let allocations = allocation_counter::measure(|| {
            for (index, expected) in expected_cameras.into_iter().enumerate() {
                assert_eq!(camera_changes(&source, index, conversion), expected);
            }
            for (index, expected) in expected_lights.into_iter().enumerate() {
                assert_eq!(light_changes(&source, index, conversion), expected);
            }
            assert!(!camera_changes(&source, usize::MAX, conversion));
            assert!(!light_changes(&source, usize::MAX, conversion));
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
}

#[test]
fn declaration_only_and_empty_owners_keep_exact_work_and_preserve_uv_and_tangent_facts() {
    let source = SpatialStorage {
        space: SceneSpace {
            uv_origin: UvOrigin::LowerLeft,
            ..SceneSpace::UNKNOWN
        },
        cameras: Vec::new(),
        lights: Vec::new(),
    };
    let conversion = SpaceConversion::try_new(
        &source.space,
        TargetSpace::STANDARD,
        Some(TargetSpace::STANDARD),
    )
    .unwrap();
    let mut control = Control::default();
    let plan = SpatialSpacePlan::try_new(&source, conversion, &mut control).unwrap();
    assert_eq!(control.completed, 1);
    assert_eq!(plan.work(), 1);
    assert_eq!(plan.correspondence_work(), 1);
    assert_eq!(plan.name_bytes(), 0);
    assert_eq!(
        plan.capacities()
            .map(|capacity| capacity.desired_capacity()),
        [0, 0]
    );
    let allocations = allocation_counter::measure(|| {
        let mut control = Control::default();
        let actual = plan.materialize(&mut control).unwrap();
        assert_eq!(control.completed, 1);
        assert_eq!(control.strings, 0);
        assert_eq!(actual.space.uv_origin, UvOrigin::LowerLeft);
        assert_eq!(actual.space.tangent_basis, TangentBasisConvention::Unknown);
        assert_eq!(
            actual.space.tangent_handedness_encoding,
            TangentHandednessEncoding::Unknown
        );
        assert_eq!(
            plan.validate_correspondence(&actual, &mut Control::default()),
            Ok(true)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}
