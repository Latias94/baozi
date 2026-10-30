use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::entity::{CameraProjectionKind, LightKindCategory, validate_canonical_light_emission};

use super::*;

impl SpatialStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        expected_cameras: usize,
        expected_lights: usize,
        expected_name_bytes: usize,
    ) -> Result<bool, C::Error> {
        if self.cameras.len() != expected_cameras
            || self.lights.len() != expected_lights
            || schema_name_bytes(&self.space) != expected_name_bytes
        {
            return Ok(false);
        }
        if control.run_unit(|| self.space.validate().is_ok())? != Some(true) {
            return Ok(false);
        }
        for byte in schema_name_iter(&self.space) {
            if control.run_unit(|| byte.is_ascii())? != Some(true) {
                return Ok(false);
            }
        }
        for &camera in &self.cameras {
            if control.run_unit(|| valid_camera(camera))? != Some(true) {
                return Ok(false);
            }
        }
        for light in &self.lights {
            if control.run_unit(|| valid_light(*light))? != Some(true) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn valid_camera(camera: CameraProjection) -> bool {
    match camera.kind() {
        CameraProjectionKind::Perspective => {
            let Some((fov, aspect, near, far)) = camera.perspective_parameters() else {
                return false;
            };
            CameraProjection::perspective(fov, aspect, near, far).is_ok()
        }
        CameraProjectionKind::Orthographic => {
            let Some((x, y, near, far)) = camera.orthographic_parameters() else {
                return false;
            };
            near.to_bits() != (-0.0_f32).to_bits()
                && CameraProjection::orthographic(x, y, near, far).is_ok()
        }
    }
}

fn valid_light(light: StoredLight) -> bool {
    let kind_valid = match light.kind.category() {
        LightKindCategory::Directional => {
            light.kind.range().is_none()
                && light.kind.inner_cone_radians().is_none()
                && light.kind.outer_cone_radians().is_none()
        }
        LightKindCategory::Point => LightKind::point(light.kind.range()).is_ok(),
        LightKindCategory::Spot => match (
            light.kind.inner_cone_radians(),
            light.kind.outer_cone_radians(),
        ) {
            (Some(inner), Some(outer)) => LightKind::spot(light.kind.range(), inner, outer).is_ok(),
            _ => false,
        },
    };
    kind_valid && validate_canonical_light_emission(light.tint, light.intensity).is_ok()
}
