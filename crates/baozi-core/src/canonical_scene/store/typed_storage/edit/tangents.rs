//! Bounded replacement of one TriangleList primitive's tangent attribute.
//!
//! The source and provisional corner values stay borrowed through central correspondence
//! validation. Publication and all nine capacity admissions remain with the Scene edit lane.

use baozi_operation::CapacityPlan;

use crate::attribute::{
    AttributeDomain, TangentBasisConvention, TangentConventions, TangentHandednessEncoding,
};
use crate::geometry::TopologyKind;

use super::super::build::append_name;
use super::super::validation::TypedStorageValidationControl;
use super::super::{
    StoredAttributeSemantic, StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
};
use super::normals::{
    attribute_scalar_counts, compare_optional_ranges, compare_payload, compare_values, copy_payload,
};
use super::{
    EditResult, GeometryStorage, ScalarCounts, StoredAttribute, StoredAttributePayload,
    StoredLogicalMesh, StoredMeshInstance, StoredPrimitive, TableRange, TypedStorageBuildControl,
    TypedStorageBuildError, add_count, allocate, append, capacity, compare_tangent_schema_names,
    copy_attribute_names, copy_optional, overflow, push, same_range, same_tangent_convention_kinds,
};

const UNIT_LENGTH_TOLERANCE: f64 = 4.0 * f32::EPSILON as f64;
const GENERATED_CONVENTIONS: StoredTangentConventions = StoredTangentConventions {
    basis: StoredTangentBasis::TriangleUvDerivative,
    encoding: StoredTangentEncoding::SignInW,
};

/// Allocation-free global owner correspondence, including a replaced tangent's source owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct TangentAttributeMapping {
    source_attributes: u32,
    output_attributes: u32,
    tangent_attribute: u32,
    source_tangent: Option<u32>,
    source_position: u32,
    source_normal: u32,
    source_uv: u32,
}

impl TangentAttributeMapping {
    pub(in crate::canonical_scene::store) const fn tangent_attribute(self) -> u32 {
        self.tangent_attribute
    }

    pub(in crate::canonical_scene::store) const fn source_tangent(self) -> Option<u32> {
        self.source_tangent
    }

    pub(in crate::canonical_scene::store) const fn source_position(self) -> u32 {
        self.source_position
    }

    pub(in crate::canonical_scene::store) const fn source_normal(self) -> u32 {
        self.source_normal
    }

    pub(in crate::canonical_scene::store) const fn source_uv(self) -> u32 {
        self.source_uv
    }

    pub(in crate::canonical_scene::store) fn output_attribute(self, source: u32) -> Option<u32> {
        if source >= self.source_attributes {
            return None;
        }
        source.checked_add(u32::from(
            self.source_tangent.is_none() && source >= self.tangent_attribute,
        ))
    }

    pub(in crate::canonical_scene::store) fn source_attribute(self, output: u32) -> Option<u32> {
        if output >= self.output_attributes {
            return None;
        }
        if output == self.tangent_attribute {
            return self.source_tangent;
        }
        output.checked_sub(u32::from(
            self.source_tangent.is_none() && output > self.tangent_attribute,
        ))
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryTangentsPlan<'a> {
    source: &'a GeometryStorage,
    tangents: &'a [[f32; 4]],
    target: usize,
    capacities: [CapacityPlan; 9],
    work: u64,
    validation_work: u64,
    mapping: TangentAttributeMapping,
}

impl<'a> GeometryTangentsPlan<'a> {
    /// The source is centrally validated. Every source-sized scan and supplied tuple is metered;
    /// this phase allocates nothing and does not change the source or supplied values.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        tangents: &'a [[f32; 4]],
        had_tangent: bool,
        normal_domain: AttributeDomain,
        uv_set: u32,
        uv_domain: AttributeDomain,
        conventions: &TangentConventions,
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
                    || tangents.len() != primitive.corner_count as usize
                    || !matches!(
                        normal_domain,
                        AttributeDomain::Vertex | AttributeDomain::Corner
                    )
                    || !matches!(uv_domain, AttributeDomain::Vertex | AttributeDomain::Corner)
                    || !matches!(
                        conventions.basis,
                        TangentBasisConvention::TriangleUvDerivative
                    )
                    || !matches!(conventions.encoding, TangentHandednessEncoding::SignInW)
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
        for tangent in tangents {
            let valid = control
                .run_unit(|| {
                    let [x, y, z, w] = tangent.map(f64::from);
                    x.is_finite()
                        && y.is_finite()
                        && z.is_finite()
                        && (w == 1.0 || w == -1.0)
                        && (x * x + y * y + z * z - 1.0).abs() <= UNIT_LENGTH_TOLERANCE
                })
                .map_err(TypedStorageBuildError::Control)?;
            if !valid {
                return Err(TypedStorageBuildError::Invariant);
            }
        }

        let source_attributes = u32::try_from(source.attributes.len()).map_err(|_| overflow())?;
        let mut insertion = u32::try_from(target_attributes.end).map_err(|_| overflow())?;
        let mut source_tangent = None;
        let mut source_position = None;
        let mut source_normal = None;
        let mut source_uv = None;
        let mut scalars = ScalarCounts::default();
        let mut next_name = 0_usize;
        let mut removed_names = 0_usize;
        for (index, attribute) in source.attributes.iter().enumerate() {
            let counts = control
                .run_unit(|| {
                    let counts = attribute_scalar_counts(source, *attribute)?;
                    let name_start = next_name;
                    if let StoredAttributeSemantic::Custom(name) = attribute.semantic {
                        if name.0.start() != next_name || name.get(&source.names).is_none() {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        add_count(&mut next_name, name.0.len())?;
                    }
                    if let Some(conventions) = attribute.tangent_conventions {
                        for name in conventions.schema_names() {
                            if name.0.start() != next_name || name.get(&source.names).is_none() {
                                return Err(TypedStorageBuildError::Invariant);
                            }
                            add_count(&mut next_name, name.0.len())?;
                        }
                    }
                    if target_attributes.contains(&index) {
                        if index < insertion as usize
                            && !matches!(
                                attribute.semantic,
                                StoredAttributeSemantic::Position | StoredAttributeSemantic::Normal
                            )
                        {
                            insertion = index as u32;
                        }
                        let slot = match attribute.semantic {
                            StoredAttributeSemantic::Position => Some(&mut source_position),
                            StoredAttributeSemantic::Normal
                                if attribute.domain == normal_domain =>
                            {
                                Some(&mut source_normal)
                            }
                            StoredAttributeSemantic::Texcoord(set)
                                if set == uv_set && attribute.domain == uv_domain =>
                            {
                                if attribute.lanes != 2 {
                                    return Err(TypedStorageBuildError::Invariant);
                                }
                                Some(&mut source_uv)
                            }
                            StoredAttributeSemantic::Tangent => Some(&mut source_tangent),
                            _ => None,
                        };
                        if let Some(slot) = slot
                            && slot.replace(index as u32).is_some()
                        {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        if attribute.semantic == StoredAttributeSemantic::Tangent {
                            removed_names = next_name - name_start;
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
        let source_normal = source_normal.ok_or(TypedStorageBuildError::Invariant)?;
        let source_uv = source_uv.ok_or(TypedStorageBuildError::Invariant)?;
        if had_tangent != source_tangent.is_some() {
            return Err(TypedStorageBuildError::Invariant);
        }
        let output_attributes = source_attributes
            .checked_add(u32::from(source_tangent.is_none()))
            .ok_or_else(overflow)?;
        add_count(
            &mut scalars.f32s,
            tangents.len().checked_mul(4).ok_or_else(overflow)?,
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
                        if name.0.start() != next_name || name.get(&source.names).is_none() {
                            return Err(TypedStorageBuildError::Invariant);
                        }
                        add_count(&mut next_name, name.0.len())?;
                    }
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        if next_attribute != source.attributes.len()
            || next_primitive != source.primitives.len()
            || next_name != source.names.len()
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let names = source
            .names
            .len()
            .checked_sub(removed_names)
            .ok_or_else(overflow)?;
        let capacities = [
            capacity::<StoredLogicalMesh, C::Error>(source.logical_meshes.len())?,
            capacity::<StoredPrimitive, C::Error>(source.primitives.len())?,
            capacity::<StoredAttribute, C::Error>(output_attributes as usize)?,
            capacity::<StoredMeshInstance, C::Error>(source.mesh_instances.len())?,
            capacity::<u8, C::Error>(names)?,
            capacity::<f32, C::Error>(scalars.f32s)?,
            capacity::<f64, C::Error>(scalars.f64s)?,
            capacity::<u32, C::Error>(scalars.u32s)?,
            capacity::<i32, C::Error>(scalars.i32s)?,
        ];
        let work = capacities.iter().try_fold(0_u64, |work, plan| {
            work.checked_add(u64::try_from(plan.desired_capacity()).map_err(|_| overflow())?)
                .ok_or_else(overflow)
        })?;
        let validation_work = work
            .checked_sub(u64::try_from(names).map_err(|_| overflow())?)
            .and_then(|work| work.checked_add(u64::try_from(source.primitives.len()).ok()?))
            .ok_or_else(overflow)?;
        Ok(Self {
            source,
            tangents,
            target,
            capacities,
            work,
            validation_work,
            mapping: TangentAttributeMapping {
                source_attributes,
                output_attributes,
                tangent_attribute: source_tangent.unwrap_or(insertion),
                source_tangent,
                source_position,
                source_normal,
                source_uv,
            },
        })
    }

    pub(in crate::canonical_scene::store) const fn target_row(&self) -> usize {
        self.target
    }

    pub(in crate::canonical_scene::store) const fn capacities(&self) -> [CapacityPlan; 9] {
        self.capacities
    }

    pub(in crate::canonical_scene::store) const fn work(&self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.validation_work
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(&self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(&self) -> usize {
        self.capacities[4].desired_capacity()
    }

    pub(in crate::canonical_scene::store) const fn attribute_mapping(
        &self,
    ) -> TangentAttributeMapping {
        self.mapping
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(&self) -> u32 {
        self.mapping.output_attributes
    }

    pub(in crate::canonical_scene::store) fn output_corner_count(&self) -> u32 {
        self.tangents.len() as u32
    }

    fn primitive_attributes(
        &self,
        index: usize,
        primitive: &StoredPrimitive,
    ) -> Option<TableRange<StoredAttribute>> {
        let added = self.mapping.source_tangent.is_none();
        let start = primitive.attributes.start().checked_add(usize::from(
            added && primitive.attributes.start() >= self.mapping.tangent_attribute as usize,
        ))?;
        let count = primitive
            .attributes
            .len()
            .checked_add(usize::from(added && index == self.target))?;
        Some(TableRange::new(start, count))
    }

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
        for index in 0..self.output_attribute_count() {
            let row = if index == self.mapping.tangent_attribute {
                let range = append(
                    &mut output.f32_values,
                    f32s,
                    self.tangents.as_flattened(),
                    control,
                )?;
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Tangent,
                    tangent_conventions: Some(GENERATED_CONVENTIONS),
                    domain: AttributeDomain::Corner,
                    lanes: 4,
                    element_count: self.output_corner_count(),
                    payload: StoredAttributePayload::DenseF32(range),
                }
            } else {
                let source = self
                    .mapping
                    .source_attribute(index)
                    .and_then(|index| self.source.attributes.get(index as usize))
                    .ok_or(TypedStorageBuildError::Invariant)?;
                let source =
                    copy_attribute_names(self.source, *source, &mut output.names, names, control)?;
                StoredAttribute {
                    payload: copy_payload(
                        source.payload,
                        self.source,
                        &mut output,
                        self.capacities,
                        control,
                    )?,
                    ..source
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
        for source in &self.source.logical_meshes {
            let name = match source.name {
                Some(name) => append_name(
                    &mut output.names,
                    names,
                    crate::canonical_scene::store::production::Presence::Present(
                        name.get(&self.source.names)
                            .ok_or(TypedStorageBuildError::Invariant)?,
                    ),
                    control,
                )?,
                None => None,
            };
            push(
                &mut output.logical_meshes,
                meshes,
                StoredLogicalMesh { name, ..*source },
                control,
            )?;
        }
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

    /// Checks every retained payload bit and name, independently of compacted pool offsets.
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
            if index as u32 == self.mapping.tangent_attribute {
                check!(
                    row.semantic == StoredAttributeSemantic::Tangent
                        && row.tangent_conventions == Some(GENERATED_CONVENTIONS)
                        && row.domain == AttributeDomain::Corner
                        && row.lanes == 4
                        && row.element_count == self.output_corner_count()
                );
                let StoredAttributePayload::DenseF32(range) = row.payload else {
                    return Ok(false);
                };
                let Some(values) = range.get(&actual.f32_values) else {
                    return Ok(false);
                };
                if !compare_values(
                    self.tangents.as_flattened(),
                    values,
                    |a, b| a.to_bits() == b.to_bits(),
                    control,
                )? {
                    return Ok(false);
                }
            } else {
                let Some(source) = self
                    .mapping
                    .source_attribute(index as u32)
                    .and_then(|index| self.source.attributes.get(index as usize))
                else {
                    return Ok(false);
                };
                check!(
                    (match (source.semantic, row.semantic) {
                        (
                            StoredAttributeSemantic::Custom(_),
                            StoredAttributeSemantic::Custom(_),
                        ) => true,
                        (source, row) => source == row,
                    }) && same_tangent_convention_kinds(
                        source.tangent_conventions,
                        row.tangent_conventions
                    ) && source.domain == row.domain
                        && source.lanes == row.lanes
                        && source.element_count == row.element_count
                );
                if let (
                    StoredAttributeSemantic::Custom(source),
                    StoredAttributeSemantic::Custom(row),
                ) = (source.semantic, row.semantic)
                    && !compare_optional_ranges(
                        Some(source.0),
                        Some(row.0),
                        &self.source.names,
                        &actual.names,
                        control,
                    )?
                {
                    return Ok(false);
                }
                if !compare_tangent_schema_names(
                    source.tangent_conventions,
                    row.tangent_conventions,
                    &self.source.names,
                    &actual.names,
                    control,
                )? || !compare_payload(
                    source.payload,
                    row.payload,
                    self.source,
                    actual,
                    control,
                )? {
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

#[cfg(test)]
#[path = "tangents_tests.rs"]
mod tests;
