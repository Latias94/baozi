//! Source-bound conversion of Scene-unit appearance values without copying immutable payloads.
//!
//! Only supplemental material thickness and displacement binding strength are lengths. A positive
//! unit ratio scales both, independently of handedness; all other fields retain their exact bits.
//! Preflight checks every binding and material. Materialization and correspondence each charge one
//! action per row in the five tables, without walking image bytes or text. The enclosing retained
//! lane admits the row owner and its five capacities; payload ownership remains shared unchanged.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::appearance::{
    AlphaMode, MaterialFactors, SupplementalPbrFactors, TextureRole, TextureStrength,
};
use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::appearance::build::AppearanceStoragePlan;
use super::super::appearance::{AppearanceRows, AppearanceStorage, StoredBinding, StoredMaterial};
use super::super::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, finish,
};
use super::super::validation::TypedStorageValidationControl;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum AppearanceSpacePlanError<E> {
    Control(E),
    NumericMaterial {
        material: usize,
        source: NumericConversionError,
    },
    NumericDisplacement {
        binding: usize,
        source: NumericConversionError,
    },
    Plan(StorePlanError),
    Invariant,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AppearanceSpacePlan<'a> {
    source: &'a AppearanceStorage,
    conversion: SpaceConversion,
    shape: AppearanceStoragePlan,
    work: u64,
    changes_storage: bool,
}

impl<'a> AppearanceSpacePlan<'a> {
    /// The source has passed central validation. Numeric preflight takes one action per binding
    /// and material, including unchanged rows, and continues after discovering changed bits.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a AppearanceStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, AppearanceSpacePlanError<C::Error>> {
        let shape =
            AppearanceStoragePlan::for_storage(source.rows.as_ref(), source.payloads.as_ref())
                .map_err(AppearanceSpacePlanError::Plan)?;
        let work = shape.rows_work().map_err(AppearanceSpacePlanError::Plan)?;
        let changes_storage = preflight_rows(source.rows.as_ref(), conversion, control)?;
        Ok(Self {
            source,
            conversion,
            shape,
            work,
            changes_storage,
        })
    }

    pub(in crate::canonical_scene::store) const fn changes_storage(self) -> bool {
        self.changes_storage
    }

    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 5], StorePlanError> {
        self.shape.rows_capacities()
    }

    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        0
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }

    /// Local buffers drop on failure or unwind before returning to the enclosing retained lane.
    /// The result contains rows only; the source's immutable payload owner must be shared.
    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> BuildResult<AppearanceRows, C::Error> {
        let capacities = self
            .capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        materialize_rows(
            self.source.rows.as_ref(),
            self.conversion,
            capacities,
            control,
        )
    }

    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &AppearanceStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        // Equal bytes are insufficient: sharing this immutable owner is part of the recipe.
        // Owner identity also proves the unchanged contents without an input-sized payload walk.
        if !std::ptr::eq(self.source.payloads.as_ref(), actual.payloads.as_ref()) {
            return Ok(false);
        }
        validate_rows_correspondence(
            self.source.rows.as_ref(),
            actual.rows.as_ref(),
            self.conversion,
            control,
        )
    }
}

/// Called within the profile row action after complete numeric preflight. Each row is fixed-size;
/// selection needs neither an additional traversal nor a retained cache.
pub(in crate::canonical_scene::store) fn material_changes(
    source: &AppearanceRows,
    index: usize,
    conversion: SpaceConversion,
) -> bool {
    source.materials.get(index).is_some_and(|&source| {
        convert_material(source, conversion)
            .is_ok_and(|converted| !same_material(source, converted))
    })
}

pub(in crate::canonical_scene::store) fn binding_changes(
    source: &AppearanceRows,
    index: usize,
    conversion: SpaceConversion,
) -> bool {
    source.bindings.get(index).is_some_and(|&source| {
        convert_binding(source, conversion).is_ok_and(|converted| !same_binding(source, converted))
    })
}

fn preflight_rows<C: TypedStorageBuildControl>(
    source: &AppearanceRows,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, AppearanceSpacePlanError<C::Error>> {
    let mut changes_storage = false;
    for (binding, &value) in source.bindings.iter().enumerate() {
        let changed = control
            .run_unit(|| {
                convert_binding(value, conversion).map(|converted| !same_binding(value, converted))
            })
            .map_err(AppearanceSpacePlanError::Control)?
            .map_err(|error| match error {
                AppearanceRowConversionError::Numeric(source) => {
                    AppearanceSpacePlanError::NumericDisplacement { binding, source }
                }
                AppearanceRowConversionError::Invariant => AppearanceSpacePlanError::Invariant,
            })?;
        changes_storage |= changed;
    }
    for (material, &value) in source.materials.iter().enumerate() {
        let changed = control
            .run_unit(|| {
                convert_material(value, conversion)
                    .map(|converted| !same_material(value, converted))
            })
            .map_err(AppearanceSpacePlanError::Control)?
            .map_err(|error| match error {
                AppearanceRowConversionError::Numeric(source) => {
                    AppearanceSpacePlanError::NumericMaterial { material, source }
                }
                AppearanceRowConversionError::Invariant => AppearanceSpacePlanError::Invariant,
            })?;
        changes_storage |= changed;
    }
    Ok(changes_storage)
}

fn materialize_rows<C: TypedStorageBuildControl>(
    source: &AppearanceRows,
    conversion: SpaceConversion,
    capacities: [CapacityPlan; 5],
    control: &mut C,
) -> BuildResult<AppearanceRows, C::Error> {
    let mut output = AppearanceRows {
        images: allocate(capacities[0])?,
        samplers: allocate(capacities[1])?,
        textures: allocate(capacities[2])?,
        bindings: allocate(capacities[3])?,
        materials: allocate(capacities[4])?,
    };
    append(&mut output.images, capacities[0], &source.images, control)?;
    append(
        &mut output.samplers,
        capacities[1],
        &source.samplers,
        control,
    )?;
    append(
        &mut output.textures,
        capacities[2],
        &source.textures,
        control,
    )?;
    for &source in &source.bindings {
        if output.bindings.len() >= capacities[3].desired_capacity()
            || output.bindings.len() == output.bindings.capacity()
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        control
            .run_unit(|| {
                let converted = convert_binding(source, conversion)
                    .map_err(|_| TypedStorageBuildError::Invariant)?;
                output.bindings.push(converted);
                Ok(())
            })
            .map_err(TypedStorageBuildError::Control)??;
    }
    for &source in &source.materials {
        if output.materials.len() >= capacities[4].desired_capacity()
            || output.materials.len() == output.materials.capacity()
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        control
            .run_unit(|| {
                let converted = convert_material(source, conversion)
                    .map_err(|_| TypedStorageBuildError::Invariant)?;
                output.materials.push(converted);
                Ok(())
            })
            .map_err(TypedStorageBuildError::Control)??;
    }
    finish(&output.images, capacities[0])?;
    finish(&output.samplers, capacities[1])?;
    finish(&output.textures, capacities[2])?;
    finish(&output.bindings, capacities[3])?;
    finish(&output.materials, capacities[4])?;
    Ok(output)
}

fn validate_rows_correspondence<C: TypedStorageValidationControl>(
    source: &AppearanceRows,
    actual: &AppearanceRows,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, C::Error> {
    if source.images.len() != actual.images.len()
        || source.samplers.len() != actual.samplers.len()
        || source.textures.len() != actual.textures.len()
        || source.bindings.len() != actual.bindings.len()
        || source.materials.len() != actual.materials.len()
    {
        return Ok(false);
    }
    for (source, actual) in source.images.iter().zip(&actual.images) {
        if control.run_unit(|| {
            source.payload == actual.payload && source.media_type == actual.media_type
        })? != Some(true)
        {
            return Ok(false);
        }
    }
    for (source, actual) in source.samplers.iter().zip(&actual.samplers) {
        if control.run_unit(|| source == actual)? != Some(true) {
            return Ok(false);
        }
    }
    for (source, actual) in source.textures.iter().zip(&actual.textures) {
        if control.run_unit(|| source.image == actual.image && source.sampler == actual.sampler)?
            != Some(true)
        {
            return Ok(false);
        }
    }
    for (&source, &actual) in source.bindings.iter().zip(&actual.bindings) {
        if control.run_unit(|| {
            convert_binding(source, conversion).is_ok_and(|expected| same_binding(expected, actual))
        })? != Some(true)
        {
            return Ok(false);
        }
    }
    for (&source, &actual) in source.materials.iter().zip(&actual.materials) {
        if control.run_unit(|| {
            convert_material(source, conversion)
                .is_ok_and(|expected| same_material(expected, actual))
        })? != Some(true)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug)]
enum AppearanceRowConversionError {
    Numeric(NumericConversionError),
    Invariant,
}

fn convert_binding(
    source: StoredBinding,
    conversion: SpaceConversion,
) -> Result<StoredBinding, AppearanceRowConversionError> {
    if source.role != TextureRole::Displacement {
        return Ok(source);
    }
    let strength = conversion
        .length(source.strength.get())
        .map_err(AppearanceRowConversionError::Numeric)?;
    let strength =
        TextureStrength::try_new(strength).map_err(|_| AppearanceRowConversionError::Invariant)?;
    Ok(StoredBinding { strength, ..source })
}

fn convert_material(
    source: StoredMaterial,
    conversion: SpaceConversion,
) -> Result<StoredMaterial, AppearanceRowConversionError> {
    let factors = source.factors;
    let supplemental = factors.supplemental_pbr();
    let thickness = conversion
        .length(supplemental.thickness())
        .map_err(AppearanceRowConversionError::Numeric)?;
    let supplemental = SupplementalPbrFactors::try_new(
        supplemental.clearcoat(),
        supplemental.clearcoat_roughness(),
        supplemental.transmission(),
        thickness,
        supplemental.sheen_color(),
        supplemental.sheen_roughness(),
    )
    .map_err(|_| AppearanceRowConversionError::Invariant)?;
    let factors = MaterialFactors::try_new_with_supplemental_pbr(
        factors.emissive(),
        factors.pbr_metallic_roughness(),
        factors.pbr_specular_glossiness(),
        factors.legacy(),
        supplemental,
    )
    .map_err(|_| AppearanceRowConversionError::Invariant)?;
    Ok(StoredMaterial { factors, ..source })
}

fn same_binding(source: StoredBinding, actual: StoredBinding) -> bool {
    let source_transform = source.transform;
    let actual_transform = actual.transform;
    source.texture == actual.texture
        && source.role == actual.role
        && source.uv_set == actual.uv_set
        && source.channels == actual.channels
        && source.color == actual.color
        && source_transform.offset().x.to_bits() == actual_transform.offset().x.to_bits()
        && source_transform.offset().y.to_bits() == actual_transform.offset().y.to_bits()
        && source_transform.rotation_radians().to_bits()
            == actual_transform.rotation_radians().to_bits()
        && source_transform.scale().x.to_bits() == actual_transform.scale().x.to_bits()
        && source_transform.scale().y.to_bits() == actual_transform.scale().y.to_bits()
        && source.strength.get().to_bits() == actual.strength.get().to_bits()
        && source.blend == actual.blend
        && source.source_key == actual.source_key
}

fn same_material(source: StoredMaterial, actual: StoredMaterial) -> bool {
    let same_alpha = match (source.alpha, actual.alpha) {
        (AlphaMode::Opaque, AlphaMode::Opaque) | (AlphaMode::Blend, AlphaMode::Blend) => true,
        (AlphaMode::Mask(source), AlphaMode::Mask(actual)) => {
            source.get().to_bits() == actual.get().to_bits()
        }
        _ => false,
    };
    source.bindings == actual.bindings
        && source.shading == actual.shading
        && same_alpha
        && source.double_sided == actual.double_sided
        && same_factors(source.factors, actual.factors)
        && source.name == actual.name
}

fn same_factors(source: MaterialFactors, actual: MaterialFactors) -> bool {
    // These bounded inline groups have distinct optional presence and exact authored f32 bits.
    // Keep the comparisons tied to the concrete values rather than a separate scalar schema.
    let metallic_roughness = |pbr: crate::appearance::PbrMetallicRoughnessFactors| {
        let color = pbr.base_color();
        [
            color.r.to_bits(),
            color.g.to_bits(),
            color.b.to_bits(),
            color.a.to_bits(),
            pbr.metallic().to_bits(),
            pbr.roughness().to_bits(),
        ]
    };
    let specular_glossiness = |pbr: crate::appearance::PbrSpecularGlossinessFactors| {
        let color = pbr.diffuse();
        let specular = pbr.specular();
        [
            color.r.to_bits(),
            color.g.to_bits(),
            color.b.to_bits(),
            color.a.to_bits(),
            specular.x.to_bits(),
            specular.y.to_bits(),
            specular.z.to_bits(),
            pbr.glossiness().to_bits(),
        ]
    };
    let legacy = |legacy: crate::appearance::LegacyMaterialFactors| {
        let ambient = legacy.ambient();
        let diffuse = legacy.diffuse();
        let specular = legacy.specular();
        let emissive = legacy.emissive();
        [
            ambient.x.to_bits(),
            ambient.y.to_bits(),
            ambient.z.to_bits(),
            diffuse.x.to_bits(),
            diffuse.y.to_bits(),
            diffuse.z.to_bits(),
            specular.x.to_bits(),
            specular.y.to_bits(),
            specular.z.to_bits(),
            emissive.x.to_bits(),
            emissive.y.to_bits(),
            emissive.z.to_bits(),
            legacy.shininess().to_bits(),
            legacy.opacity().to_bits(),
        ]
    };
    let supplemental = |pbr: SupplementalPbrFactors| {
        let sheen = pbr.sheen_color();
        [
            pbr.clearcoat().to_bits(),
            pbr.clearcoat_roughness().to_bits(),
            pbr.transmission().to_bits(),
            pbr.thickness().to_bits(),
            sheen.x.to_bits(),
            sheen.y.to_bits(),
            sheen.z.to_bits(),
            pbr.sheen_roughness().to_bits(),
        ]
    };
    source.emissive().x.to_bits() == actual.emissive().x.to_bits()
        && source.emissive().y.to_bits() == actual.emissive().y.to_bits()
        && source.emissive().z.to_bits() == actual.emissive().z.to_bits()
        && source.pbr_metallic_roughness().map(metallic_roughness)
            == actual.pbr_metallic_roughness().map(metallic_roughness)
        && source.pbr_specular_glossiness().map(specular_glossiness)
            == actual.pbr_specular_glossiness().map(specular_glossiness)
        && source.legacy().map(legacy) == actual.legacy().map(legacy)
        && supplemental(source.supplemental_pbr()) == supplemental(actual.supplemental_pbr())
}
