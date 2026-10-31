//! Source-bound replacement of one TriangleList primitive's authored normal field.
//!
//! This plan borrows provisional corner normals. The caller admits all nine capacities before
//! materialization and retains publication authority. No source table or supplied normal is mutated.

use baozi_operation::CapacityPlan;

use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;

use super::super::StoredAttributeSemantic;
use super::super::validation::TypedStorageValidationControl;
use super::{
    EditResult, GeometryStorage, ScalarCounts, StoredAttribute, StoredAttributePayload,
    StoredLogicalMesh, StoredMeshInstance, StoredPrimitive, TableRange, TypedStorageBuildControl,
    TypedStorageBuildError, add_count, allocate, append, capacity, copy_optional, overflow, push,
    same_range, validate_dense_range,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NormalWritePolicy {
    Add,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum TangentPolicy {
    Reject,
    Remove,
}

/// The squared length of an f32-rounded unit vector can differ from one by a few ulps.
const UNIT_LENGTH_TOLERANCE: f64 = 4.0 * f32::EPSILON as f64;

/// A closed, allocation-free index relation that can outlive the borrowed geometry plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NormalAttributeMapping {
    source_attributes: u32,
    output_attributes: u32,
    /// Source coordinate of the first target attribute ranked after Normal, or the range end.
    insertion_boundary: u32,
    normal_attribute: u32,
    source_normal: Option<u32>,
    source_tangent: Option<u32>,
    source_position: u32,
    source_smoothing_group: Option<u32>,
}

impl NormalAttributeMapping {
    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_attribute_count(self) -> u32 {
        self.source_attributes
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(self) -> u32 {
        self.output_attributes
    }

    pub(in crate::canonical_scene::store) const fn normal_attribute(self) -> u32 {
        self.normal_attribute
    }

    pub(in crate::canonical_scene::store) const fn source_normal(self) -> Option<u32> {
        self.source_normal
    }

    pub(in crate::canonical_scene::store) const fn source_tangent(self) -> Option<u32> {
        self.source_tangent
    }

    pub(in crate::canonical_scene::store) const fn source_position(self) -> u32 {
        self.source_position
    }

    pub(in crate::canonical_scene::store) const fn source_smoothing_group(self) -> Option<u32> {
        self.source_smoothing_group
    }

    /// Global attribute mapping, including a replaced Normal and excluding a removed Tangent.
    pub(in crate::canonical_scene::store) fn output_attribute(self, source: u32) -> Option<u32> {
        if source >= self.source_attributes || self.source_tangent == Some(source) {
            return None;
        }
        source
            .checked_sub(u32::from(
                self.source_tangent.is_some_and(|index| index < source),
            ))?
            .checked_add(u32::from(
                self.source_normal.is_none() && source >= self.insertion_boundary,
            ))
    }

    /// A newly added Normal has no source row; a replacement maps to the old Normal row.
    pub(in crate::canonical_scene::store) fn source_attribute(self, output: u32) -> Option<u32> {
        if output >= self.output_attributes {
            return None;
        }
        if output == self.normal_attribute {
            return self.source_normal;
        }
        let without_addition = output.checked_sub(u32::from(
            self.source_normal.is_none() && output > self.normal_attribute,
        ))?;
        without_addition.checked_add(u32::from(
            self.source_tangent
                .is_some_and(|index| index <= without_addition),
        ))
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryNormalsPlan<'a> {
    source: &'a GeometryStorage,
    normals: &'a [[f32; 3]],
    target: usize,
    capacities: [CapacityPlan; 9],
    work: u64,
    validation_work: u64,
    mapping: NormalAttributeMapping,
}

impl<'a> GeometryNormalsPlan<'a> {
    /// Source tables have already passed central validation. Descriptor extents and supplied
    /// normals are checked here without allocating; all input-sized scans are action scoped.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        normals: &'a [[f32; 3]],
        write: NormalWritePolicy,
        tangents: TangentPolicy,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let primitive = source
            .primitives
            .get(target)
            .ok_or(TypedStorageBuildError::Invariant)?;
        let target_attributes = control
            .run_unit(|| {
                if primitive.topology != TopologyKind::TriangleList
                    || primitive.corner_count == 0
                    || !primitive.corner_count.is_multiple_of(3)
                    || primitive.face_count != primitive.corner_count / 3
                    || primitive.vertex_count == 0
                    || primitive.polygon_boundaries.is_some()
                    || normals.len() != primitive.corner_count as usize
                    || match primitive.indices {
                        Some(range) => {
                            range.len() != primitive.corner_count as usize
                                || range.checked(source.u32_values.len()).is_none()
                        }
                        None => primitive.vertex_count != primitive.corner_count,
                    }
                {
                    return Err(TypedStorageBuildError::Invariant);
                }
                primitive
                    .attributes
                    .checked(source.attributes.len())
                    .ok_or(TypedStorageBuildError::Invariant)
            })
            .map_err(TypedStorageBuildError::Control)??;
        for normal in normals {
            let valid = control
                .run_unit(|| {
                    let [x, y, z] = normal.map(f64::from);
                    x.is_finite()
                        && y.is_finite()
                        && z.is_finite()
                        && (x * x + y * y + z * z - 1.0).abs() <= UNIT_LENGTH_TOLERANCE
                })
                .map_err(TypedStorageBuildError::Control)?;
            if !valid {
                return Err(TypedStorageBuildError::Invariant);
            }
        }

        let source_attribute_count =
            u32::try_from(source.attributes.len()).map_err(|_| overflow())?;
        let mut insertion_boundary =
            u32::try_from(target_attributes.end).map_err(|_| overflow())?;
        let mut source_normal = None;
        let mut source_tangent = None;
        let mut source_position = None;
        let mut source_smoothing_group = None;
        let mut scalars = ScalarCounts::default();
        let mut next_name = 0_usize;
        for (index, attribute) in source.attributes.iter().enumerate() {
            let counts = control
                .run_unit(|| {
                    let counts = attribute_scalar_counts(source, *attribute)?;
                    if let StoredAttributeSemantic::Custom(name) = attribute.semantic {
                        if name.0.start() != next_name || name.0.get(&source.names).is_none() {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        add_count(&mut next_name, name.0.len())?;
                    }
                    if target_attributes.contains(&index) {
                        // The source is canonical: Position ranks 0, Normal 1, and every other
                        // semantic ranks above Normal. Keep the first insertion point in source
                        // coordinates, including a Tangent that this operation will remove.
                        if index < insertion_boundary as usize
                            && !matches!(
                                attribute.semantic,
                                StoredAttributeSemantic::Position | StoredAttributeSemantic::Normal
                            )
                        {
                            insertion_boundary = index as u32;
                        }
                        let slot = match attribute.semantic {
                            StoredAttributeSemantic::Normal => Some(&mut source_normal),
                            StoredAttributeSemantic::Tangent => Some(&mut source_tangent),
                            StoredAttributeSemantic::Position => Some(&mut source_position),
                            StoredAttributeSemantic::SmoothingGroup => {
                                Some(&mut source_smoothing_group)
                            }
                            _ => None,
                        };
                        if let Some(slot) = slot
                            && slot.replace(index as u32).is_some()
                        {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        if matches!(
                            attribute.semantic,
                            StoredAttributeSemantic::Normal | StoredAttributeSemantic::Tangent
                        ) {
                            // Both old payloads disappear. The one new Normal is counted below.
                            return Ok(ScalarCounts::default());
                        }
                    }
                    Ok(counts)
                })
                .map_err(TypedStorageBuildError::Control)??;
            add_count(&mut scalars.f32s, counts.f32s)?;
            add_count(&mut scalars.f64s, counts.f64s)?;
            add_count(&mut scalars.u32s, counts.u32s)?;
            add_count(&mut scalars.i32s, counts.i32s)?;
        }
        let source_position = source_position.ok_or(TypedStorageBuildError::Invariant)?;
        if matches!(write, NormalWritePolicy::Add) != source_normal.is_none()
            || (source_tangent.is_some() && tangents == TangentPolicy::Reject)
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let added = u32::from(source_normal.is_none());
        let removed = u32::from(source_tangent.is_some());
        let output_attributes = source_attribute_count
            .checked_sub(removed)
            .and_then(|count| count.checked_add(added))
            .ok_or_else(overflow)?;
        let normal_origin = source_normal.unwrap_or(insertion_boundary);
        let normal_attribute = normal_origin
            .checked_sub(u32::from(
                source_tangent.is_some_and(|index| index < normal_origin),
            ))
            .ok_or_else(overflow)?;
        add_count(
            &mut scalars.f32s,
            normals.len().checked_mul(3).ok_or_else(overflow)?,
        )?;

        let mut next_attribute = 0_usize;
        for primitive in &source.primitives {
            let count = control
                .run_unit(|| {
                    if primitive.attributes.start() != next_attribute
                        || primitive
                            .attributes
                            .checked(source.attributes.len())
                            .is_none()
                    {
                        return Err(TypedStorageBuildError::Invariant);
                    }
                    add_count(&mut next_attribute, primitive.attributes.len())?;
                    [primitive.polygon_boundaries, primitive.indices]
                        .into_iter()
                        .flatten()
                        .try_fold(0_usize, |count, range| {
                            if range.checked(source.u32_values.len()).is_none() {
                                return Err(TypedStorageBuildError::Invariant);
                            }
                            count.checked_add(range.len()).ok_or_else(overflow)
                        })
                })
                .map_err(TypedStorageBuildError::Control)??;
            add_count(&mut scalars.u32s, count)?;
        }
        if next_attribute != source.attributes.len() {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut next_primitive = 0_usize;
        for mesh in &source.logical_meshes {
            control
                .run_unit(|| {
                    if mesh.primitives.start() != next_primitive
                        || mesh.primitives.checked(source.primitives.len()).is_none()
                    {
                        return Err(TypedStorageBuildError::Invariant);
                    }
                    add_count(&mut next_primitive, mesh.primitives.len())?;
                    if let Some(name) = mesh.name {
                        if name.0.start() != next_name || name.0.get(&source.names).is_none() {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        add_count(&mut next_name, name.0.len())?;
                    }
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        if next_primitive != source.primitives.len() || next_name != source.names.len() {
            return Err(TypedStorageBuildError::Invariant);
        }

        let capacities = [
            capacity::<StoredLogicalMesh, C::Error>(source.logical_meshes.len())?,
            capacity::<StoredPrimitive, C::Error>(source.primitives.len())?,
            capacity::<StoredAttribute, C::Error>(output_attributes as usize)?,
            capacity::<StoredMeshInstance, C::Error>(source.mesh_instances.len())?,
            capacity::<u8, C::Error>(source.names.len())?,
            capacity::<f32, C::Error>(scalars.f32s)?,
            capacity::<f64, C::Error>(scalars.f64s)?,
            capacity::<u32, C::Error>(scalars.u32s)?,
            capacity::<i32, C::Error>(scalars.i32s)?,
        ];
        let work = capacities.iter().try_fold(0_u64, |sum, plan| {
            sum.checked_add(u64::try_from(plan.desired_capacity()).map_err(|_| overflow())?)
                .ok_or_else(overflow)
        })?;
        let validation_work = work
            .checked_sub(u64::try_from(source.names.len()).map_err(|_| overflow())?)
            .and_then(|count| count.checked_add(u64::try_from(source.primitives.len()).ok()?))
            .ok_or_else(overflow)?;
        Ok(Self {
            source,
            normals,
            target,
            capacities,
            work,
            validation_work,
            mapping: NormalAttributeMapping {
                source_attributes: source_attribute_count,
                output_attributes,
                insertion_boundary,
                normal_attribute,
                source_normal,
                source_tangent,
                source_position,
                source_smoothing_group,
            },
        })
    }

    pub(in crate::canonical_scene::store) const fn target_row(&self) -> usize {
        self.target
    }

    pub(in crate::canonical_scene::store) const fn capacities(&self) -> [CapacityPlan; 9] {
        self.capacities
    }

    /// One action per output row/scalar and one string action per name byte; no lookup actions.
    pub(in crate::canonical_scene::store) const fn work(&self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.validation_work
    }

    /// Each descriptor, referenced name byte and scalar is compared exactly once on success.
    pub(in crate::canonical_scene::store) const fn correspondence_work(&self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(&self) -> usize {
        self.source.names.len()
    }

    pub(in crate::canonical_scene::store) const fn attribute_mapping(
        &self,
    ) -> NormalAttributeMapping {
        self.mapping
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_attribute_count(&self) -> u32 {
        self.mapping.source_attribute_count()
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(&self) -> u32 {
        self.mapping.output_attribute_count()
    }

    pub(in crate::canonical_scene::store) const fn normal_attribute(&self) -> u32 {
        self.mapping.normal_attribute()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_normal(&self) -> Option<u32> {
        self.mapping.source_normal()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_tangent(&self) -> Option<u32> {
        self.mapping.source_tangent()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_position(&self) -> u32 {
        self.mapping.source_position()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_smoothing_group(&self) -> Option<u32> {
        self.mapping.source_smoothing_group()
    }

    pub(in crate::canonical_scene::store) fn output_corner_count(&self) -> u32 {
        // try_new established the u32 cardinality and binds this immutable slice.
        self.normals.len() as u32
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn output_face_count(&self) -> u32 {
        self.output_corner_count() / 3
    }

    /// Global attribute mapping, including the replaced Normal and excluding a removed Tangent.
    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn output_attribute(&self, source: u32) -> Option<u32> {
        self.mapping.output_attribute(source)
    }

    /// A newly added Normal has no source row; a replacement maps to the old Normal row.
    pub(in crate::canonical_scene::store) fn source_attribute(&self, output: u32) -> Option<u32> {
        self.mapping.source_attribute(output)
    }

    fn primitive_attributes(
        &self,
        index: usize,
        primitive: &StoredPrimitive,
    ) -> Option<TableRange<StoredAttribute>> {
        let start = primitive
            .attributes
            .start()
            .checked_sub(usize::from(self.mapping.source_tangent.is_some_and(
                |tangent| (tangent as usize) < primitive.attributes.start(),
            )))?
            .checked_add(usize::from(
                self.mapping.source_normal.is_none()
                    && primitive.attributes.start() >= self.mapping.insertion_boundary as usize,
            ))?;
        let count = if index == self.target {
            primitive
                .attributes
                .len()
                .checked_sub(usize::from(self.mapping.source_tangent.is_some()))?
                .checked_add(usize::from(self.mapping.source_normal.is_none()))?
        } else {
            primitive.attributes.len()
        };
        Some(TableRange::new(start, count))
    }

    /// The caller must admit every capacity before invoking this method.
    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        &self,
        control: &mut C,
    ) -> EditResult<GeometryStorage, C::Error> {
        let [
            meshes,
            primitives,
            attributes,
            instances,
            names,
            f32s,
            f64s,
            u32s,
            i32s,
        ] = self.capacities;
        let mut output = GeometryStorage {
            logical_meshes: allocate(meshes)?,
            primitives: allocate(primitives)?,
            attributes: allocate(attributes)?,
            mesh_instances: allocate(instances)?,
            names: allocate(names)?,
            f32_values: allocate(f32s)?,
            f64_values: allocate(f64s)?,
            u32_values: allocate(u32s)?,
            i32_values: allocate(i32s)?,
        };
        // Normal/Tangent never own a custom name, so no name range needs to move.
        for &byte in &self.source.names {
            if output.names.len() >= names.desired_capacity()
                || output.names.len() == output.names.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_string_unit(|| output.names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        for index in 0..self.output_attribute_count() {
            // Add emits Normal at its canonical insertion point; Replace uses the original row.
            let row = if index == self.normal_attribute() {
                let start = output.f32_values.len();
                for normal in self.normals {
                    append(&mut output.f32_values, f32s, normal, control)?;
                }
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Normal,
                    domain: AttributeDomain::Corner,
                    lanes: 3,
                    element_count: self.output_corner_count(),
                    payload: StoredAttributePayload::DenseF32(TableRange::new(
                        start,
                        output.f32_values.len() - start,
                    )),
                }
            } else {
                let source = self
                    .source_attribute(index)
                    .and_then(|index| self.source.attributes.get(index as usize))
                    .ok_or(TypedStorageBuildError::Invariant)?;
                StoredAttribute {
                    payload: copy_payload(
                        source.payload,
                        self.source,
                        &mut output,
                        self.capacities,
                        control,
                    )?,
                    ..*source
                }
            };
            push(&mut output.attributes, attributes, row, control)?;
        }
        for (index, source) in self.source.primitives.iter().enumerate() {
            let row = StoredPrimitive {
                attributes: self
                    .primitive_attributes(index, source)
                    .ok_or(TypedStorageBuildError::Invariant)?,
                polygon_boundaries: copy_optional(
                    source.polygon_boundaries,
                    &self.source.u32_values,
                    &mut output.u32_values,
                    u32s,
                    control,
                )?,
                indices: copy_optional(
                    source.indices,
                    &self.source.u32_values,
                    &mut output.u32_values,
                    u32s,
                    control,
                )?,
                ..*source
            };
            push(&mut output.primitives, primitives, row, control)?;
        }
        append(
            &mut output.logical_meshes,
            meshes,
            &self.source.logical_meshes,
            control,
        )?;
        append(
            &mut output.mesh_instances,
            instances,
            &self.source.mesh_instances,
            control,
        )?;
        for (length, actual, plan) in [
            (
                output.logical_meshes.len(),
                output.logical_meshes.capacity(),
                meshes,
            ),
            (
                output.primitives.len(),
                output.primitives.capacity(),
                primitives,
            ),
            (
                output.attributes.len(),
                output.attributes.capacity(),
                attributes,
            ),
            (
                output.mesh_instances.len(),
                output.mesh_instances.capacity(),
                instances,
            ),
            (output.names.len(), output.names.capacity(), names),
            (output.f32_values.len(), output.f32_values.capacity(), f32s),
            (output.f64_values.len(), output.f64_values.capacity(), f64s),
            (output.u32_values.len(), output.u32_values.capacity(), u32s),
            (output.i32_values.len(), output.i32_values.capacity(), i32s),
        ] {
            if length != plan.desired_capacity() {
                return Err(TypedStorageBuildError::Invariant);
            }
            plan.finish(0, actual)
                .map_err(TypedStorageBuildError::Capacity)?;
        }
        Ok(output)
    }

    /// Compare semantics and scalar bits, independently of private scalar/name offsets. Central
    /// validation separately proves canonical pool coverage and Scene-wide references.
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        &self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        macro_rules! check {
            ($action:expr) => {
                if control.run_unit(|| $action)? != Some(true) {
                    return Ok(false);
                }
            };
        }
        if [
            actual.logical_meshes.len(),
            actual.primitives.len(),
            actual.attributes.len(),
            actual.mesh_instances.len(),
            actual.names.len(),
            actual.f32_values.len(),
            actual.f64_values.len(),
            actual.u32_values.len(),
            actual.i32_values.len(),
        ] != self.capacities.map(|plan| plan.desired_capacity())
        {
            return Ok(false);
        }
        for (index, row) in actual.attributes.iter().enumerate() {
            // The sealed mapping fixes the canonical Normal position as well as all shifted rows.
            if index as u32 == self.normal_attribute() {
                check!(
                    row.semantic == StoredAttributeSemantic::Normal
                        && row.domain == AttributeDomain::Corner
                        && row.lanes == 3
                        && row.element_count == self.output_corner_count()
                );
                let StoredAttributePayload::DenseF32(range) = row.payload else {
                    return Ok(false);
                };
                let Some(values) = range.get(&actual.f32_values) else {
                    return Ok(false);
                };
                let expected = self.normals.as_flattened();
                if !compare_values(expected, values, |a, b| a.to_bits() == b.to_bits(), control)? {
                    return Ok(false);
                }
            } else {
                let Some(source) = self
                    .source_attribute(index as u32)
                    .and_then(|index| self.source.attributes.get(index as usize))
                else {
                    return Ok(false);
                };
                check!(
                    same_semantic_kind(source.semantic, row.semantic)
                        && source.domain == row.domain
                        && source.lanes == row.lanes
                        && source.element_count == row.element_count
                );
                if let (
                    StoredAttributeSemantic::Custom(source),
                    StoredAttributeSemantic::Custom(row),
                ) = (source.semantic, row.semantic)
                    && !compare_ranges(
                        source.0,
                        row.0,
                        &self.source.names,
                        &actual.names,
                        |a, b| a == b,
                        control,
                    )?
                {
                    return Ok(false);
                }
                if !compare_payload(source.payload, row.payload, self.source, actual, control)? {
                    return Ok(false);
                }
            }
        }
        for (index, (source, row)) in self
            .source
            .primitives
            .iter()
            .zip(&actual.primitives)
            .enumerate()
        {
            check!(
                source.topology == row.topology
                    && source.vertex_count == row.vertex_count
                    && source.corner_count == row.corner_count
                    && source.face_count == row.face_count
                    && source.material == row.material
                    && self
                        .primitive_attributes(index, source)
                        .is_some_and(|expected| same_range(expected, row.attributes))
            );
            for (source, row) in [
                (source.polygon_boundaries, row.polygon_boundaries),
                (source.indices, row.indices),
            ] {
                if !compare_optional_ranges(
                    source,
                    row,
                    &self.source.u32_values,
                    &actual.u32_values,
                    control,
                )? {
                    return Ok(false);
                }
            }
        }
        for (source, row) in self
            .source
            .logical_meshes
            .iter()
            .zip(&actual.logical_meshes)
        {
            check!(
                same_range(source.primitives, row.primitives)
                    && source.name.is_some() == row.name.is_some()
            );
            if !compare_optional_ranges(
                source.name.map(|name| name.0),
                row.name.map(|name| name.0),
                &self.source.names,
                &actual.names,
                control,
            )? {
                return Ok(false);
            }
        }
        for (source, row) in self
            .source
            .mesh_instances
            .iter()
            .zip(&actual.mesh_instances)
        {
            check!(source.owner_node == row.owner_node && source.logical_mesh == row.logical_mesh);
        }
        Ok(true)
    }
}

fn attribute_scalar_counts<E>(
    source: &GeometryStorage,
    attribute: StoredAttribute,
) -> EditResult<ScalarCounts, E> {
    let lanes = usize::from(attribute.lanes);
    if !(1..=16).contains(&lanes) {
        return Err(TypedStorageBuildError::Invariant);
    }
    let mut counts = ScalarCounts::default();
    macro_rules! dense {
        ($range:expr, $pool:ident, $count:ident) => {{
            validate_dense_range($range, &source.$pool, attribute)?;
            counts.$count = $range.len();
        }};
    }
    macro_rules! sparse {
        ($indices:expr, $values:expr, $default:expr, $pool:ident, $count:ident) => {{
            if $indices.checked(source.u32_values.len()).is_none()
                || $values.checked(source.$pool.len()).is_none()
                || $default.checked(source.$pool.len()).is_none()
                || $indices.len().checked_mul(lanes) != Some($values.len())
                || $default.len() != lanes
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            counts.u32s = $indices.len();
            let payload = $values
                .len()
                .checked_add($default.len())
                .ok_or_else(overflow)?;
            // The sparse U32 shape stores indices, values and default in the same pool.
            add_count(&mut counts.$count, payload)?;
        }};
    }
    use StoredAttributePayload::*;
    match attribute.payload {
        DenseF32(range) => dense!(range, f32_values, f32s),
        DenseF64(range) => dense!(range, f64_values, f64s),
        DenseU32(range) => dense!(range, u32_values, u32s),
        DenseI32(range) => dense!(range, i32_values, i32s),
        SparseF32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, f32_values, f32s),
        SparseU32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, u32_values, u32s),
        SparseI32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, i32_values, i32s),
    }
    Ok(counts)
}

fn copy_payload<C: TypedStorageBuildControl>(
    payload: StoredAttributePayload,
    source: &GeometryStorage,
    output: &mut GeometryStorage,
    capacities: [CapacityPlan; 9],
    control: &mut C,
) -> EditResult<StoredAttributePayload, C::Error> {
    let [_, _, _, _, _, f32s, f64s, u32s, i32s] = capacities;
    macro_rules! dense {
        ($range:expr, $pool:ident, $capacity:expr, $variant:ident) => {
            StoredAttributePayload::$variant(append(
                &mut output.$pool,
                $capacity,
                $range
                    .get(&source.$pool)
                    .ok_or(TypedStorageBuildError::Invariant)?,
                control,
            )?)
        };
    }
    macro_rules! sparse {
        ($indices:expr, $values:expr, $default:expr, $pool:ident, $capacity:expr, $variant:ident) => {{
            let indices = append(
                &mut output.u32_values,
                u32s,
                $indices
                    .get(&source.u32_values)
                    .ok_or(TypedStorageBuildError::Invariant)?,
                control,
            )?;
            let values = append(
                &mut output.$pool,
                $capacity,
                $values
                    .get(&source.$pool)
                    .ok_or(TypedStorageBuildError::Invariant)?,
                control,
            )?;
            let default = append(
                &mut output.$pool,
                $capacity,
                $default
                    .get(&source.$pool)
                    .ok_or(TypedStorageBuildError::Invariant)?,
                control,
            )?;
            StoredAttributePayload::$variant {
                indices,
                values,
                default,
            }
        }};
    }
    use StoredAttributePayload::*;
    Ok(match payload {
        DenseF32(range) => dense!(range, f32_values, f32s, DenseF32),
        DenseF64(range) => dense!(range, f64_values, f64s, DenseF64),
        DenseU32(range) => dense!(range, u32_values, u32s, DenseU32),
        DenseI32(range) => dense!(range, i32_values, i32s, DenseI32),
        SparseF32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, f32_values, f32s, SparseF32),
        SparseU32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, u32_values, u32s, SparseU32),
        SparseI32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, i32_values, i32s, SparseI32),
    })
}

fn same_semantic_kind(source: StoredAttributeSemantic, actual: StoredAttributeSemantic) -> bool {
    match (source, actual) {
        (StoredAttributeSemantic::Custom(_), StoredAttributeSemantic::Custom(_)) => true,
        _ => source == actual,
    }
}

fn compare_values<T: Copy, C: TypedStorageValidationControl>(
    source: &[T],
    actual: &[T],
    equal: impl Fn(T, T) -> bool,
    control: &mut C,
) -> Result<bool, C::Error> {
    if source.len() != actual.len() {
        return Ok(false);
    }
    for (&source, &actual) in source.iter().zip(actual) {
        if control.run_unit(|| equal(source, actual))? != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn compare_ranges<T: Copy, C: TypedStorageValidationControl>(
    source_range: TableRange<T>,
    actual_range: TableRange<T>,
    source: &[T],
    actual: &[T],
    equal: impl Fn(T, T) -> bool,
    control: &mut C,
) -> Result<bool, C::Error> {
    let (Some(source), Some(actual)) = (source_range.get(source), actual_range.get(actual)) else {
        return Ok(false);
    };
    compare_values(source, actual, equal, control)
}

fn compare_optional_ranges<T: Copy + PartialEq, C: TypedStorageValidationControl>(
    source_range: Option<TableRange<T>>,
    actual_range: Option<TableRange<T>>,
    source: &[T],
    actual: &[T],
    control: &mut C,
) -> Result<bool, C::Error> {
    match (source_range, actual_range) {
        (None, None) => Ok(true),
        (Some(source_range), Some(actual_range)) => compare_ranges(
            source_range,
            actual_range,
            source,
            actual,
            |a, b| a == b,
            control,
        ),
        _ => Ok(false),
    }
}

fn compare_payload<C: TypedStorageValidationControl>(
    source_payload: StoredAttributePayload,
    actual_payload: StoredAttributePayload,
    source: &GeometryStorage,
    actual: &GeometryStorage,
    control: &mut C,
) -> Result<bool, C::Error> {
    macro_rules! dense {
        ($sr:expr, $ar:expr, $pool:ident, $equal:expr) => {
            compare_ranges($sr, $ar, &source.$pool, &actual.$pool, $equal, control)
        };
    }
    macro_rules! sparse {
        ($si:expr, $sv:expr, $sd:expr, $ai:expr, $av:expr, $ad:expr, $pool:ident, $equal:expr) => {
            Ok(compare_ranges(
                $si,
                $ai,
                &source.u32_values,
                &actual.u32_values,
                |a, b| a == b,
                control,
            )? && compare_ranges($sv, $av, &source.$pool, &actual.$pool, $equal, control)?
                && compare_ranges($sd, $ad, &source.$pool, &actual.$pool, $equal, control)?)
        };
    }
    use StoredAttributePayload::*;
    match (source_payload, actual_payload) {
        (DenseF32(sr), DenseF32(ar)) => dense!(sr, ar, f32_values, |a: f32, b: f32| a.to_bits()
            == b.to_bits()),
        (DenseF64(sr), DenseF64(ar)) => dense!(sr, ar, f64_values, |a: f64, b: f64| a.to_bits()
            == b.to_bits()),
        (DenseU32(sr), DenseU32(ar)) => dense!(sr, ar, u32_values, |a, b| a == b),
        (DenseI32(sr), DenseI32(ar)) => dense!(sr, ar, i32_values, |a, b| a == b),
        (
            SparseF32 {
                indices: si,
                values: sv,
                default: sd,
            },
            SparseF32 {
                indices: ai,
                values: av,
                default: ad,
            },
        ) => sparse!(si, sv, sd, ai, av, ad, f32_values, |a: f32, b: f32| a
            .to_bits()
            == b.to_bits()),
        (
            SparseU32 {
                indices: si,
                values: sv,
                default: sd,
            },
            SparseU32 {
                indices: ai,
                values: av,
                default: ad,
            },
        ) => sparse!(si, sv, sd, ai, av, ad, u32_values, |a, b| a == b),
        (
            SparseI32 {
                indices: si,
                values: sv,
                default: sd,
            },
            SparseI32 {
                indices: ai,
                values: av,
                default: ad,
            },
        ) => sparse!(si, sv, sd, ai, av, ad, i32_values, |a, b| a == b),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests;
