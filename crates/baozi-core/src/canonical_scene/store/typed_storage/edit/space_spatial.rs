//! Source-bound conversion of the complete typed spatial owner.
//!
//! Intrinsic axes compose as Q B, including reflections. Distances use the positive unit ratio;
//! angular, aspect, and photometric values retain their source bits. The source row order and all
//! node placements remain unchanged.
//!
//! Preflight, materialization, and correspondence each charge one inline SceneSpace action, one
//! action per inline schema-name byte, and one action per camera/light. The enclosing retained
//! lane admits the existing two capacities; this module owns no lease or selection cache.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::entity::{
    CameraProjection, CameraProjectionError, LightDefinition, LightKind, LightKindCategory,
};
use crate::scene_space::SceneSpace;
use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, finish,
};
use super::super::spatial::SpatialStorage;
use super::super::spatial::build::SpatialStoragePlan;
use super::super::validation::TypedStorageValidationControl;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum SpatialSpacePlanError<E> {
    Control(E),
    NumericCamera {
        camera: usize,
        source: NumericConversionError,
    },
    CameraProjection {
        camera: usize,
        source: CameraProjectionError,
    },
    NumericLight {
        light: usize,
        source: NumericConversionError,
    },
    Plan(StorePlanError),
    Invariant,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct SpatialSpacePlan<'a> {
    source: &'a SpatialStorage,
    conversion: SpaceConversion,
    shape: SpatialStoragePlan,
    work: u64,
}

impl<'a> SpatialSpacePlan<'a> {
    /// The source has passed central validation. Check every converted row before retained
    /// admission, including projection constraints after the final f32 rounding.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a SpatialStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, SpatialSpacePlanError<C::Error>> {
        let shape = SpatialStoragePlan::for_storage(source).map_err(SpatialSpacePlanError::Plan)?;
        let work = shape.work().map_err(SpatialSpacePlanError::Plan)?;
        let valid = control
            .run_unit(|| {
                converted_space(&source.space, conversion)
                    .validate()
                    .is_ok()
            })
            .map_err(SpatialSpacePlanError::Control)?;
        if !valid {
            return Err(SpatialSpacePlanError::Invariant);
        }
        for byte in schema_name_iter(&source.space) {
            control
                .run_string_unit(|| byte)
                .map_err(SpatialSpacePlanError::Control)?;
        }
        for (camera, &value) in source.cameras.iter().enumerate() {
            control
                .run_unit(|| convert_camera(value, conversion))
                .map_err(SpatialSpacePlanError::Control)?
                .map_err(|error| match error {
                    CameraConversionError::Numeric(source) => {
                        SpatialSpacePlanError::NumericCamera { camera, source }
                    }
                    CameraConversionError::Projection(source) => {
                        SpatialSpacePlanError::CameraProjection { camera, source }
                    }
                    CameraConversionError::Invariant => SpatialSpacePlanError::Invariant,
                })?;
        }
        for (light, &value) in source.lights.iter().enumerate() {
            control
                .run_unit(|| convert_light(value, conversion))
                .map_err(SpatialSpacePlanError::Control)?
                .map_err(|error| match error {
                    LightConversionError::Numeric(source) => {
                        SpatialSpacePlanError::NumericLight { light, source }
                    }
                    LightConversionError::Invariant => SpatialSpacePlanError::Invariant,
                })?;
        }
        Ok(Self {
            source,
            conversion,
            shape,
            work,
        })
    }

    pub(in crate::canonical_scene::store) fn capacities(self) -> [CapacityPlan; 2] {
        self.shape.capacities()
    }

    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.shape.name_bytes()
    }

    /// Local buffers drop before an error or unwind returns to the enclosing retained lane.
    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> BuildResult<SpatialStorage, C::Error> {
        let [camera_capacity, light_capacity] = self.capacities();
        let mut cameras = allocate(camera_capacity)?;
        let mut lights = allocate(light_capacity)?;
        let space = control
            .run_unit(|| converted_space(&self.source.space, self.conversion))
            .map_err(TypedStorageBuildError::Control)?;
        for byte in schema_name_iter(&self.source.space) {
            control
                .run_string_unit(|| byte)
                .map_err(TypedStorageBuildError::Control)?;
        }
        for &source in &self.source.cameras {
            if cameras.len() >= camera_capacity.desired_capacity()
                || cameras.len() == cameras.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_unit(|| {
                    let converted = convert_camera(source, self.conversion)
                        .map_err(|_| TypedStorageBuildError::Invariant)?;
                    cameras.push(converted);
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        for &source in &self.source.lights {
            if lights.len() >= light_capacity.desired_capacity()
                || lights.len() == lights.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_unit(|| {
                    let converted = convert_light(source, self.conversion)
                        .map_err(|_| TypedStorageBuildError::Invariant)?;
                    lights.push(converted);
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        finish(&cameras, camera_capacity)?;
        finish(&lights, light_capacity)?;
        Ok(SpatialStorage {
            space,
            cameras,
            lights,
        })
    }

    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &SpatialStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if SpatialStoragePlan::for_storage(actual).ok() != Some(self.shape) {
            return Ok(false);
        }
        if control.run_unit(|| {
            same_space(
                &converted_space(&self.source.space, self.conversion),
                &actual.space,
            )
        })? != Some(true)
        {
            return Ok(false);
        }
        for (source, actual) in
            schema_name_iter(&self.source.space).zip(schema_name_iter(&actual.space))
        {
            if control.run_unit(|| source == actual)? != Some(true) {
                return Ok(false);
            }
        }
        for (&source, &actual) in self.source.cameras.iter().zip(&actual.cameras) {
            if control.run_unit(|| {
                convert_camera(source, self.conversion)
                    .is_ok_and(|expected| same_camera(expected, actual))
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        for (&source, &actual) in self.source.lights.iter().zip(&actual.lights) {
            if control.run_unit(|| {
                convert_light(source, self.conversion)
                    .is_ok_and(|expected| same_light(expected, actual))
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Called inside the profile row action after complete numeric preflight. Each definition is
/// fixed-size, so selection needs neither an extra metered traversal nor retained scratch.
pub(in crate::canonical_scene::store) fn camera_changes(
    source: &SpatialStorage,
    index: usize,
    conversion: SpaceConversion,
) -> bool {
    source.cameras.get(index).is_some_and(|&source| {
        convert_camera(source, conversion).is_ok_and(|converted| !same_camera(source, converted))
    })
}

pub(in crate::canonical_scene::store) fn light_changes(
    source: &SpatialStorage,
    index: usize,
    conversion: SpaceConversion,
) -> bool {
    source.lights.get(index).is_some_and(|&source| {
        convert_light(source, conversion).is_ok_and(|converted| !same_light(source, converted))
    })
}

#[derive(Debug)]
enum CameraConversionError {
    Numeric(NumericConversionError),
    Projection(CameraProjectionError),
    Invariant,
}

fn convert_camera(
    source: CameraProjection,
    conversion: SpaceConversion,
) -> Result<CameraProjection, CameraConversionError> {
    let axes = conversion
        .local_axes(source.axes())
        .ok_or(CameraConversionError::Invariant)?;
    let length = |value| {
        conversion
            .length(value)
            .map_err(CameraConversionError::Numeric)
    };
    match source {
        CameraProjection::Perspective(source) => CameraProjection::perspective(
            axes,
            source.vertical_fov_radians(),
            source.aspect_ratio(),
            length(source.near())?,
            source.far().map(length).transpose()?,
        ),
        CameraProjection::Orthographic(source) => CameraProjection::orthographic(
            axes,
            length(source.horizontal_magnification())?,
            length(source.vertical_magnification())?,
            length(source.near())?,
            length(source.far())?,
        ),
    }
    .map_err(CameraConversionError::Projection)
}

#[derive(Debug)]
enum LightConversionError {
    Numeric(NumericConversionError),
    Invariant,
}

fn convert_light(
    source: LightDefinition,
    conversion: SpaceConversion,
) -> Result<LightDefinition, LightConversionError> {
    let axes = conversion
        .local_axes(source.axes())
        .ok_or(LightConversionError::Invariant)?;
    let kind = source.kind();
    let range = kind
        .range()
        .map(|range| conversion.length(range))
        .transpose()
        .map_err(LightConversionError::Numeric)?;
    let converted_kind = match kind.category() {
        LightKindCategory::Directional => LightKind::directional(),
        LightKindCategory::Point => {
            LightKind::point(range).map_err(|_| LightConversionError::Invariant)?
        }
        LightKindCategory::Spot => LightKind::spot(
            range,
            kind.inner_cone_radians()
                .ok_or(LightConversionError::Invariant)?,
            kind.outer_cone_radians()
                .ok_or(LightConversionError::Invariant)?,
        )
        .map_err(|_| LightConversionError::Invariant)?,
    };
    // Reuse canonical emission directly; re-normalizing source color would lose exact bits.
    Ok(LightDefinition::new(
        converted_kind,
        axes,
        source.emission(),
    ))
}

fn converted_space(source: &SceneSpace, conversion: SpaceConversion) -> SceneSpace {
    let target = conversion.target();
    SceneSpace {
        handedness: target.handedness(),
        up_axis: target.up_axis(),
        front_axis: target.front_axis(),
        unit_scale_to_meters: Some(target.unit_scale_to_meters()),
        front_face_winding: target.front_face_winding(),
        ..source.clone()
    }
}

fn schema_name_iter(space: &SceneSpace) -> impl Iterator<Item = u8> + '_ {
    space
        .tangent_basis
        .schema_id()
        .into_iter()
        .chain(space.tangent_handedness_encoding.schema_id())
        .flat_map(|schema| schema.name().as_str().bytes())
}

fn same_space(source: &SceneSpace, actual: &SceneSpace) -> bool {
    source == actual
        && source.unit_scale_to_meters.map(f64::to_bits)
            == actual.unit_scale_to_meters.map(f64::to_bits)
}

fn same_camera(source: CameraProjection, actual: CameraProjection) -> bool {
    match (source, actual) {
        (CameraProjection::Perspective(source), CameraProjection::Perspective(actual)) => {
            source.axes() == actual.axes()
                && source.vertical_fov_radians().to_bits()
                    == actual.vertical_fov_radians().to_bits()
                && same_optional_float(source.aspect_ratio(), actual.aspect_ratio())
                && source.near().to_bits() == actual.near().to_bits()
                && same_optional_float(source.far(), actual.far())
        }
        (CameraProjection::Orthographic(source), CameraProjection::Orthographic(actual)) => {
            source.axes() == actual.axes()
                && source.horizontal_magnification().to_bits()
                    == actual.horizontal_magnification().to_bits()
                && source.vertical_magnification().to_bits()
                    == actual.vertical_magnification().to_bits()
                && source.near().to_bits() == actual.near().to_bits()
                && source.far().to_bits() == actual.far().to_bits()
        }
        _ => false,
    }
}

fn same_light(source: LightDefinition, actual: LightDefinition) -> bool {
    let source_kind = source.kind();
    let actual_kind = actual.kind();
    source.axes() == actual.axes()
        && source_kind.category() == actual_kind.category()
        && same_optional_float(source_kind.range(), actual_kind.range())
        && same_optional_float(
            source_kind.inner_cone_radians(),
            actual_kind.inner_cone_radians(),
        )
        && same_optional_float(
            source_kind.outer_cone_radians(),
            actual_kind.outer_cone_radians(),
        )
        && same_emission(source, actual)
}

fn same_emission(source: LightDefinition, actual: LightDefinition) -> bool {
    let source_tint = source.tint();
    let actual_tint = actual.tint();
    source_tint.x.to_bits() == actual_tint.x.to_bits()
        && source_tint.y.to_bits() == actual_tint.y.to_bits()
        && source_tint.z.to_bits() == actual_tint.z.to_bits()
        && source.intensity().to_bits() == actual.intensity().to_bits()
}

fn same_optional_float(source: Option<f32>, actual: Option<f32>) -> bool {
    source.map(f32::to_bits) == actual.map(f32::to_bits)
}
