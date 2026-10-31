use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;

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
    camera.validate()
}

fn valid_light(light: LightDefinition) -> bool {
    light.validate()
}
