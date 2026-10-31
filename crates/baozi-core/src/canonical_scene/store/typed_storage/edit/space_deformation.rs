//! Source-bound conversion of the complete typed deformation owner.
//!
//! The source has already passed central validation. The caller admits the owner and all seventeen
//! capacities through the existing retained-storage lane before materialization. Local buffers drop
//! on failure or unwind before control returns to that lane; this module owns no independent lease.
//!
//! The complete Scene plan includes this domain in numeric preflight and correspondence.
//! Unchanged converted bits share the original owner. Central deformation counts,
//! validation work, and scratch shape stay unchanged.
//!
//! The profile selectors below revisit every bind/tuple through build control, without a cache.
//! One complete selection pass over validated owners costs skins + inverse binds + morph deltas +
//! morph scalars / 3 actions. Charge each lineage/state pass separately; the caller skips this domain
//! for winding/declaration-only edits. Numeric admission remains owned by `DeformationSpacePlan`.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::deform::{MorphDeltaSemantic, SkinMatrixCorrection, SkinMatrixSpaces};
use crate::math::Mat4;
use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::TableRange;
use super::super::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, finish, push,
};
use super::super::deformation::build::DeformationStoragePlan;
use super::super::deformation::{DeformationStorage, StoredMorphDeltaPayload, StoredSkin};
use super::super::validation::TypedStorageValidationControl;

#[cfg(test)]
mod tests;

/// Direct storage coordinates: finding an owning skin/joint is deliberately left to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum DeformationSpaceNumericLocation {
    SkinCorrection {
        skin: usize,
    },
    InverseBind {
        matrix: usize,
    },
    /// `tuple` is the explicit dense/sparse tuple ordinal within this delta, not a sparse vertex ID.
    MorphDelta {
        delta: usize,
        tuple: usize,
    },
}

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum DeformationSpacePlanError<E> {
    Control(E),
    Numeric {
        location: DeformationSpaceNumericLocation,
        source: NumericConversionError,
    },
    Plan(StorePlanError),
    Invariant,
}

/// Only a source borrow, arithmetic, initialized shape, and aggregate work/change facts persist.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct DeformationSpacePlan<'a> {
    source: &'a DeformationStorage,
    conversion: SpaceConversion,
    shape: DeformationStoragePlan,
    work: u64,
    changes_storage: bool,
}

impl<'a> DeformationSpacePlan<'a> {
    /// Numeric preflight takes one action per skin row, inverse-bind Mat4, morph-delta row, and
    /// explicit morph xyz tuple. It never stops checking later numeric data after finding a change.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a DeformationStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, DeformationSpacePlanError<C::Error>> {
        let shape =
            DeformationStoragePlan::for_storage(source).map_err(DeformationSpacePlanError::Plan)?;
        let work = shape.work().map_err(DeformationSpacePlanError::Plan)?;
        let mut changes_storage = false;
        for (index, &skin) in source.skins.iter().enumerate() {
            let changed = control
                .run_unit(|| {
                    convert_skin(conversion, skin).map(|converted| !same_skin(skin, converted))
                })
                .map_err(DeformationSpacePlanError::Control)?
                .map_err(|error| match error {
                    SkinConversionError::Numeric(source) => DeformationSpacePlanError::Numeric {
                        location: DeformationSpaceNumericLocation::SkinCorrection { skin: index },
                        source,
                    },
                    SkinConversionError::Invariant => DeformationSpacePlanError::Invariant,
                })?;
            changes_storage |= changed;
        }
        for (index, &matrix) in source.inverse_bind_matrices.iter().enumerate() {
            let changed = control
                .run_unit(|| {
                    conversion
                        .matrix(matrix)
                        .map(|converted| !same_matrix(matrix, converted))
                })
                .map_err(DeformationSpacePlanError::Control)?
                .map_err(|source| DeformationSpacePlanError::Numeric {
                    location: DeformationSpaceNumericLocation::InverseBind { matrix: index },
                    source,
                })?;
            changes_storage |= changed;
        }
        let mut next_value = 0_usize;
        for (index, delta) in source.morph_deltas.iter().enumerate() {
            let values = control
                .run_unit(|| -> Result<_, DeformationSpacePlanError<C::Error>> {
                    let range = morph_values(delta.payload);
                    if range.start() != next_value || !range.len().is_multiple_of(3) {
                        return Err(DeformationSpacePlanError::Invariant);
                    }
                    let values = range
                        .get(&source.morph_values)
                        .ok_or(DeformationSpacePlanError::Invariant)?;
                    if let StoredMorphDeltaPayload::Sparse { indices, .. } = delta.payload
                        && (indices.get(&source.morph_sparse_indices).is_none()
                            || indices.len().checked_mul(3) != Some(values.len()))
                    {
                        return Err(DeformationSpacePlanError::Invariant);
                    }
                    next_value = range
                        .start()
                        .checked_add(range.len())
                        .ok_or(DeformationSpacePlanError::Invariant)?;
                    Ok(values)
                })
                .map_err(DeformationSpacePlanError::Control)??;
            for (tuple_index, tuple) in values.as_chunks::<3>().0.iter().enumerate() {
                let changed = control
                    .run_unit(|| {
                        convert_morph_tuple(conversion, delta.semantic, tuple)
                            .map(|converted| !same_morph_tuple(tuple, converted))
                    })
                    .map_err(DeformationSpacePlanError::Control)?
                    .map_err(|source| DeformationSpacePlanError::Numeric {
                        location: DeformationSpaceNumericLocation::MorphDelta {
                            delta: index,
                            tuple: tuple_index,
                        },
                        source,
                    })?;
                changes_storage |= changed;
            }
        }
        if next_value != source.morph_values.len() {
            return Err(DeformationSpacePlanError::Invariant);
        }
        Ok(Self {
            source,
            conversion,
            shape,
            work,
            changes_storage,
        })
    }

    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 17], StorePlanError> {
        self.shape.capacities()
    }

    /// Exactly sum(rows) + 2*joints + names + offsets + 3*influences + weights + values + indices.
    /// Corrections are inline skin-row work; a bind matrix is one element, a morph scalar is one.
    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }

    /// Correspondence uses the same per-row/element actions as materialization, without allocation.
    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.shape.name_bytes()
    }

    /// The caller can share this exact owner when every converted bit is unchanged.
    pub(in crate::canonical_scene::store) const fn changes_storage(self) -> bool {
        self.changes_storage
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> BuildResult<DeformationStorage, C::Error> {
        let p = self
            .capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        // This is the existing production builder's seventeen-capacity order. Every allocation is
        // owned by a local Vec or this aggregate, including errors during aggregate initialization.
        let mut output = DeformationStorage {
            skins: allocate(p[0])?,
            logical_morphs: allocate(p[1])?,
            morph_definitions: allocate(p[2])?,
            primitives: allocate(p[3])?,
            morph_targets: allocate(p[4])?,
            morph_deltas: allocate(p[5])?,
            mesh_instances: allocate(p[6])?,
            joints: allocate(p[7])?,
            inverse_bind_matrices: allocate(p[8])?,
            names: allocate(p[9])?,
            influence_vertex_offsets: allocate(p[10])?,
            influence_joints: allocate(p[11])?,
            influence_weights: allocate(p[12])?,
            influence_source_orders: allocate(p[13])?,
            morph_weights: allocate(p[14])?,
            morph_values: allocate(p[15])?,
            morph_sparse_indices: allocate(p[16])?,
        };
        macro_rules! copy {
            ($field:ident, $index:literal) => {
                append(&mut output.$field, p[$index], &self.source.$field, control)?;
                finish(&output.$field, p[$index])?;
            };
        }
        for &skin in &self.source.skins {
            push_converted(&mut output.skins, p[0], control, || {
                convert_skin(self.conversion, skin).map_err(|_| TypedStorageBuildError::Invariant)
            })?;
        }
        finish(&output.skins, p[0])?;
        copy!(logical_morphs, 1);
        copy!(morph_definitions, 2);
        copy!(primitives, 3);
        copy!(morph_targets, 4);
        for &delta in &self.source.morph_deltas {
            push(&mut output.morph_deltas, p[5], delta, control)?;
            let values = morph_values(delta.payload)
                .get(&self.source.morph_values)
                .ok_or(TypedStorageBuildError::Invariant)?;
            for tuple in values.as_chunks::<3>().0 {
                for component in 0..3 {
                    push_converted(&mut output.morph_values, p[15], control, || {
                        convert_morph_component(self.conversion, delta.semantic, tuple, component)
                            .map_err(|_| TypedStorageBuildError::Invariant)
                    })?;
                }
            }
        }
        finish(&output.morph_deltas, p[5])?;
        finish(&output.morph_values, p[15])?;
        copy!(mesh_instances, 6);
        copy!(joints, 7);
        for &matrix in &self.source.inverse_bind_matrices {
            push_converted(&mut output.inverse_bind_matrices, p[8], control, || {
                self.conversion
                    .matrix(matrix)
                    .map_err(|_| TypedStorageBuildError::Invariant)
            })?;
        }
        finish(&output.inverse_bind_matrices, p[8])?;
        for &byte in &self.source.names {
            if output.names.len() >= p[9].desired_capacity()
                || output.names.len() == output.names.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_string_unit(|| output.names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        finish(&output.names, p[9])?;
        copy!(influence_vertex_offsets, 10);
        copy!(influence_joints, 11);
        copy!(influence_weights, 12);
        copy!(influence_source_orders, 13);
        copy!(morph_weights, 14);
        copy!(morph_sparse_indices, 16);
        Ok(output)
    }

    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &DeformationStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if DeformationStoragePlan::for_storage(actual).ok() != Some(self.shape) {
            return Ok(false);
        }
        macro_rules! compare {
            ($field:ident, $same:expr) => {
                if !compare_elements(&self.source.$field, &actual.$field, control, $same)? {
                    return Ok(false);
                }
            };
        }
        compare!(skins, |source, actual| {
            convert_skin(self.conversion, *source)
                .is_ok_and(|expected| same_skin(expected, *actual))
        });
        compare!(logical_morphs, |source, actual| {
            source.definitions == actual.definitions
                && source.default_weights == actual.default_weights
        });
        compare!(morph_definitions, |source, actual| {
            source.logical_mesh == actual.logical_mesh
                && source.target_ordinal == actual.target_ordinal
                && source.stable_identity == actual.stable_identity
                && source.name == actual.name
        });
        compare!(primitives, |source, actual| {
            source.morph_targets == actual.morph_targets
                && match (source.influences, actual.influences) {
                    (None, None) => true,
                    (Some(source), Some(actual)) => {
                        source.vertex_offsets == actual.vertex_offsets
                            && source.influences == actual.influences
                    }
                    _ => false,
                }
        });
        compare!(morph_targets, |source, actual| {
            source.primitive == actual.primitive
                && source.target_ordinal == actual.target_ordinal
                && source.deltas == actual.deltas
        });
        for (source, actual_delta) in self.source.morph_deltas.iter().zip(&actual.morph_deltas) {
            if control.run_unit(|| {
                source.target == actual_delta.target
                    && source.semantic == actual_delta.semantic
                    && same_morph_payload(source.payload, actual_delta.payload)
            })? != Some(true)
            {
                return Ok(false);
            }
            // The descriptor action above checked identical ranges. Preflight proved that all
            // source value ranges are compact and complete, including the empty streams.
            let range = morph_values(source.payload);
            let Some(source_values) = range.get(&self.source.morph_values) else {
                return Ok(false);
            };
            let Some(actual_values) = range.get(&actual.morph_values) else {
                return Ok(false);
            };
            for (source_tuple, actual_tuple) in source_values
                .as_chunks::<3>()
                .0
                .iter()
                .zip(actual_values.as_chunks::<3>().0)
            {
                for (component, &actual) in actual_tuple.iter().enumerate() {
                    if control.run_unit(|| {
                        convert_morph_component(
                            self.conversion,
                            source.semantic,
                            source_tuple,
                            component,
                        )
                        .is_ok_and(|expected| expected.to_bits() == actual.to_bits())
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        compare!(mesh_instances, |source, actual| {
            source.skin == actual.skin && source.morph_weights == actual.morph_weights
        });
        compare!(joints, |source, actual| source == actual);
        compare!(inverse_bind_matrices, |source, actual| {
            self.conversion
                .matrix(*source)
                .is_ok_and(|expected| same_matrix(expected, *actual))
        });
        compare!(names, |source, actual| source == actual);
        compare!(influence_vertex_offsets, |source, actual| source == actual);
        compare!(influence_joints, |source, actual| source == actual);
        compare!(influence_weights, |source, actual| {
            source.to_bits() == actual.to_bits()
        });
        compare!(influence_source_orders, |source, actual| source == actual);
        compare!(morph_weights, |source, actual| {
            source.to_bits() == actual.to_bits()
        });
        compare!(morph_sparse_indices, |source, actual| source == actual);
        Ok(true)
    }
}

/// Selects one skin whose correction or inverse-bind bits change under the admitted conversion.
///
/// Exactly one descriptor/correction action plus one action per matrix in this skin's bind range,
/// even after a change is found. A missing index or invalid bind range returns false after the
/// descriptor action. Numeric conversion failure also returns false, after visiting all matrices;
/// call the plan's numeric preflight first to reject such a conversion with its precise location.
pub(in crate::canonical_scene::store) fn skin_changes<C: TypedStorageBuildControl>(
    source: &DeformationStorage,
    skin: usize,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, C::Error> {
    let Some((matrices, correction_changed)) = control.run_unit(|| {
        let skin = *source.skins.get(skin)?;
        let matrices = skin
            .inverse_bind_matrices
            .get(&source.inverse_bind_matrices)?;
        let correction_changed = convert_skin(conversion, skin)
            .ok()
            .map(|converted| !same_skin(skin, converted));
        Some((matrices, correction_changed))
    })?
    else {
        return Ok(false);
    };
    let mut valid = correction_changed.is_some();
    let mut changed = correction_changed.unwrap_or(false);
    for &matrix in matrices {
        let matrix_changed = control.run_unit(|| {
            conversion
                .matrix(matrix)
                .ok()
                .map(|converted| !same_matrix(matrix, converted))
        })?;
        valid &= matrix_changed.is_some();
        changed |= matrix_changed.unwrap_or(false);
    }
    Ok(valid && changed)
}

/// Selects one morph-delta owner without expanding sparse omissions or inspecting other deltas.
///
/// Exactly one descriptor action plus one action per explicit xyz tuple. All tuples are visited,
/// even after a change or numeric failure. Missing indices, invalid value/sparse-index ranges, and
/// invalid tuple shape return false after the descriptor action. Numeric failures return false;
/// the plan's preflight must reject them before profile selection is used for publication.
pub(in crate::canonical_scene::store) fn morph_delta_changes<C: TypedStorageBuildControl>(
    source: &DeformationStorage,
    delta: usize,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, C::Error> {
    let Some((semantic, values)) = control.run_unit(|| {
        let delta = source.morph_deltas.get(delta)?;
        let values = morph_values(delta.payload).get(&source.morph_values)?;
        if !values.len().is_multiple_of(3) {
            return None;
        }
        if let StoredMorphDeltaPayload::Sparse { indices, .. } = delta.payload {
            indices.get(&source.morph_sparse_indices)?;
            if indices.len().checked_mul(3) != Some(values.len()) {
                return None;
            }
        }
        Some((delta.semantic, values))
    })?
    else {
        return Ok(false);
    };
    let mut valid = true;
    let mut changed = false;
    for tuple in values.as_chunks::<3>().0 {
        let tuple_changed = control.run_unit(|| {
            convert_morph_tuple(conversion, semantic, tuple)
                .ok()
                .map(|converted| !same_morph_tuple(tuple, converted))
        })?;
        valid &= tuple_changed.is_some();
        changed |= tuple_changed.unwrap_or(false);
    }
    Ok(valid && changed)
}

enum SkinConversionError {
    Numeric(NumericConversionError),
    Invariant,
}

fn convert_skin(
    conversion: SpaceConversion,
    source: StoredSkin,
) -> Result<StoredSkin, SkinConversionError> {
    let Some(correction) = source.matrix_spaces.correction() else {
        return Ok(source);
    };
    let matrix = conversion
        .matrix(correction.matrix())
        .map_err(SkinConversionError::Numeric)?;
    let correction = SkinMatrixCorrection::new(correction.from(), correction.to(), matrix)
        .map_err(|_| SkinConversionError::Invariant)?;
    Ok(StoredSkin {
        matrix_spaces: source.matrix_spaces.with_correction(correction),
        ..source
    })
}

fn morph_values(payload: StoredMorphDeltaPayload) -> TableRange<f32> {
    match payload {
        StoredMorphDeltaPayload::Dense(values) | StoredMorphDeltaPayload::Sparse { values, .. } => {
            values
        }
    }
}

fn convert_morph_tuple(
    conversion: SpaceConversion,
    semantic: MorphDeltaSemantic,
    tuple: &[f32],
) -> Result<[f32; 3], NumericConversionError> {
    let value = [tuple[0], tuple[1], tuple[2]];
    match semantic {
        MorphDeltaSemantic::Position => conversion.point(value),
        MorphDeltaSemantic::Normal | MorphDeltaSemantic::Tangent => conversion.direction(value),
    }
}

/// Repeating three bounded component conversions avoids a tuple cache while retaining the existing
/// scalar work formula. Both the arithmetic and each scalar write/check run inside their action.
fn convert_morph_component(
    conversion: SpaceConversion,
    semantic: MorphDeltaSemantic,
    tuple: &[f32],
    component: usize,
) -> Result<f32, NumericConversionError> {
    convert_morph_tuple(conversion, semantic, tuple).map(|value| value[component])
}

fn push_converted<T, C: TypedStorageBuildControl>(
    values: &mut Vec<T>,
    capacity: CapacityPlan,
    control: &mut C,
    convert: impl FnOnce() -> BuildResult<T, C::Error>,
) -> BuildResult<(), C::Error> {
    if values.len() >= capacity.desired_capacity() || values.len() == values.capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    control
        .run_unit(|| {
            values.push(convert()?);
            Ok(())
        })
        .map_err(TypedStorageBuildError::Control)?
}

fn compare_elements<T, C: TypedStorageValidationControl>(
    source: &[T],
    actual: &[T],
    control: &mut C,
    mut same: impl FnMut(&T, &T) -> bool,
) -> Result<bool, C::Error> {
    if source.len() != actual.len() {
        return Ok(false);
    }
    for (source, actual) in source.iter().zip(actual) {
        if control.run_unit(|| same(source, actual))? != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn same_matrix(source: Mat4, actual: Mat4) -> bool {
    source
        .cols
        .into_iter()
        .flatten()
        .zip(actual.cols.into_iter().flatten())
        .all(|(source, actual)| source.to_bits() == actual.to_bits())
}

fn same_morph_tuple(source: &[f32], actual: [f32; 3]) -> bool {
    source
        .iter()
        .zip(actual)
        .all(|(&source, actual)| source.to_bits() == actual.to_bits())
}

fn same_matrix_spaces(source: SkinMatrixSpaces, actual: SkinMatrixSpaces) -> bool {
    source.input_position() == actual.input_position()
        && source.inverse_bind_from() == actual.inverse_bind_from()
        && source.inverse_bind_to() == actual.inverse_bind_to()
        && source.current_joint_from() == actual.current_joint_from()
        && source.current_joint_to() == actual.current_joint_to()
        && source.output_position() == actual.output_position()
        && match (source.correction(), actual.correction()) {
            (None, None) => true,
            (Some(source), Some(actual)) => {
                source.from() == actual.from()
                    && source.to() == actual.to()
                    && same_matrix(source.matrix(), actual.matrix())
            }
            _ => false,
        }
}

fn same_skin(source: StoredSkin, actual: StoredSkin) -> bool {
    source.name == actual.name
        && source.joints == actual.joints
        && source.inverse_bind_matrices == actual.inverse_bind_matrices
        && source.skeleton_root == actual.skeleton_root
        && same_matrix_spaces(source.matrix_spaces, actual.matrix_spaces)
}

fn same_morph_payload(source: StoredMorphDeltaPayload, actual: StoredMorphDeltaPayload) -> bool {
    match (source, actual) {
        (StoredMorphDeltaPayload::Dense(source), StoredMorphDeltaPayload::Dense(actual)) => {
            source == actual
        }
        (
            StoredMorphDeltaPayload::Sparse {
                logical_count: source_count,
                indices: source_indices,
                values: source_values,
            },
            StoredMorphDeltaPayload::Sparse {
                logical_count: actual_count,
                indices: actual_indices,
                values: actual_values,
            },
        ) => {
            source_count == actual_count
                && source_indices == actual_indices
                && source_values == actual_values
        }
        _ => false,
    }
}
