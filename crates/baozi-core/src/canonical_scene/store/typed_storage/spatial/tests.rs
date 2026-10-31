use super::build::SpatialStoragePlan;
use super::{SpatialStorage, schema_name_bytes};
use crate::LocalAxes;
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::entity::{CameraProjection, LightDefinition, LightEmission, LightKind};
use crate::extension::ExtensionSchemaId;
use crate::math::{Color, Vec3};
use crate::namespaced::MAX_NAMESPACED_NAME_LEN;
use crate::scene_space::{Axis, SceneSpace, TangentBasisConvention, TangentHandednessEncoding};

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

fn named_space() -> SceneSpace {
    let basis = format!("test:{}", "a".repeat(MAX_NAMESPACED_NAME_LEN - 5));
    let encoding = format!("test:{}", "b".repeat(MAX_NAMESPACED_NAME_LEN - 5));
    SceneSpace {
        tangent_basis: TangentBasisConvention::Namespaced {
            schema_id: ExtensionSchemaId::new(basis, u32::MAX).unwrap(),
        },
        tangent_handedness_encoding: TangentHandednessEncoding::Namespaced {
            schema_id: ExtensionSchemaId::new(encoding, 1).unwrap(),
        },
        ..SceneSpace::STANDARD_TARGET
    }
}

fn storage() -> SpatialStorage {
    let white =
        |intensity| LightEmission::try_from_scaled_linear_srgb(Color::WHITE, intensity).unwrap();
    SpatialStorage {
        space: named_space(),
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
fn actual_closed_storage_validates_inline_names_and_numeric_boundaries_without_allocation() {
    let storage = storage();
    let before_space = storage.space.clone();
    let before_cameras = storage.cameras.clone();
    let before_lights = storage.lights.clone();
    let camera_ptr = storage.cameras.as_ptr();
    let light_ptr = storage.lights.as_ptr();
    let mut control = Control::default();

    let allocations = allocation_counter::measure(|| {
        assert_eq!(storage.validate(&mut control, 2, 3, 128), Ok(true));
    });

    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 134);
    assert_eq!(control.attempted, 134);
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
fn plan_accounts_for_only_two_row_capacities_and_inline_name_work() {
    let named = SpatialStoragePlan::for_shape(2, 3, 128).unwrap();
    let nameless = SpatialStoragePlan::for_shape(2, 3, 0).unwrap();
    assert_eq!(
        named.capacities().map(|plan| plan.desired_capacity()),
        [2, 3],
    );
    assert_eq!(named.work().unwrap(), 134);
    assert_eq!(named.validation_work().unwrap(), 134);
    assert_eq!(nameless.work().unwrap(), 6);
    assert_eq!(named.canonical_bytes(), nameless.canonical_bytes());
    assert!(
        SpatialStoragePlan::for_shape(0, 0, u64::MAX)
            .and_then(|plan| plan.work())
            .is_err(),
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
        assert_eq!(storage.validate(&mut control, 0, 0, 0), Ok(true));
        assert_eq!(control.completed, 1);
    }
    let plan = SpatialStoragePlan::for_shape(0, 0, 0).unwrap();
    assert_eq!(plan.validation_work().unwrap(), 1);
}

#[test]
fn stale_counts_are_rejected_before_any_validation_action() {
    let storage = storage();
    for (cameras, lights, names) in [(1, 3, 128), (2, 4, 128), (2, 3, 127), (2, 3, usize::MAX)] {
        let mut control = Control::default();
        assert_eq!(
            storage.validate(&mut control, cameras, lights, names),
            Ok(false)
        );
        assert_eq!(control.attempted, 0);
    }
}

#[test]
fn actual_inline_space_rejects_nonpositive_and_nonfinite_units() {
    let mut storage = storage();
    for scale in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY] {
        storage.space.unit_scale_to_meters = Some(scale);
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 2, 3, 128), Ok(false));
        assert_eq!(control.attempted, 1);
        assert_eq!(control.completed, 1);
    }
}

#[test]
fn actual_inline_space_rejects_coincident_axes_and_incomplete_or_conflicting_tangents() {
    let encoding = TangentHandednessEncoding::Namespaced {
        schema_id: ExtensionSchemaId::new("test:sign", 1).unwrap(),
    };
    for space in [
        SceneSpace {
            front_axis: Axis::NegativeY,
            ..SceneSpace::STANDARD_TARGET
        },
        SceneSpace {
            tangent_basis: TangentBasisConvention::MikkTspace,
            ..SceneSpace::UNKNOWN
        },
        SceneSpace {
            tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
            ..SceneSpace::UNKNOWN
        },
        SceneSpace {
            tangent_basis: TangentBasisConvention::MikkTspace,
            tangent_handedness_encoding: encoding,
            ..SceneSpace::UNKNOWN
        },
    ] {
        let names = schema_name_bytes(&space);
        let storage = SpatialStorage {
            space,
            cameras: vec![],
            lights: vec![],
        };
        let mut control = Control::default();
        assert_eq!(storage.validate(&mut control, 0, 0, names), Ok(false));
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
    assert_eq!(storage.validate(&mut Control::default(), 0, 2, 0), Ok(true));
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
        assert_eq!(storage.validate(&mut control, 0, 2, 0), Ok(false));
        assert_eq!(control.attempted, 3);
        assert_eq!(control.completed, 3);
    }
}

#[test]
fn validation_preserves_exact_control_errors_and_rejects_missing_actions_at_phase_boundaries() {
    let storage = storage();
    // Space, inline names, first camera, first light, and last light boundaries.
    for completed in [0, 1, 128, 129, 131, 133] {
        let mut control = Control {
            reject_before: Some(completed),
            ..Control::default()
        };
        assert_eq!(
            storage.validate(&mut control, 2, 3, 128),
            Err(Interrupted(completed)),
        );
        assert_eq!(control.completed, completed);
        assert_eq!(control.attempted, completed + 1);
    }
    for completed in [0, 133] {
        let mut control = Control {
            missing_before: Some(completed),
            ..Control::default()
        };
        assert_eq!(storage.validate(&mut control, 2, 3, 128), Ok(false));
        assert_eq!(control.completed, completed);
        assert_eq!(control.attempted, completed + 1);
    }
    let mut exact = Control {
        reject_before: Some(134),
        ..Control::default()
    };
    assert_eq!(storage.validate(&mut exact, 2, 3, 128), Ok(true));
    assert_eq!(exact.completed, 134);
    assert_eq!(exact.attempted, 134);
}
