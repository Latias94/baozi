//! Source-bound conversion of every geometry row, preserving vertex and face identities.
//!
//! The host admits all nine capacities before materialization. Scalar pools are rebuilt in canonical
//! attribute order, followed by primitive boundaries and indices; no remapping table is allocated.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;
use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, finish, push,
};
use super::super::validation::TypedStorageValidationControl;
use super::super::{
    GeometryStorage, StoredAttribute, StoredAttributePayload, StoredAttributeSemantic,
    StoredLogicalMesh, StoredMeshInstance, StoredPrimitive, TableRange,
};

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum GeometrySpacePlanError<E> {
    Control(E),
    Numeric {
        attribute: usize,
        source: NumericConversionError,
    },
    UnsupportedTopology {
        primitive: usize,
        topology: TopologyKind,
    },
    CustomAttribute {
        attribute: usize,
    },
    Plan(StorePlanError),
    Invariant,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometrySpacePlan<'a> {
    source: &'a GeometryStorage,
    conversion: SpaceConversion,
    capacities: [CapacityPlan; 9],
    work: u64,
    validation_work: u64,
}

#[derive(Clone, Copy)]
enum SpatialTuple {
    Point,
    Direction,
    Tangent,
}

impl<'a> GeometrySpacePlan<'a> {
    /// The source is already centrally valid. This preflight checks capabilities, final capacities,
    /// and the actual spatial tuples, so ordinary numeric errors precede retained admission.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, GeometrySpacePlanError<C::Error>> {
        let mut extra_indices = 0_usize;
        let mut payload_work = 0_u64;
        for (index, primitive) in source.primitives.iter().enumerate() {
            let (extra, work) = control
                .run_unit(|| -> Result<_, GeometrySpacePlanError<C::Error>> {
                    if conversion.reverse_winding()
                        && primitive.corner_count != 0
                        && matches!(
                            primitive.topology,
                            TopologyKind::Polygons
                                | TopologyKind::TriangleStrip
                                | TopologyKind::TriangleFan
                        )
                    {
                        return Err(GeometrySpacePlanError::UnsupportedTopology {
                            primitive: index,
                            topology: primitive.topology,
                        });
                    }
                    if primitive
                        .attributes
                        .checked(source.attributes.len())
                        .is_none()
                    {
                        return Err(GeometrySpacePlanError::Invariant);
                    }
                    let extra = if needs_implicit_indices(primitive, conversion) {
                        primitive.corner_count as usize
                    } else {
                        0
                    };
                    let work = [primitive.polygon_boundaries, primitive.indices]
                        .into_iter()
                        .flatten()
                        .try_fold(extra, |sum, range| {
                            range.get(&source.u32_values)?;
                            sum.checked_add(range.len())
                        })
                        .ok_or(GeometrySpacePlanError::Invariant)?;
                    Ok((extra, work))
                })
                .map_err(GeometrySpacePlanError::Control)??;
            extra_indices = extra_indices.checked_add(extra).ok_or_else(plan_overflow)?;
            add_work(&mut payload_work, work).map_err(GeometrySpacePlanError::Plan)?;
        }
        for (index, attribute) in source.attributes.iter().enumerate() {
            let (spatial, work) = control
                .run_unit(|| -> Result<_, GeometrySpacePlanError<C::Error>> {
                    if conversion.changes_coordinates()
                        && matches!(attribute.semantic, StoredAttributeSemantic::Custom(_))
                    {
                        return Err(GeometrySpacePlanError::CustomAttribute { attribute: index });
                    }
                    let work = attribute_work(source, *attribute)
                        .ok_or(GeometrySpacePlanError::Invariant)?;
                    let spatial =
                        spatial_tuple(*attribute).ok_or(GeometrySpacePlanError::Invariant)?;
                    Ok((spatial, work))
                })
                .map_err(GeometrySpacePlanError::Control)??;
            payload_work = payload_work.checked_add(work).ok_or_else(plan_overflow)?;
            let Some(spatial) = spatial else { continue };
            let (values, default) = match attribute.payload {
                StoredAttributePayload::DenseF32(values) => (values, None),
                StoredAttributePayload::SparseF32 {
                    values, default, ..
                } => (values, Some(default)),
                _ => return Err(GeometrySpacePlanError::Invariant),
            };
            for range in [Some(values), default].into_iter().flatten() {
                let values = range
                    .get(&source.f32_values)
                    .ok_or(GeometrySpacePlanError::Invariant)?;
                for tuple in values.chunks_exact(usize::from(attribute.lanes)) {
                    control
                        .run_unit(|| convert_tuple(conversion, spatial, tuple))
                        .map_err(GeometrySpacePlanError::Control)?
                        .map_err(|source| GeometrySpacePlanError::Numeric {
                            attribute: index,
                            source,
                        })?;
                }
            }
        }
        let u32_count = source
            .u32_values
            .len()
            .checked_add(extra_indices)
            .ok_or_else(plan_overflow)?;
        let capacities = [
            capacity::<StoredLogicalMesh>(source.logical_meshes.len()),
            capacity::<StoredPrimitive>(source.primitives.len()),
            capacity::<StoredAttribute>(source.attributes.len()),
            capacity::<StoredMeshInstance>(source.mesh_instances.len()),
            capacity::<u8>(source.names.len()),
            capacity::<f32>(source.f32_values.len()),
            capacity::<f64>(source.f64_values.len()),
            capacity::<u32>(u32_count),
            capacity::<i32>(source.i32_values.len()),
        ];
        let [
            logical_meshes,
            primitives,
            attributes,
            instances,
            names,
            f32s,
            f64s,
            u32s,
            i32s,
        ] = capacities;
        let capacities = [
            logical_meshes.map_err(GeometrySpacePlanError::Plan)?,
            primitives.map_err(GeometrySpacePlanError::Plan)?,
            attributes.map_err(GeometrySpacePlanError::Plan)?,
            instances.map_err(GeometrySpacePlanError::Plan)?,
            names.map_err(GeometrySpacePlanError::Plan)?,
            f32s.map_err(GeometrySpacePlanError::Plan)?,
            f64s.map_err(GeometrySpacePlanError::Plan)?,
            u32s.map_err(GeometrySpacePlanError::Plan)?,
            i32s.map_err(GeometrySpacePlanError::Plan)?,
        ];
        let row_work = sum_counts([
            source.logical_meshes.len(),
            source.primitives.len(),
            source.primitives.len(),
            source.attributes.len(),
            source.mesh_instances.len(),
        ])
        .map_err(GeometrySpacePlanError::Plan)?;
        let work = row_work
            .checked_add(payload_work)
            .and_then(|work| work.checked_add(source.names.len() as u64))
            .ok_or_else(plan_overflow)?;
        let validation_work = row_work
            .checked_add(
                sum_counts([
                    source.f32_values.len(),
                    source.f64_values.len(),
                    u32_count,
                    source.i32_values.len(),
                ])
                .map_err(GeometrySpacePlanError::Plan)?,
            )
            .ok_or_else(plan_overflow)?;
        Ok(Self {
            source,
            conversion,
            capacities,
            work,
            validation_work,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 9] {
        self.capacities
    }
    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }
    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }
    pub(in crate::canonical_scene::store) const fn validation_work(self) -> u64 {
        self.validation_work
    }
    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.capacities[4].desired_capacity()
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> BuildResult<GeometryStorage, C::Error> {
        let c = self.capacities;
        let mut output = GeometryStorage {
            logical_meshes: allocate(c[0])?,
            primitives: allocate(c[1])?,
            attributes: allocate(c[2])?,
            mesh_instances: allocate(c[3])?,
            names: allocate(c[4])?,
            f32_values: allocate(c[5])?,
            f64_values: allocate(c[6])?,
            u32_values: allocate(c[7])?,
            i32_values: allocate(c[8])?,
        };
        // This pass is separately metered even for primitives with no attributes.
        for primitive in &self.source.primitives {
            let (attributes, flip) = control
                .run_unit(|| owner(self.source, primitive, self.conversion))
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
            for index in attributes {
                let attribute = self.source.attributes[index];
                let payload = self.copy_payload(attribute, flip, &mut output, control)?;
                room(&output.attributes, c[2], 1)?;
                control
                    .run_unit(|| {
                        output.attributes.push(StoredAttribute {
                            payload,
                            ..attribute
                        })
                    })
                    .map_err(TypedStorageBuildError::Control)?;
            }
        }
        // All attribute U32 ranges precede all primitive U32 ranges, including synthesized indices.
        for primitive in &self.source.primitives {
            let polygon_boundaries = match primitive.polygon_boundaries {
                Some(range) => Some(append(
                    &mut output.u32_values,
                    c[7],
                    range
                        .get(&self.source.u32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    control,
                )?),
                None => None,
            };
            let indices =
                self.copy_primitive_indices(primitive, &mut output.u32_values, control)?;
            room(&output.primitives, c[1], 1)?;
            control
                .run_unit(|| {
                    output.primitives.push(StoredPrimitive {
                        polygon_boundaries,
                        indices,
                        ..*primitive
                    })
                })
                .map_err(TypedStorageBuildError::Control)?;
        }
        for &mesh in &self.source.logical_meshes {
            push(&mut output.logical_meshes, c[0], mesh, control)?;
        }
        for &instance in &self.source.mesh_instances {
            push(&mut output.mesh_instances, c[3], instance, control)?;
        }
        for &byte in &self.source.names {
            room(&output.names, c[4], 1)?;
            control
                .run_string_unit(|| output.names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        finish(&output.logical_meshes, c[0])?;
        finish(&output.primitives, c[1])?;
        finish(&output.attributes, c[2])?;
        finish(&output.mesh_instances, c[3])?;
        finish(&output.names, c[4])?;
        finish(&output.f32_values, c[5])?;
        finish(&output.f64_values, c[6])?;
        finish(&output.u32_values, c[7])?;
        finish(&output.i32_values, c[8])?;
        Ok(output)
    }

    fn copy_payload<C: TypedStorageBuildControl>(
        self,
        attribute: StoredAttribute,
        flip: bool,
        output: &mut GeometryStorage,
        control: &mut C,
    ) -> BuildResult<StoredAttributePayload, C::Error> {
        let lanes = usize::from(attribute.lanes);
        let remap = flip && attribute.domain == AttributeDomain::Corner;
        let spatial = spatial_tuple(attribute).ok_or(TypedStorageBuildError::Invariant)?;
        macro_rules! dense {
            ($range:expr, $pool:ident, $cap:expr, $variant:ident) => {{
                let values = $range
                    .get(&self.source.$pool)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                StoredAttributePayload::$variant(copy_tuples(
                    values,
                    None,
                    lanes,
                    remap,
                    &mut output.$pool,
                    self.capacities[$cap],
                    control,
                )?)
            }};
        }
        macro_rules! sparse {
            ($indices:expr, $values:expr, $default:expr, $pool:ident, $cap:expr, $variant:ident) => {{
                let ids = $indices
                    .get(&self.source.u32_values)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                let indices = copy_sparse_indices(
                    ids,
                    remap,
                    &mut output.u32_values,
                    self.capacities[7],
                    control,
                )?;
                let values = copy_tuples(
                    $values
                        .get(&self.source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    Some(ids),
                    lanes,
                    remap,
                    &mut output.$pool,
                    self.capacities[$cap],
                    control,
                )?;
                let default = copy_tuples(
                    $default
                        .get(&self.source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    None,
                    lanes,
                    false,
                    &mut output.$pool,
                    self.capacities[$cap],
                    control,
                )?;
                StoredAttributePayload::$variant {
                    indices,
                    values,
                    default,
                }
            }};
        }
        Ok(match attribute.payload {
            StoredAttributePayload::DenseF32(range) => {
                StoredAttributePayload::DenseF32(copy_f32_tuples(
                    range
                        .get(&self.source.f32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    TupleMap {
                        indices: None,
                        lanes,
                        remap,
                    },
                    spatial,
                    self.conversion,
                    &mut output.f32_values,
                    self.capacities[5],
                    control,
                )?)
            }
            StoredAttributePayload::DenseF64(range) => dense!(range, f64_values, 6, DenseF64),
            StoredAttributePayload::DenseU32(range) => dense!(range, u32_values, 7, DenseU32),
            StoredAttributePayload::DenseI32(range) => dense!(range, i32_values, 8, DenseI32),
            StoredAttributePayload::SparseF32 {
                indices,
                values,
                default,
            } => {
                let ids = indices
                    .get(&self.source.u32_values)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                let indices = copy_sparse_indices(
                    ids,
                    remap,
                    &mut output.u32_values,
                    self.capacities[7],
                    control,
                )?;
                let values = copy_f32_tuples(
                    values
                        .get(&self.source.f32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    TupleMap {
                        indices: Some(ids),
                        lanes,
                        remap,
                    },
                    spatial,
                    self.conversion,
                    &mut output.f32_values,
                    self.capacities[5],
                    control,
                )?;
                let default = copy_f32_tuples(
                    default
                        .get(&self.source.f32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    TupleMap {
                        indices: None,
                        lanes,
                        remap: false,
                    },
                    spatial,
                    self.conversion,
                    &mut output.f32_values,
                    self.capacities[5],
                    control,
                )?;
                StoredAttributePayload::SparseF32 {
                    indices,
                    values,
                    default,
                }
            }
            StoredAttributePayload::SparseU32 {
                indices,
                values,
                default,
            } => sparse!(indices, values, default, u32_values, 7, SparseU32),
            StoredAttributePayload::SparseI32 {
                indices,
                values,
                default,
            } => sparse!(indices, values, default, i32_values, 8, SparseI32),
        })
    }

    fn copy_primitive_indices<C: TypedStorageBuildControl>(
        self,
        primitive: &StoredPrimitive,
        output: &mut Vec<u32>,
        control: &mut C,
    ) -> BuildResult<Option<TableRange<u32>>, C::Error> {
        let implicit = needs_implicit_indices(primitive, self.conversion);
        let source = match primitive.indices {
            Some(range) => Some(
                range
                    .get(&self.source.u32_values)
                    .ok_or(TypedStorageBuildError::Invariant)?,
            ),
            None => None,
        };
        if source.is_none() && !implicit {
            return Ok(None);
        }
        let count = source.map_or(primitive.corner_count as usize, <[u32]>::len);
        let start = output.len();
        for corner in 0..count {
            room(output, self.capacities[7], 1)?;
            control
                .run_unit(|| {
                    let source_corner =
                        corner_ordinal(corner, flip_triangles(primitive, self.conversion))
                            .ok_or(TypedStorageBuildError::Invariant)?;
                    let index = match source {
                        Some(values) => *values
                            .get(source_corner)
                            .ok_or(TypedStorageBuildError::Invariant)?,
                        None => u32::try_from(source_corner)
                            .map_err(|_| TypedStorageBuildError::Invariant)?,
                    };
                    output.push(index);
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        Ok(Some(TableRange::new(start, count)))
    }

    /// Compares actual fields, canonical ranges, and transformed tuple bits against the exact source.
    /// The only projection is a fixed four-component tuple and four pool cursors on the stack.
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if !same_lengths(actual, self.capacities) {
            return Ok(false);
        }
        let mut pools = [0_usize; 4];
        for primitive in &self.source.primitives {
            let Some(Some((attributes, flip))) =
                control.run_unit(|| owner(self.source, primitive, self.conversion))?
            else {
                return Ok(false);
            };
            for index in attributes {
                let attribute = self.source.attributes[index];
                let row = actual.attributes[index];
                let Some(payload) = expected_payload(attribute.payload, &mut pools) else {
                    return Ok(false);
                };
                if control.run_unit(|| {
                    row.semantic == attribute.semantic
                        && row.domain == attribute.domain
                        && row.lanes == attribute.lanes
                        && row.element_count == attribute.element_count
                        && same_payload_ranges(row.payload, payload)
                })? != Some(true)
                {
                    return Ok(false);
                }
                if !self.compare_payload(attribute, row.payload, flip, actual, control)? {
                    return Ok(false);
                }
            }
        }
        for (source, row) in self.source.primitives.iter().zip(&actual.primitives) {
            let boundaries = match source.polygon_boundaries {
                Some(range) => match take_range(&mut pools[2], range.len()) {
                    Some(range) => Some(range),
                    None => return Ok(false),
                },
                None => None,
            };
            let indices =
                if source.indices.is_some() || needs_implicit_indices(source, self.conversion) {
                    let count = source
                        .indices
                        .map_or(source.corner_count as usize, TableRange::len);
                    match take_range(&mut pools[2], count) {
                        Some(range) => Some(range),
                        None => return Ok(false),
                    }
                } else {
                    None
                };
            if control.run_unit(|| {
                source.topology == row.topology
                    && source.vertex_count == row.vertex_count
                    && source.corner_count == row.corner_count
                    && source.face_count == row.face_count
                    && source.material == row.material
                    && source.attributes == row.attributes
                    && row.polygon_boundaries == boundaries
                    && row.indices == indices
            })? != Some(true)
            {
                return Ok(false);
            }
            if let (Some(source), Some(target)) = (source.polygon_boundaries, boundaries) {
                let (Some(source), Some(target)) = (
                    source.get(&self.source.u32_values),
                    target.get(&actual.u32_values),
                ) else {
                    return Ok(false);
                };
                for (&source, &target) in source.iter().zip(target) {
                    if control.run_unit(|| source == target)? != Some(true) {
                        return Ok(false);
                    }
                }
            }
            if let Some(indices) = indices {
                let Some(values) = indices.get(&actual.u32_values) else {
                    return Ok(false);
                };
                let source_indices = match source.indices {
                    Some(range) => match range.get(&self.source.u32_values) {
                        Some(values) => Some(values),
                        None => return Ok(false),
                    },
                    None => None,
                };
                for (corner, &value) in values.iter().enumerate() {
                    if control.run_unit(|| {
                        let Some(origin) =
                            corner_ordinal(corner, flip_triangles(source, self.conversion))
                        else {
                            return false;
                        };
                        source_indices.map_or_else(
                            || u32::try_from(origin).ok(),
                            |indices| indices.get(origin).copied(),
                        ) == Some(value)
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        for (source, row) in self
            .source
            .logical_meshes
            .iter()
            .zip(&actual.logical_meshes)
        {
            if control
                .run_unit(|| source.primitives == row.primitives && source.name == row.name)?
                != Some(true)
            {
                return Ok(false);
            }
        }
        for (source, row) in self
            .source
            .mesh_instances
            .iter()
            .zip(&actual.mesh_instances)
        {
            if control.run_unit(|| {
                source.owner_node == row.owner_node && source.logical_mesh == row.logical_mesh
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        for (&source, &byte) in self.source.names.iter().zip(&actual.names) {
            if control.run_unit(|| source == byte)? != Some(true) {
                return Ok(false);
            }
        }
        Ok(pools
            == [
                actual.f32_values.len(),
                actual.f64_values.len(),
                actual.u32_values.len(),
                actual.i32_values.len(),
            ])
    }

    fn compare_payload<C: TypedStorageValidationControl>(
        self,
        attribute: StoredAttribute,
        actual_payload: StoredAttributePayload,
        flip: bool,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        let lanes = usize::from(attribute.lanes);
        let remap = flip && attribute.domain == AttributeDomain::Corner;
        let Some(spatial) = spatial_tuple(attribute) else {
            return Ok(false);
        };
        macro_rules! compare {
            ($source:expr, $actual:expr, $pool:ident, $ids:expr, $remap:expr, $bits:expr) => {{
                let (Some(source), Some(actual)) =
                    ($source.get(&self.source.$pool), $actual.get(&actual.$pool))
                else {
                    return Ok(false);
                };
                let map = TupleMap {
                    indices: $ids,
                    lanes,
                    remap: $remap,
                };
                for (ordinal, tuple) in actual.chunks_exact(lanes).enumerate() {
                    if control.run_unit(|| {
                        map.get(source, ordinal).is_some_and(|expected| {
                            expected
                                .iter()
                                .copied()
                                .map($bits)
                                .eq(tuple.iter().copied().map($bits))
                        })
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
            }};
        }
        macro_rules! sparse {
            ($si:expr, $sv:expr, $sd:expr, $ai:expr, $av:expr, $ad:expr, $pool:ident, $bits:expr) => {{
                let (Some(ids), Some(output_ids)) = (
                    $si.get(&self.source.u32_values),
                    $ai.get(&actual.u32_values),
                ) else {
                    return Ok(false);
                };
                if !compare_sparse_indices(ids, output_ids, remap, control)? {
                    return Ok(false);
                }
                compare!($sv, $av, $pool, Some(ids), remap, $bits);
                compare!($sd, $ad, $pool, None, false, $bits);
            }};
        }
        match (attribute.payload, actual_payload) {
            (
                StoredAttributePayload::DenseF32(source),
                StoredAttributePayload::DenseF32(target),
            ) => {
                let (Some(source), Some(target)) = (
                    source.get(&self.source.f32_values),
                    target.get(&actual.f32_values),
                ) else {
                    return Ok(false);
                };
                if !compare_f32_tuples(
                    source,
                    target,
                    TupleMap {
                        indices: None,
                        lanes,
                        remap,
                    },
                    spatial,
                    self.conversion,
                    control,
                )? {
                    return Ok(false);
                }
            }
            (
                StoredAttributePayload::DenseF64(source),
                StoredAttributePayload::DenseF64(target),
            ) => compare!(source, target, f64_values, None, remap, f64::to_bits),
            (
                StoredAttributePayload::DenseU32(source),
                StoredAttributePayload::DenseU32(target),
            ) => compare!(
                source,
                target,
                u32_values,
                None,
                remap,
                std::convert::identity
            ),
            (
                StoredAttributePayload::DenseI32(source),
                StoredAttributePayload::DenseI32(target),
            ) => compare!(
                source,
                target,
                i32_values,
                None,
                remap,
                std::convert::identity
            ),
            (
                StoredAttributePayload::SparseF32 {
                    indices: si,
                    values: sv,
                    default: sd,
                },
                StoredAttributePayload::SparseF32 {
                    indices: ai,
                    values: av,
                    default: ad,
                },
            ) => {
                let (Some(ids), Some(output_ids)) =
                    (si.get(&self.source.u32_values), ai.get(&actual.u32_values))
                else {
                    return Ok(false);
                };
                if !compare_sparse_indices(ids, output_ids, remap, control)? {
                    return Ok(false);
                }
                let (Some(source), Some(target)) =
                    (sv.get(&self.source.f32_values), av.get(&actual.f32_values))
                else {
                    return Ok(false);
                };
                if !compare_f32_tuples(
                    source,
                    target,
                    TupleMap {
                        indices: Some(ids),
                        lanes,
                        remap,
                    },
                    spatial,
                    self.conversion,
                    control,
                )? {
                    return Ok(false);
                }
                let (Some(source), Some(target)) =
                    (sd.get(&self.source.f32_values), ad.get(&actual.f32_values))
                else {
                    return Ok(false);
                };
                if !compare_f32_tuples(
                    source,
                    target,
                    TupleMap {
                        indices: None,
                        lanes,
                        remap: false,
                    },
                    spatial,
                    self.conversion,
                    control,
                )? {
                    return Ok(false);
                }
            }
            (
                StoredAttributePayload::SparseU32 {
                    indices: si,
                    values: sv,
                    default: sd,
                },
                StoredAttributePayload::SparseU32 {
                    indices: ai,
                    values: av,
                    default: ad,
                },
            ) => sparse!(si, sv, sd, ai, av, ad, u32_values, std::convert::identity),
            (
                StoredAttributePayload::SparseI32 {
                    indices: si,
                    values: sv,
                    default: sd,
                },
                StoredAttributePayload::SparseI32 {
                    indices: ai,
                    values: av,
                    default: ad,
                },
            ) => sparse!(si, sv, sd, ai, av, ad, i32_values, std::convert::identity),
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn capacity<T>(count: usize) -> Result<CapacityPlan, StorePlanError> {
    CapacityPlan::for_type::<T>(0, 0, count).map_err(StorePlanError::Capacity)
}

fn plan_overflow<E>() -> GeometrySpacePlanError<E> {
    GeometrySpacePlanError::Plan(StorePlanError::SizeOverflow)
}
fn sum_counts<const N: usize>(counts: [usize; N]) -> Result<u64, StorePlanError> {
    counts.into_iter().try_fold(0_u64, |sum, count| {
        let count = u64::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
    })
}
fn add_work(work: &mut u64, count: usize) -> Result<(), StorePlanError> {
    *work = work
        .checked_add(u64::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?)
        .ok_or(StorePlanError::SizeOverflow)?;
    Ok(())
}

fn flip_triangles(primitive: &StoredPrimitive, conversion: SpaceConversion) -> bool {
    conversion.reverse_winding() && primitive.topology == TopologyKind::TriangleList
}
fn needs_implicit_indices(primitive: &StoredPrimitive, conversion: SpaceConversion) -> bool {
    flip_triangles(primitive, conversion)
        && primitive.indices.is_none()
        && primitive.corner_count != 0
}
fn owner(
    source: &GeometryStorage,
    primitive: &StoredPrimitive,
    conversion: SpaceConversion,
) -> Option<(std::ops::Range<usize>, bool)> {
    Some((
        primitive.attributes.checked(source.attributes.len())?,
        flip_triangles(primitive, conversion),
    ))
}

fn spatial_tuple(attribute: StoredAttribute) -> Option<Option<SpatialTuple>> {
    let spatial = match attribute.semantic {
        StoredAttributeSemantic::Position if attribute.lanes == 3 => Some(SpatialTuple::Point),
        StoredAttributeSemantic::Normal if attribute.lanes == 3 => Some(SpatialTuple::Direction),
        StoredAttributeSemantic::Tangent if attribute.lanes == 4 => Some(SpatialTuple::Tangent),
        StoredAttributeSemantic::Position
        | StoredAttributeSemantic::Normal
        | StoredAttributeSemantic::Tangent => return None,
        _ => None,
    };
    (attribute.lanes != 0).then_some(spatial)
}

fn attribute_work(source: &GeometryStorage, attribute: StoredAttribute) -> Option<u64> {
    let lanes = usize::from(attribute.lanes);
    if lanes == 0 {
        return None;
    }
    macro_rules! dense {
        ($range:expr, $pool:ident) => {{
            $range.get(&source.$pool)?;
            ($range.len() == (attribute.element_count as usize).checked_mul(lanes)?)
                .then_some(u64::from(attribute.element_count))
        }};
    }
    macro_rules! sparse {
        ($indices:expr, $values:expr, $default:expr, $pool:ident) => {{
            $indices.get(&source.u32_values)?;
            $values.get(&source.$pool)?;
            $default.get(&source.$pool)?;
            if $indices.len().checked_mul(lanes)? != $values.len() || $default.len() != lanes {
                return None;
            }
            u64::try_from($indices.len())
                .ok()?
                .checked_mul(2)?
                .checked_add(1)
        }};
    }
    match attribute.payload {
        StoredAttributePayload::DenseF32(range) => dense!(range, f32_values),
        StoredAttributePayload::DenseF64(range) => dense!(range, f64_values),
        StoredAttributePayload::DenseU32(range) => dense!(range, u32_values),
        StoredAttributePayload::DenseI32(range) => dense!(range, i32_values),
        StoredAttributePayload::SparseF32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, f32_values),
        StoredAttributePayload::SparseU32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, u32_values),
        StoredAttributePayload::SparseI32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, i32_values),
    }
}

fn convert_tuple(
    conversion: SpaceConversion,
    spatial: SpatialTuple,
    tuple: &[f32],
) -> Result<[f32; 4], NumericConversionError> {
    let xyz = [tuple[0], tuple[1], tuple[2]];
    let xyz = match spatial {
        SpatialTuple::Point => conversion.point(xyz)?,
        _ => conversion.direction(xyz)?,
    };
    let w = if matches!(spatial, SpatialTuple::Tangent) {
        if conversion.determinant_sign() < 0 {
            -tuple[3]
        } else {
            tuple[3]
        }
    } else {
        0.0
    };
    Ok([xyz[0], xyz[1], xyz[2], w])
}

fn corner_ordinal(corner: usize, remap: bool) -> Option<usize> {
    if !remap {
        return Some(corner);
    }
    match corner % 3 {
        1 => corner.checked_add(1),
        2 => corner.checked_sub(1),
        _ => Some(corner),
    }
}

/// Only two adjacent explicit entries within one triangle can exchange sorted order.
fn sparse_ordinal(ids: &[u32], ordinal: usize, remap: bool) -> Option<usize> {
    let id = *ids.get(ordinal)?;
    if remap {
        if id % 3 == 1 && ids.get(ordinal.checked_add(1)?) == id.checked_add(1).as_ref() {
            return ordinal.checked_add(1);
        }
        if id % 3 == 2
            && ordinal != 0
            && ids.get(ordinal - 1).and_then(|id| id.checked_add(1)) == Some(id)
        {
            return Some(ordinal - 1);
        }
    }
    Some(ordinal)
}

#[derive(Clone, Copy)]
struct TupleMap<'a> {
    indices: Option<&'a [u32]>,
    lanes: usize,
    remap: bool,
}
impl TupleMap<'_> {
    fn get<T>(self, values: &[T], ordinal: usize) -> Option<&[T]> {
        let origin = match self.indices {
            Some(ids) => sparse_ordinal(ids, ordinal, self.remap)?,
            None => corner_ordinal(ordinal, self.remap)?,
        };
        let start = origin.checked_mul(self.lanes)?;
        values.get(start..start.checked_add(self.lanes)?)
    }
}

fn room<T, E>(values: &Vec<T>, capacity: CapacityPlan, count: usize) -> BuildResult<(), E> {
    let end = values
        .len()
        .checked_add(count)
        .ok_or(TypedStorageBuildError::Invariant)?;
    if end > capacity.desired_capacity() || end > values.capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    Ok(())
}

fn copy_tuples<T: Copy, C: TypedStorageBuildControl>(
    values: &[T],
    indices: Option<&[u32]>,
    lanes: usize,
    remap: bool,
    output: &mut Vec<T>,
    capacity: CapacityPlan,
    control: &mut C,
) -> BuildResult<TableRange<T>, C::Error> {
    let start = output.len();
    let map = TupleMap {
        indices,
        lanes,
        remap,
    };
    for ordinal in 0..values.len() / lanes {
        room(output, capacity, lanes)?;
        control
            .run_unit(|| {
                output.extend_from_slice(
                    map.get(values, ordinal)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                );
                Ok(())
            })
            .map_err(TypedStorageBuildError::Control)??;
    }
    Ok(TableRange::new(start, values.len()))
}

fn copy_f32_tuples<C: TypedStorageBuildControl>(
    values: &[f32],
    map: TupleMap<'_>,
    spatial: Option<SpatialTuple>,
    conversion: SpaceConversion,
    output: &mut Vec<f32>,
    capacity: CapacityPlan,
    control: &mut C,
) -> BuildResult<TableRange<f32>, C::Error> {
    let start = output.len();
    for ordinal in 0..values.len() / map.lanes {
        room(output, capacity, map.lanes)?;
        control
            .run_unit(|| {
                let tuple = map
                    .get(values, ordinal)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                match spatial {
                    Some(spatial) => output.extend_from_slice(
                        &convert_tuple(conversion, spatial, tuple)
                            .map_err(|_| TypedStorageBuildError::Invariant)?[..map.lanes],
                    ),
                    None => output.extend_from_slice(tuple),
                }
                Ok(())
            })
            .map_err(TypedStorageBuildError::Control)??;
    }
    Ok(TableRange::new(start, values.len()))
}

fn copy_sparse_indices<C: TypedStorageBuildControl>(
    ids: &[u32],
    remap: bool,
    output: &mut Vec<u32>,
    capacity: CapacityPlan,
    control: &mut C,
) -> BuildResult<TableRange<u32>, C::Error> {
    let start = output.len();
    for ordinal in 0..ids.len() {
        room(output, capacity, 1)?;
        control
            .run_unit(|| {
                let source =
                    sparse_ordinal(ids, ordinal, remap).ok_or(TypedStorageBuildError::Invariant)?;
                let id = corner_ordinal(ids[source] as usize, remap)
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or(TypedStorageBuildError::Invariant)?;
                output.push(id);
                Ok(())
            })
            .map_err(TypedStorageBuildError::Control)??;
    }
    Ok(TableRange::new(start, ids.len()))
}

fn compare_sparse_indices<C: TypedStorageValidationControl>(
    source: &[u32],
    actual: &[u32],
    remap: bool,
    control: &mut C,
) -> Result<bool, C::Error> {
    if source.len() != actual.len() {
        return Ok(false);
    }
    for (ordinal, &actual) in actual.iter().enumerate() {
        if control.run_unit(|| {
            sparse_ordinal(source, ordinal, remap)
                .and_then(|ordinal| corner_ordinal(source[ordinal] as usize, remap))
                .and_then(|value| u32::try_from(value).ok())
                == Some(actual)
        })? != Some(true)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn compare_f32_tuples<C: TypedStorageValidationControl>(
    source: &[f32],
    actual: &[f32],
    map: TupleMap<'_>,
    spatial: Option<SpatialTuple>,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, C::Error> {
    for (ordinal, tuple) in actual.chunks_exact(map.lanes).enumerate() {
        if control.run_unit(|| {
            let Some(expected) = map.get(source, ordinal) else {
                return false;
            };
            match spatial {
                Some(spatial) => {
                    convert_tuple(conversion, spatial, expected).is_ok_and(|expected| {
                        expected[..map.lanes]
                            .iter()
                            .map(|value| value.to_bits())
                            .eq(tuple.iter().map(|value| value.to_bits()))
                    })
                }
                None => expected
                    .iter()
                    .map(|value| value.to_bits())
                    .eq(tuple.iter().map(|value| value.to_bits())),
            }
        })? != Some(true)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn take_range<T>(cursor: &mut usize, count: usize) -> Option<TableRange<T>> {
    let range = TableRange::new(*cursor, count);
    *cursor = cursor.checked_add(count)?;
    Some(range)
}
fn expected_payload(
    source: StoredAttributePayload,
    pools: &mut [usize; 4],
) -> Option<StoredAttributePayload> {
    Some(match source {
        StoredAttributePayload::DenseF32(range) => {
            StoredAttributePayload::DenseF32(take_range(&mut pools[0], range.len())?)
        }
        StoredAttributePayload::DenseF64(range) => {
            StoredAttributePayload::DenseF64(take_range(&mut pools[1], range.len())?)
        }
        StoredAttributePayload::DenseU32(range) => {
            StoredAttributePayload::DenseU32(take_range(&mut pools[2], range.len())?)
        }
        StoredAttributePayload::DenseI32(range) => {
            StoredAttributePayload::DenseI32(take_range(&mut pools[3], range.len())?)
        }
        StoredAttributePayload::SparseF32 {
            indices,
            values,
            default,
        } => StoredAttributePayload::SparseF32 {
            indices: take_range(&mut pools[2], indices.len())?,
            values: take_range(&mut pools[0], values.len())?,
            default: take_range(&mut pools[0], default.len())?,
        },
        StoredAttributePayload::SparseU32 {
            indices,
            values,
            default,
        } => StoredAttributePayload::SparseU32 {
            indices: take_range(&mut pools[2], indices.len())?,
            values: take_range(&mut pools[2], values.len())?,
            default: take_range(&mut pools[2], default.len())?,
        },
        StoredAttributePayload::SparseI32 {
            indices,
            values,
            default,
        } => StoredAttributePayload::SparseI32 {
            indices: take_range(&mut pools[2], indices.len())?,
            values: take_range(&mut pools[3], values.len())?,
            default: take_range(&mut pools[3], default.len())?,
        },
    })
}
fn same_payload_ranges(left: StoredAttributePayload, right: StoredAttributePayload) -> bool {
    match (left, right) {
        (StoredAttributePayload::DenseF32(a), StoredAttributePayload::DenseF32(b)) => a == b,
        (StoredAttributePayload::DenseF64(a), StoredAttributePayload::DenseF64(b)) => a == b,
        (StoredAttributePayload::DenseU32(a), StoredAttributePayload::DenseU32(b)) => a == b,
        (StoredAttributePayload::DenseI32(a), StoredAttributePayload::DenseI32(b)) => a == b,
        (
            StoredAttributePayload::SparseF32 {
                indices: ai,
                values: av,
                default: ad,
            },
            StoredAttributePayload::SparseF32 {
                indices: bi,
                values: bv,
                default: bd,
            },
        ) => ai == bi && av == bv && ad == bd,
        (
            StoredAttributePayload::SparseU32 {
                indices: ai,
                values: av,
                default: ad,
            },
            StoredAttributePayload::SparseU32 {
                indices: bi,
                values: bv,
                default: bd,
            },
        ) => ai == bi && av == bv && ad == bd,
        (
            StoredAttributePayload::SparseI32 {
                indices: ai,
                values: av,
                default: ad,
            },
            StoredAttributePayload::SparseI32 {
                indices: bi,
                values: bv,
                default: bd,
            },
        ) => ai == bi && av == bv && ad == bd,
        _ => false,
    }
}
fn same_lengths(storage: &GeometryStorage, capacities: [CapacityPlan; 9]) -> bool {
    [
        storage.logical_meshes.len(),
        storage.primitives.len(),
        storage.attributes.len(),
        storage.mesh_instances.len(),
        storage.names.len(),
        storage.f32_values.len(),
        storage.f64_values.len(),
        storage.u32_values.len(),
        storage.i32_values.len(),
    ]
    .into_iter()
    .zip(capacities)
    .all(|(len, capacity)| len == capacity.desired_capacity())
}

#[cfg(test)]
mod tests;
