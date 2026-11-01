use super::SpatialStorage;
use super::build::SpatialStoragePlan;
use crate::LocalAxes;
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::entity::{CameraProjection, LightDefinition, LightEmission, LightKind};
use crate::math::{Color, Vec3};
use crate::scene_space::{Axis, SceneSpace};

#[derive(Debug, PartialEq, Eq)]
struct Interrupted(usize);

#[derive(Default)]
struct Control {
    attempted: usize,
    completed: usize,
    reject_before: Option<usize>,
    missing_before: Option<usize>,
}

impl TypedStorageValidationControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        self.attempted += 1;
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.missing_before == Some(self.completed) {
            return Ok(None);
        }
        let value = action();
        self.completed += 1;
        Ok(Some(value))
    }
}

fn storage() -> SpatialStorage {
    let white =
        |intensity| LightEmission::try_from_scaled_linear_srgb(Color::WHITE, intensity).unwrap();
    SpatialStorage {
        space: SceneSpace::STANDARD_TARGET,
        cameras: vec![
            CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, f32::from_bits(1), None)
                .unwrap(),
            CameraProjection::orthographic(
                LocalAxes::STANDARD,
                f32::from_bits(1),
                f32::MAX,
                -0.0,
                f32::MAX,
            )
            .unwrap(),
        ],
        lights: vec![
            LightDefinition::new(LightKind::directional(), LocalAxes::STANDARD, white(0.0)),
            LightDefinition::new(
                LightKind::point(Some(f32::from_bits(1))).unwrap(),
                LocalAxes::STANDARD,
                white(f32::from_bits(1)),
            ),
            LightDefinition::new(
                LightKind::spot(None, -0.0, std::f32::consts::FRAC_PI_2).unwrap(),
                LocalAxes::STANDARD,
                white(f32::MAX),
            ),
        ],
    }
}

#[test]
fn actual_closed_storage_validates_space_and_numeric_boundaries_without_allocation() {
    let storage = storage();
    let before_space = storage.space.clone();
    let before_cameras = storage.cameras.clone();
    let before_lights = storage.lights.clone();
    let camera_ptr = storage.cameras.as_ptr();
    let light_ptr = storage.lights.as_ptr();
    let mut control = Control::default();

    let allocations = allocation_counter::measure(|| {
        assert_eq!(storage.validate(&mut control, 2, 3), Ok(true));
    });

    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 6);
    assert_eq!(control.attempted, 6);
    assert_eq!(storage.space, before_space);
    assert_eq!(storage.cameras, before_cameras);
    assert_eq!(storage.lights, before_lights);
    assert_eq!(storage.cameras.as_ptr(), camera_ptr);
    assert_eq!(storage.lights.as_ptr(), light_ptr);
    let CameraProjection::Orthographic(projection) = storage.cameras[1] else {
        panic!("orthographic projection expected");
    };
    assert_eq!(projection.axes(), LocalAxes::STANDARD);
    assert_eq!(projection.near().to_bits(), 0);
    assert_eq!(projection.horizontal_magnification().to_bits(), 1);
    assert_eq!(
        projection.vertical_magnification().to_bits(),
        f32::MAX.to_bits()
    );
    assert_eq!(projection.far().to_bits(), f32::MAX.to_bits());
    assert_eq!(
        storage.lights[2]
            .kind()
            .inner_cone_radians()
            .unwrap()
            .to_bits(),
        0,
    );
}

#[test]
fn plan_accounts_for_two_row_capacities_and_one_inline_space_action() {
    let plan = SpatialStoragePlan::for_shape(2, 3).unwrap();
    assert_eq!(
        plan.capacities().map(|plan| plan.desired_capacity()),
        [2, 3],
    );
    assert_eq!(plan.work().unwrap(), 6);
    assert_eq!(plan.validation_work().unwrap(), 6);
    assert_eq!(
        plan.canonical_bytes().unwrap(),
        (std::mem::size_of::<SpatialStorage>()
            + 2 * std::mem::size_of::<CameraProjection>()
            + 3 * std::mem::size_of::<LightDefinition>()) as u64,
    );
}

#[test]
fn empty_partial_space_and_finite_unit_extremes_need_one_validation_action() {
    for scale in [None, Some(f64::from_bits(1)), Some(f64::MAX)] {
        let storage = SpatialStorage {
            space: SceneSpace {
                unit_scale_to_meters: scale,
                ..SceneSpace::UNKNOWN
            },
            cameras: vec![],
            lights: vec![],
        };
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 0, 0), Ok(true));
        assert_eq!(control.completed, 1);
    }
    let plan = SpatialStoragePlan::for_shape(0, 0).unwrap();
    assert_eq!(plan.validation_work().unwrap(), 1);
}

#[test]
fn stale_counts_are_rejected_before_any_validation_action() {
    let storage = storage();
    for (cameras, lights) in [(1, 3), (2, 4), (usize::MAX, 3), (2, usize::MAX)] {
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, cameras, lights), Ok(false));
        assert_eq!(control.attempted, 0);
    }
}

#[test]
fn actual_inline_space_rejects_nonpositive_and_nonfinite_units() {
    let mut storage = storage();
    for scale in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY] {
        storage.space.unit_scale_to_meters = Some(scale);
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 2, 3), Ok(false));
        assert_eq!(control.attempted, 1);
        assert_eq!(control.completed, 1);
    }
}

#[test]
fn actual_inline_space_rejects_coincident_axes() {
    for front_axis in [Axis::PositiveY, Axis::NegativeY] {
        let storage = SpatialStorage {
            space: SceneSpace {
                front_axis,
                ..SceneSpace::STANDARD_TARGET
            },
            cameras: vec![],
            lights: vec![],
        };
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 0, 0), Ok(false));
        assert_eq!(control.attempted, 1);
        assert_eq!(control.completed, 1);
    }
}

#[test]
fn actual_light_rows_reject_invalid_emission_without_recanonicalizing_it() {
    let emission = LightEmission::try_from_scaled_linear_srgb(
        Color {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        },
        10.0,
    )
    .unwrap();
    let canonical = LightDefinition::new(
        LightKind::point(None).unwrap(),
        LocalAxes::STANDARD,
        emission,
    );
    let mut storage = SpatialStorage {
        space: SceneSpace::UNKNOWN,
        cameras: vec![],
        lights: vec![canonical, canonical],
    };
    assert_eq!(storage.validate(&mut Control::default(), 0, 2), Ok(true));
    for (tint, intensity) in [
        (Vec3::new(f32::NAN, 1.0, 1.0), 1.0),
        (Vec3::new(-1.0, 1.0, 1.0), 1.0),
        (Vec3::new(1.0, 1.0, 1.0), f32::INFINITY),
        (Vec3::new(1.0, 1.0, 1.0), -1.0),
        (Vec3::new(1.0, 1.0, 1.0), -0.0),
        (Vec3::ZERO, 0.0),
        (Vec3::new(2.0, 2.0, 2.0), 1.0),
        (
            Vec3::new(-0.0, canonical.tint().y, canonical.tint().z),
            canonical.intensity(),
        ),
    ] {
        storage.lights[1] = canonical.with_emission_parts_for_test(tint, intensity);
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 0, 2), Ok(false));
        assert_eq!(control.attempted, 3);
        assert_eq!(control.completed, 3);
    }
}

#[test]
fn validation_preserves_exact_control_errors_and_rejects_missing_actions_at_phase_boundaries() {
    let storage = storage();
    // Space, both cameras, and every light remain independent interruption boundaries.
    for completed in 0..6 {
        let mut control = Control {
            reject_before: Some(completed),
            ..Control::default()
        };
        assert_eq!(
            storage.validate(&mut control, 2, 3),
            Err(Interrupted(completed)),
        );
        assert_eq!(control.completed, completed);
        assert_eq!(control.attempted, completed + 1);
    }
    for completed in 0..6 {
        let mut control = Control {
            missing_before: Some(completed),
            ..Control::default()
        };
        assert_eq!(storage.validate(&mut control, 2, 3), Ok(false));
        assert_eq!(control.completed, completed);
        assert_eq!(control.attempted, completed + 1);
    }
    let mut exact = Control {
        reject_before: Some(6),
        ..Control::default()
    };
    assert_eq!(storage.validate(&mut exact, 2, 3), Ok(true));
    assert_eq!(exact.completed, 6);
    assert_eq!(exact.attempted, 6);
}
