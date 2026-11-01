//! One source-bound typed geometry replacement, independent of Scene publication authority.
//!
//! The caller must admit all nine capacities before invoking `materialize`.

use baozi_operation::{CapacityPlan, CapacityPlanError};

use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;

use super::build::{TypedStorageBuildControl, TypedStorageBuildError};
use super::{
    GeometryStorage, StoredAttribute, StoredAttributePayload, StoredLogicalMesh,
    StoredMeshInstance, StoredPrimitive, TableRange,
};

pub(in crate::canonical_scene::store) mod normals;
pub(in crate::canonical_scene::store) mod space;
pub(in crate::canonical_scene::store) mod space_animation;
pub(in crate::canonical_scene::store) mod space_appearance;
pub(in crate::canonical_scene::store) mod space_deformation;
pub(in crate::canonical_scene::store) mod space_geometry;
pub(in crate::canonical_scene::store) mod space_nodes;
pub(in crate::canonical_scene::store) mod space_spatial;
pub(in crate::canonical_scene::store) mod tangents;

type EditResult<T, E> = Result<T, TypedStorageBuildError<E>>;

/// Borrowed generated vectors at their selected output width.
#[derive(Debug, Clone, Copy)]
pub enum VectorValues<'a, const N: usize> {
    F32(&'a [[f32; N]]),
    F64(&'a [[f64; N]]),
}

impl<const N: usize> VectorValues<'_, N> {
    pub const fn len(self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub const fn is_f64(self) -> bool {
        matches!(self, Self::F64(_))
    }

    fn get(self, index: usize) -> Option<[f64; N]> {
        match self {
            Self::F32(values) => values.get(index).map(|value| value.map(f64::from)),
            Self::F64(values) => values.get(index).copied(),
        }
    }

    const fn unit_length_tolerance(self) -> f64 {
        if self.is_f64() {
            8.0 * f64::EPSILON
        } else {
            4.0 * f32::EPSILON as f64
        }
    }

    fn append<C: TypedStorageBuildControl>(
        self,
        output: &mut GeometryStorage,
        capacities: [CapacityPlan; 9],
        control: &mut C,
    ) -> EditResult<StoredAttributePayload, C::Error> {
        match self {
            Self::F32(values) => append(
                &mut output.f32_values,
                capacities[5],
                values.as_flattened(),
                control,
            )
            .map(StoredAttributePayload::DenseF32),
            Self::F64(values) => append(
                &mut output.f64_values,
                capacities[6],
                values.as_flattened(),
                control,
            )
            .map(StoredAttributePayload::DenseF64),
        }
    }
}

/// Copies the text owned by exactly one descriptor and updates only its text ranges.
/// Callers rebuild names in attribute order, followed by logical-mesh names.
pub(in crate::canonical_scene::store) fn copy_attribute_names<C: TypedStorageBuildControl>(
    source: &GeometryStorage,
    mut attribute: StoredAttribute,
    names: &mut Vec<u8>,
    plan: CapacityPlan,
    control: &mut C,
) -> EditResult<StoredAttribute, C::Error> {
    use super::{StoredAttributeSemantic, StoredTangentBasis, StoredTangentEncoding, TextRange};
    let mut copy = |range: TextRange| -> EditResult<TextRange, C::Error> {
        let text = range
            .get(&source.names)
            .ok_or(TypedStorageBuildError::Invariant)?;
        super::build::append_name(
            names,
            plan,
            crate::canonical_scene::store::production::Presence::Present(text),
            control,
        )?
        .ok_or(TypedStorageBuildError::Invariant)
    };
    if let StoredAttributeSemantic::Custom(name) = attribute.semantic {
        attribute.semantic = StoredAttributeSemantic::Custom(copy(name)?);
    }
    if let Some(conventions) = &mut attribute.tangent_conventions {
        if let StoredTangentBasis::Namespaced { name, .. } = &mut conventions.basis {
            *name = copy(*name)?;
        }
        if let StoredTangentEncoding::Namespaced { name, .. } = &mut conventions.encoding {
            *name = copy(*name)?;
        }
    }
    Ok(attribute)
}

/// Compares inline declaration kinds and versions; schema bytes are compared separately.
pub(in crate::canonical_scene::store) fn same_tangent_convention_kinds(
    source: Option<super::StoredTangentConventions>,
    actual: Option<super::StoredTangentConventions>,
) -> bool {
    use super::{StoredTangentBasis as B, StoredTangentEncoding as E};
    match (source, actual) {
        (None, None) => true,
        (Some(source), Some(actual)) => {
            let basis = match (source.basis, actual.basis) {
                (B::Namespaced { version: left, .. }, B::Namespaced { version: right, .. }) => {
                    left == right
                }
                (left, right) => left == right,
            };
            let encoding = match (source.encoding, actual.encoding) {
                (E::Namespaced { version: left, .. }, E::Namespaced { version: right, .. }) => {
                    left == right
                }
                (left, right) => left == right,
            };
            basis && encoding
        }
        _ => false,
    }
}

/// Compares schema text by borrowing both geometry name pools, one metered action per byte.
pub(in crate::canonical_scene::store) fn compare_tangent_schema_names<
    C: super::validation::TypedStorageValidationControl,
>(
    source: Option<super::StoredTangentConventions>,
    actual: Option<super::StoredTangentConventions>,
    source_names: &[u8],
    actual_names: &[u8],
    control: &mut C,
) -> Result<bool, C::Error> {
    if !same_tangent_convention_kinds(source, actual) {
        return Ok(false);
    }
    if let (Some(source), Some(actual)) = (source, actual) {
        for (source, actual) in source.schema_names().zip(actual.schema_names()) {
            let (Some(source), Some(actual)) =
                (source.0.get(source_names), actual.0.get(actual_names))
            else {
                return Ok(false);
            };
            if source.len() != actual.len() {
                return Ok(false);
            }
            for (&source, &actual) in source.iter().zip(actual) {
                if control.run_unit(|| source == actual)? != Some(true) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Primitive-local source corners, grouped into triangles in source-face order.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct TriangulationOrigins<'a> {
    pub(in crate::canonical_scene::store) triangles: &'a [[u32; 3]],
    pub(in crate::canonical_scene::store) face_offsets: &'a [u32],
}

/// A sealed replacement borrows the exact immutable source used to count its capacities.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryTriangulationPlan<'a> {
    source: &'a GeometryStorage,
    target: usize,
    origins: TriangulationOrigins<'a>,
    capacities: [CapacityPlan; 9],
    copy_work: u64,
    lookup_work: u64,
    validation_work: u64,
    correspondence_work: u64,
    triangles: u32,
    corners: u32,
}

#[derive(Default)]
struct ScalarCounts {
    f32s: usize,
    f64s: usize,
    u32s: usize,
    i32s: usize,
}

impl<'a> GeometryTriangulationPlan<'a> {
    /// Runs in the host's admitted semantic scope. Origin checks and sparse lookups are metered.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        origins: TriangulationOrigins<'a>,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let primitive = source
            .primitives
            .get(target)
            .ok_or(TypedStorageBuildError::Invariant)?;
        validate_origins(source, primitive, origins, control)?;
        let triangles = u32::try_from(origins.triangles.len()).map_err(|_| overflow())?;
        let corners = triangles.checked_mul(3).ok_or_else(overflow)?;
        let attributes = primitive
            .attributes
            .checked(source.attributes.len())
            .ok_or(TypedStorageBuildError::Invariant)?;
        let mut scalars = ScalarCounts::default();
        let mut lookup_work = 0_u64;
        let mut correspondence_work = u64::try_from(source.names.len()).map_err(|_| overflow())?;
        for (index, attribute) in source.attributes.iter().enumerate() {
            let remapped = control
                .run_unit(|| {
                    attributes.contains(&index)
                        && matches!(
                            attribute.domain,
                            AttributeDomain::Corner | AttributeDomain::Face
                        )
                })
                .map_err(TypedStorageBuildError::Control)?;
            let mut gather = Gather::new(*attribute, remapped, origins, corners, triangles);
            let lanes = usize::from(attribute.lanes);
            if lanes == 0 {
                return Err(TypedStorageBuildError::Invariant);
            }
            let count = gather.output_count as usize;
            let mut correspondence_actions = 0_u64;
            let correspondence_payload_work;
            macro_rules! sparse_count {
                ($indices:expr, $values:expr, $default:expr, $pool:ident, $count:ident) => {{
                    let indices = $indices
                        .get(&source.u32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    let values = $values
                        .get(&source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    let default = $default
                        .get(&source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    if indices.len().checked_mul(lanes) != Some(values.len())
                        || default.len() != lanes
                    {
                        return Err(TypedStorageBuildError::Invariant);
                    }
                    let explicit = if remapped {
                        let mut explicit = 0_usize;
                        for output in 0..count {
                            let source = control
                                .run_unit(|| gather.source(output))
                                .map_err(TypedStorageBuildError::Control)?
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            let (found, comparisons) = sparse_lookup(indices, source, control)?;
                            // Indices and values use separate passes, including when their
                            // destination is the same u32 pool. Both passes perform this lookup.
                            let actions = comparisons
                                .checked_add(1)
                                .and_then(|work| work.checked_mul(2))
                                .ok_or_else(overflow)?;
                            add_work(&mut lookup_work, actions)?;
                            // Correspondence repeats these same source lookups. Central
                            // validation requires an exact action count, not a worst-case bound.
                            add_work(&mut correspondence_actions, actions)?;
                            if found.is_some() {
                                add_count(&mut explicit, 1)?;
                            }
                        }
                        explicit
                    } else {
                        indices.len()
                    };
                    let explicit_u64 = u64::try_from(explicit).map_err(|_| overflow())?;
                    let lanes_u64 = u64::try_from(lanes).map_err(|_| overflow())?;
                    correspondence_payload_work = if remapped {
                        explicit_u64
                            .checked_mul(lanes_u64)
                            .and_then(|work| work.checked_add(explicit_u64))
                    } else {
                        u64::try_from(indices.len())
                            .ok()
                            .and_then(|work| work.checked_add(u64::try_from(values.len()).ok()?))
                    }
                    .and_then(|work| work.checked_add(lanes_u64))
                    .ok_or_else(overflow)?;
                    add_count(&mut scalars.u32s, explicit)?;
                    let values = explicit.checked_mul(lanes).ok_or_else(overflow)?;
                    add_count(&mut scalars.$count, values)?;
                    add_count(&mut scalars.$count, lanes)?;
                }};
            }
            match attribute.payload {
                StoredAttributePayload::DenseF32(range) => {
                    validate_dense_range(range, &source.f32_values, *attribute)?;
                    add_count(
                        &mut scalars.f32s,
                        count.checked_mul(lanes).ok_or_else(overflow)?,
                    )?;
                    if remapped {
                        let count_u64 = u64::try_from(count).map_err(|_| overflow())?;
                        let lanes_u64 = u64::try_from(lanes).map_err(|_| overflow())?;
                        add_work(&mut lookup_work, count_u64)?;
                        correspondence_actions = count_u64;
                        correspondence_payload_work =
                            count_u64.checked_mul(lanes_u64).ok_or_else(overflow)?;
                    } else {
                        correspondence_payload_work =
                            u64::try_from(count.checked_mul(lanes).ok_or_else(overflow)?)
                                .map_err(|_| overflow())?;
                    }
                }
                StoredAttributePayload::DenseF64(range) => {
                    validate_dense_range(range, &source.f64_values, *attribute)?;
                    add_count(
                        &mut scalars.f64s,
                        count.checked_mul(lanes).ok_or_else(overflow)?,
                    )?;
                    if remapped {
                        let count_u64 = u64::try_from(count).map_err(|_| overflow())?;
                        let lanes_u64 = u64::try_from(lanes).map_err(|_| overflow())?;
                        add_work(&mut lookup_work, count_u64)?;
                        correspondence_actions = count_u64;
                        correspondence_payload_work =
                            count_u64.checked_mul(lanes_u64).ok_or_else(overflow)?;
                    } else {
                        correspondence_payload_work =
                            u64::try_from(count.checked_mul(lanes).ok_or_else(overflow)?)
                                .map_err(|_| overflow())?;
                    }
                }
                StoredAttributePayload::DenseU32(range) => {
                    validate_dense_range(range, &source.u32_values, *attribute)?;
                    add_count(
                        &mut scalars.u32s,
                        count.checked_mul(lanes).ok_or_else(overflow)?,
                    )?;
                    if remapped {
                        let count_u64 = u64::try_from(count).map_err(|_| overflow())?;
                        let lanes_u64 = u64::try_from(lanes).map_err(|_| overflow())?;
                        add_work(&mut lookup_work, count_u64)?;
                        correspondence_actions = count_u64;
                        correspondence_payload_work =
                            count_u64.checked_mul(lanes_u64).ok_or_else(overflow)?;
                    } else {
                        correspondence_payload_work =
                            u64::try_from(count.checked_mul(lanes).ok_or_else(overflow)?)
                                .map_err(|_| overflow())?;
                    }
                }
                StoredAttributePayload::DenseI32(range) => {
                    validate_dense_range(range, &source.i32_values, *attribute)?;
                    add_count(
                        &mut scalars.i32s,
                        count.checked_mul(lanes).ok_or_else(overflow)?,
                    )?;
                    let count_u64 = u64::try_from(count).map_err(|_| overflow())?;
                    let lanes_u64 = u64::try_from(lanes).map_err(|_| overflow())?;
                    if remapped {
                        add_work(&mut lookup_work, count_u64)?;
                        correspondence_actions = count_u64;
                    }
                    correspondence_payload_work =
                        count_u64.checked_mul(lanes_u64).ok_or_else(overflow)?;
                }
                StoredAttributePayload::SparseF32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse_count!(indices, values, default, f32_values, f32s);
                }
                StoredAttributePayload::SparseF64 {
                    indices,
                    values,
                    default,
                } => {
                    sparse_count!(indices, values, default, f64_values, f64s);
                }
                StoredAttributePayload::SparseU32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse_count!(indices, values, default, u32_values, u32s);
                }
                StoredAttributePayload::SparseI32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse_count!(indices, values, default, i32_values, i32s);
                }
            }
            let attribute_work = correspondence_actions
                .checked_add(1)
                .and_then(|work| work.checked_add(correspondence_payload_work))
                .ok_or_else(overflow)?;
            add_work(&mut correspondence_work, attribute_work)?;
        }
        for (index, primitive) in source.primitives.iter().enumerate() {
            let u32_count = control
                .run_unit(|| {
                    if index == target {
                        return Ok(corners as usize);
                    }
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
            add_count(&mut scalars.u32s, u32_count)?;
            add_work(&mut correspondence_work, 1)?;
            if index == target {
                add_work(&mut correspondence_work, u64::from(corners))?;
            } else {
                for range in [primitive.polygon_boundaries, primitive.indices]
                    .into_iter()
                    .flatten()
                {
                    add_work(
                        &mut correspondence_work,
                        u64::try_from(range.len()).map_err(|_| overflow())?,
                    )?;
                }
            }
        }
        add_work(&mut correspondence_work, source.logical_meshes.len() as u64)?;
        add_work(&mut correspondence_work, source.mesh_instances.len() as u64)?;
        let capacities = [
            capacity::<StoredLogicalMesh, C::Error>(source.logical_meshes.len())?,
            capacity::<StoredPrimitive, C::Error>(source.primitives.len())?,
            capacity::<StoredAttribute, C::Error>(source.attributes.len())?,
            capacity::<StoredMeshInstance, C::Error>(source.mesh_instances.len())?,
            capacity::<u8, C::Error>(source.names.len())?,
            capacity::<f32, C::Error>(scalars.f32s)?,
            capacity::<f64, C::Error>(scalars.f64s)?,
            capacity::<u32, C::Error>(scalars.u32s)?,
            capacity::<i32, C::Error>(scalars.i32s)?,
        ];
        let copy_work = capacities.iter().try_fold(0_u64, |sum, plan| {
            sum.checked_add(plan.desired_capacity() as u64)
                .ok_or_else(overflow)
        })?;
        let validation_work = [
            source.logical_meshes.len(),
            source.primitives.len(),
            source.primitives.len(),
            source.attributes.len(),
            source.mesh_instances.len(),
            scalars.f32s,
            scalars.f64s,
            scalars.u32s,
            scalars.i32s,
        ]
        .into_iter()
        .try_fold(0_u64, |sum, count| {
            sum.checked_add(count as u64).ok_or_else(overflow)
        })?;
        copy_work.checked_add(lookup_work).ok_or_else(overflow)?;
        Ok(Self {
            source,
            target,
            origins,
            capacities,
            copy_work,
            lookup_work,
            validation_work,
            correspondence_work,
            triangles,
            corners,
        })
    }

    pub(in crate::canonical_scene::store) const fn target_row(&self) -> usize {
        self.target
    }

    pub(in crate::canonical_scene::store) const fn capacities(&self) -> [CapacityPlan; 9] {
        self.capacities
    }

    pub(in crate::canonical_scene::store) const fn work(&self) -> u64 {
        // The checked sum was established by try_new.
        self.copy_work + self.lookup_work
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.validation_work
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(&self) -> u64 {
        self.correspondence_work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(&self) -> usize {
        self.source.names.len()
    }

    pub(in crate::canonical_scene::store) const fn output_face_count(&self) -> u32 {
        self.triangles
    }

    pub(in crate::canonical_scene::store) const fn output_corner_count(&self) -> u32 {
        self.corners
    }

    /// The central lane admits these exact capacities before this method may allocate.
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
        // No descriptor name changes, so the complete name pool keeps its original ranges.
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
        let target_attributes = self.source.primitives[self.target]
            .attributes
            .checked(self.source.attributes.len())
            .ok_or(TypedStorageBuildError::Invariant)?;
        for (index, attribute) in self.source.attributes.iter().copied().enumerate() {
            let remapped = target_attributes.contains(&index)
                && matches!(
                    attribute.domain,
                    AttributeDomain::Corner | AttributeDomain::Face
                );
            let mut gather = Gather::new(
                attribute,
                remapped,
                self.origins,
                self.corners,
                self.triangles,
            );
            let lanes = usize::from(attribute.lanes);
            macro_rules! dense {
                ($range:expr, $pool:ident, $capacity:expr, $variant:ident) => {{
                    let source = $range
                        .get(&self.source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    StoredAttributePayload::$variant(if remapped {
                        gather_dense(
                            &mut output.$pool,
                            $capacity,
                            source,
                            lanes,
                            &mut gather,
                            control,
                        )?
                    } else {
                        append(&mut output.$pool, $capacity, source, control)?
                    })
                }};
            }
            macro_rules! sparse {
                ($indices:expr, $values:expr, $default:expr, $pool:ident, $capacity:expr, $variant:ident) => {{
                    let source_indices = $indices
                        .get(&self.source.u32_values)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    let source_values = $values
                        .get(&self.source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    let default_values = $default
                        .get(&self.source.$pool)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    // The u32 scalar pool can also hold both sparse indices and values. Build each
                    // range sequentially, never holding two mutable borrows of that pool.
                    let index_start = output.u32_values.len();
                    if remapped {
                        for destination in 0..gather.output_count as usize {
                            let source = control
                                .run_unit(|| gather.source(destination))
                                .map_err(TypedStorageBuildError::Control)?
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            if sparse_lookup(source_indices, source, control)?.0.is_some() {
                                push(&mut output.u32_values, u32s, destination as u32, control)?;
                            }
                        }
                    } else {
                        append(&mut output.u32_values, u32s, source_indices, control)?;
                    }
                    let indices =
                        TableRange::new(index_start, output.u32_values.len() - index_start);
                    let value_start = output.$pool.len();
                    if remapped {
                        // Reuse increasing output sparse indices; the original lookups determine
                        // explicitness even when the stored value equals its default.
                        let mut gather = Gather::new(
                            attribute,
                            true,
                            self.origins,
                            self.corners,
                            self.triangles,
                        );
                        for destination in 0..gather.output_count as usize {
                            let source = control
                                .run_unit(|| gather.source(destination))
                                .map_err(TypedStorageBuildError::Control)?
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            if let Some(explicit) =
                                sparse_lookup(source_indices, source, control)?.0
                            {
                                let start = explicit.checked_mul(lanes).ok_or_else(overflow)?;
                                let end = start.checked_add(lanes).ok_or_else(overflow)?;
                                let values = source_values
                                    .get(start..end)
                                    .ok_or(TypedStorageBuildError::Invariant)?;
                                append(&mut output.$pool, $capacity, values, control)?;
                            }
                        }
                    } else {
                        append(&mut output.$pool, $capacity, source_values, control)?;
                    }
                    let values = TableRange::new(value_start, output.$pool.len() - value_start);
                    let default = append(&mut output.$pool, $capacity, default_values, control)?;
                    StoredAttributePayload::$variant {
                        indices,
                        values,
                        default,
                    }
                }};
            }
            let payload = match attribute.payload {
                StoredAttributePayload::DenseF32(range) => {
                    dense!(range, f32_values, f32s, DenseF32)
                }
                StoredAttributePayload::DenseF64(range) => {
                    dense!(range, f64_values, f64s, DenseF64)
                }
                StoredAttributePayload::DenseU32(range) => {
                    dense!(range, u32_values, u32s, DenseU32)
                }
                StoredAttributePayload::DenseI32(range) => {
                    dense!(range, i32_values, i32s, DenseI32)
                }
                StoredAttributePayload::SparseF32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse!(indices, values, default, f32_values, f32s, SparseF32)
                }
                StoredAttributePayload::SparseF64 {
                    indices,
                    values,
                    default,
                } => {
                    sparse!(indices, values, default, f64_values, f64s, SparseF64)
                }
                StoredAttributePayload::SparseU32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse!(indices, values, default, u32_values, u32s, SparseU32)
                }
                StoredAttributePayload::SparseI32 {
                    indices,
                    values,
                    default,
                } => {
                    sparse!(indices, values, default, i32_values, i32s, SparseI32)
                }
            };
            push(
                &mut output.attributes,
                attributes,
                StoredAttribute {
                    element_count: gather.output_count,
                    payload,
                    ..attribute
                },
                control,
            )?;
        }
        for (index, primitive) in self.source.primitives.iter().copied().enumerate() {
            let row = if index == self.target {
                let start = output.u32_values.len();
                for triangle in self.origins.triangles {
                    for &corner in triangle {
                        let vertex = match primitive.indices {
                            Some(indices) => *indices
                                .get(&self.source.u32_values)
                                .and_then(|values| values.get(corner as usize))
                                .ok_or(TypedStorageBuildError::Invariant)?,
                            None => corner,
                        };
                        push(&mut output.u32_values, u32s, vertex, control)?;
                    }
                }
                StoredPrimitive {
                    topology: TopologyKind::TriangleList,
                    corner_count: self.corners,
                    face_count: self.triangles,
                    polygon_boundaries: None,
                    indices: Some(TableRange::new(start, self.corners as usize)),
                    ..primitive
                }
            } else {
                StoredPrimitive {
                    polygon_boundaries: copy_optional(
                        primitive.polygon_boundaries,
                        &self.source.u32_values,
                        &mut output.u32_values,
                        u32s,
                        control,
                    )?,
                    indices: copy_optional(
                        primitive.indices,
                        &self.source.u32_values,
                        &mut output.u32_values,
                        u32s,
                        control,
                    )?,
                    ..primitive
                }
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
        macro_rules! finish {
            ($values:ident, $capacity:expr) => {
                if output.$values.len() != $capacity.desired_capacity() {
                    return Err(TypedStorageBuildError::Invariant);
                }
                $capacity
                    .finish(0, output.$values.capacity())
                    .map_err(TypedStorageBuildError::Capacity)?;
            };
        }
        finish!(logical_meshes, meshes);
        finish!(primitives, primitives);
        finish!(attributes, attributes);
        finish!(mesh_instances, instances);
        finish!(names, names);
        finish!(f32_values, f32s);
        finish!(f64_values, f64s);
        finish!(u32_values, u32s);
        finish!(i32_values, i32s);
        Ok(output)
    }
}

impl GeometryTriangulationPlan<'_> {
    /// Compare the actual retained destination with the exact source/origin relation. This uses
    /// the same checked action count as construction, without constructing a second table.
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: super::validation::TypedStorageValidationControl,
    >(
        &self,
        output: &GeometryStorage,
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
            output.logical_meshes.len(),
            output.primitives.len(),
            output.attributes.len(),
            output.mesh_instances.len(),
            output.names.len(),
            output.f32_values.len(),
            output.f64_values.len(),
            output.u32_values.len(),
            output.i32_values.len(),
        ] != self.capacities.map(|capacity| capacity.desired_capacity())
        {
            return Ok(false);
        }
        for (&actual, &expected) in output.names.iter().zip(&self.source.names) {
            check!(actual == expected);
        }
        let Some(target_attributes) = self.source.primitives[self.target]
            .attributes
            .checked(self.source.attributes.len())
        else {
            return Ok(false);
        };
        for (index, (&source, actual)) in self
            .source
            .attributes
            .iter()
            .zip(&output.attributes)
            .enumerate()
        {
            let remapped = target_attributes.contains(&index)
                && matches!(
                    source.domain,
                    AttributeDomain::Corner | AttributeDomain::Face
                );
            let mut gather =
                Gather::new(source, remapped, self.origins, self.corners, self.triangles);
            let lanes = source.lanes as usize;
            check!(
                actual.semantic == source.semantic
                    && actual.tangent_conventions == source.tangent_conventions
                    && actual.domain == source.domain
                    && actual.lanes == source.lanes
                    && actual.element_count == gather.output_count
            );
            macro_rules! dense {
                ($sr:expr, $ar:expr, $pool:ident, $eq:expr) => {{
                    let (Some(source_values), Some(actual_values)) =
                        ($sr.get(&self.source.$pool), $ar.get(&output.$pool))
                    else {
                        return Ok(false);
                    };
                    if actual_values.len() != gather.output_count as usize * lanes {
                        return Ok(false);
                    }
                    for destination in 0..gather.output_count as usize {
                        let origin = if remapped {
                            let Some(Some(origin)) =
                                control.run_unit(|| gather.source(destination))?
                            else {
                                return Ok(false);
                            };
                            origin
                        } else {
                            destination
                        };
                        for lane in 0..lanes {
                            check!(
                                source_values
                                    .get(origin * lanes + lane)
                                    .zip(actual_values.get(destination * lanes + lane))
                                    .is_some_and(|(&source, &actual)| ($eq)(source, actual))
                            );
                        }
                    }
                }};
            }
            macro_rules! sparse {
                ($si:expr, $sv:expr, $sd:expr, $ai:expr, $av:expr, $ad:expr, $pool:ident, $eq:expr) => {{
                    let (
                        Some(source_indices),
                        Some(source_values),
                        Some(source_default),
                        Some(actual_indices),
                        Some(actual_values),
                        Some(actual_default),
                    ) = (
                        $si.get(&self.source.u32_values),
                        $sv.get(&self.source.$pool),
                        $sd.get(&self.source.$pool),
                        $ai.get(&output.u32_values),
                        $av.get(&output.$pool),
                        $ad.get(&output.$pool),
                    )
                    else {
                        return Ok(false);
                    };
                    if remapped {
                        let mut explicit = 0;
                        for destination in 0..gather.output_count as usize {
                            let Some(Some(origin)) =
                                control.run_unit(|| gather.source(destination))?
                            else {
                                return Ok(false);
                            };
                            let (valid, found) =
                                correspondence_lookup(source_indices, origin, control)?;
                            if !valid {
                                return Ok(false);
                            }
                            if found.is_some() {
                                check!(
                                    actual_indices.get(explicit).copied()
                                        == Some(destination as u32)
                                );
                                explicit += 1;
                            }
                        }
                        if actual_indices.len() != explicit
                            || actual_values.len() != explicit * lanes
                        {
                            return Ok(false);
                        }
                        let mut gather =
                            Gather::new(source, true, self.origins, self.corners, self.triangles);
                        let mut value = 0;
                        for destination in 0..gather.output_count as usize {
                            let Some(Some(origin)) =
                                control.run_unit(|| gather.source(destination))?
                            else {
                                return Ok(false);
                            };
                            let (valid, found) =
                                correspondence_lookup(source_indices, origin, control)?;
                            if !valid {
                                return Ok(false);
                            }
                            if let Some(found) = found {
                                for lane in 0..lanes {
                                    check!(
                                        source_values
                                            .get(found * lanes + lane)
                                            .zip(actual_values.get(value))
                                            .is_some_and(|(&source, &actual)| ($eq)(
                                                source, actual
                                            ))
                                    );
                                    value += 1;
                                }
                            }
                        }
                    } else {
                        if source_indices.len() != actual_indices.len()
                            || source_values.len() != actual_values.len()
                        {
                            return Ok(false);
                        }
                        for (&source, &actual) in source_indices.iter().zip(actual_indices) {
                            check!(source == actual);
                        }
                        for (&source, &actual) in source_values.iter().zip(actual_values) {
                            check!(($eq)(source, actual));
                        }
                    }
                    if source_default.len() != actual_default.len() {
                        return Ok(false);
                    }
                    for (&source, &actual) in source_default.iter().zip(actual_default) {
                        check!(($eq)(source, actual));
                    }
                }};
            }
            use StoredAttributePayload::*;
            match (source.payload, actual.payload) {
                (DenseF32(sr), DenseF32(ar)) => dense!(sr, ar, f32_values, |a: f32, b: f32| a
                    .to_bits()
                    == b.to_bits()),
                (DenseF64(sr), DenseF64(ar)) => dense!(sr, ar, f64_values, |a: f64, b: f64| a
                    .to_bits()
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
                    SparseF64 {
                        indices: si,
                        values: sv,
                        default: sd,
                    },
                    SparseF64 {
                        indices: ai,
                        values: av,
                        default: ad,
                    },
                ) => sparse!(si, sv, sd, ai, av, ad, f64_values, |a: f64, b: f64| a
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
                _ => return Ok(false),
            }
        }
        for (index, (source, actual)) in self
            .source
            .primitives
            .iter()
            .zip(&output.primitives)
            .enumerate()
        {
            check!(
                source.vertex_count == actual.vertex_count
                    && source.material == actual.material
                    && same_range(source.attributes, actual.attributes)
                    && if index == self.target {
                        actual.topology == TopologyKind::TriangleList
                            && actual.corner_count == self.corners
                            && actual.face_count == self.triangles
                            && actual.polygon_boundaries.is_none()
                    } else {
                        source.topology == actual.topology
                            && source.corner_count == actual.corner_count
                            && source.face_count == actual.face_count
                    }
            );
            if index == self.target {
                let Some(indices) = actual
                    .indices
                    .and_then(|range| range.get(&output.u32_values))
                else {
                    return Ok(false);
                };
                if indices.len() != self.corners as usize {
                    return Ok(false);
                }
                for (destination, &corner) in self.origins.triangles.iter().flatten().enumerate() {
                    let expected = match source.indices {
                        Some(range) => range
                            .get(&self.source.u32_values)
                            .and_then(|values| values.get(corner as usize))
                            .copied(),
                        None => Some(corner),
                    };
                    check!(expected == indices.get(destination).copied());
                }
            } else {
                for (source, actual) in [
                    (source.indices, actual.indices),
                    (source.polygon_boundaries, actual.polygon_boundaries),
                ] {
                    match (source, actual) {
                        (None, None) => {}
                        (Some(source), Some(actual)) => {
                            let (Some(source), Some(actual)) = (
                                source.get(&self.source.u32_values),
                                actual.get(&output.u32_values),
                            ) else {
                                return Ok(false);
                            };
                            if source.len() != actual.len() {
                                return Ok(false);
                            }
                            for (&source, &actual) in source.iter().zip(actual) {
                                check!(source == actual);
                            }
                        }
                        _ => return Ok(false),
                    }
                }
            }
        }
        for (source, actual) in self
            .source
            .logical_meshes
            .iter()
            .zip(&output.logical_meshes)
        {
            check!(same_range(source.primitives, actual.primitives) && source.name == actual.name);
        }
        for (source, actual) in self
            .source
            .mesh_instances
            .iter()
            .zip(&output.mesh_instances)
        {
            check!(
                source.owner_node == actual.owner_node
                    && source.logical_mesh == actual.logical_mesh
            );
        }
        Ok(true)
    }
}

fn same_range<T>(left: TableRange<T>, right: TableRange<T>) -> bool {
    left.start() == right.start() && left.len() == right.len()
}

fn correspondence_lookup<C: super::validation::TypedStorageValidationControl>(
    indices: &[u32],
    source: usize,
    control: &mut C,
) -> Result<(bool, Option<usize>), C::Error> {
    let (mut low, mut high) = (0, indices.len());
    while low < high {
        let middle = low + (high - low) / 2;
        let Some(ordering) = control.run_unit(|| (indices[middle] as usize).cmp(&source))? else {
            return Ok((false, None));
        };
        match ordering {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Ok((true, Some(middle))),
        }
    }
    Ok((true, None))
}

/// Face intervals are strictly nonempty, so the face cursor advances at most once per output.
struct Gather<'a> {
    origins: TriangulationOrigins<'a>,
    domain: AttributeDomain,
    remapped: bool,
    output_count: u32,
    face: usize,
}

impl<'a> Gather<'a> {
    fn new(
        attribute: StoredAttribute,
        remapped: bool,
        origins: TriangulationOrigins<'a>,
        corners: u32,
        triangles: u32,
    ) -> Self {
        Self {
            origins,
            domain: attribute.domain,
            remapped,
            output_count: if !remapped {
                attribute.element_count
            } else if attribute.domain == AttributeDomain::Corner {
                corners
            } else {
                triangles
            },
            face: 0,
        }
    }

    fn source(&mut self, destination: usize) -> Option<usize> {
        if destination >= self.output_count as usize {
            return None;
        }
        if !self.remapped {
            return Some(destination);
        }
        match self.domain {
            AttributeDomain::Corner => self
                .origins
                .triangles
                .get(destination / 3)
                .map(|triangle| triangle[destination % 3] as usize),
            AttributeDomain::Face => {
                if destination >= *self.origins.face_offsets.get(self.face + 1)? as usize {
                    self.face = self.face.checked_add(1)?;
                }
                (destination < *self.origins.face_offsets.get(self.face + 1)? as usize)
                    .then_some(self.face)
            }
            _ => None,
        }
    }
}

fn validate_origins<C: TypedStorageBuildControl>(
    source: &GeometryStorage,
    primitive: &StoredPrimitive,
    origins: TriangulationOrigins<'_>,
    control: &mut C,
) -> EditResult<(), C::Error> {
    if primitive.topology != TopologyKind::Polygons
        || origins.triangles.is_empty()
        || (primitive.face_count as usize).checked_add(1) != Some(origins.face_offsets.len())
        || origins.face_offsets.first() != Some(&0)
        || origins
            .face_offsets
            .last()
            .copied()
            .map(|count| count as usize)
            != Some(origins.triangles.len())
    {
        return Err(TypedStorageBuildError::Invariant);
    }
    let faces = primitive
        .polygon_boundaries
        .and_then(|range| range.get(&source.u32_values))
        .ok_or(TypedStorageBuildError::Invariant)?;
    if faces.len() != primitive.face_count as usize {
        return Err(TypedStorageBuildError::Invariant);
    }
    let mut start = 0_u32;
    for (face, &size) in faces.iter().enumerate() {
        let end = start.checked_add(size).ok_or_else(overflow)?;
        let first = origins.face_offsets[face];
        let last = origins.face_offsets[face + 1];
        let valid = control
            .run_unit(|| size >= 3 && last.checked_sub(first) == size.checked_sub(2))
            .map_err(TypedStorageBuildError::Control)?;
        if !valid {
            return Err(TypedStorageBuildError::Invariant);
        }
        for triangle in origins
            .triangles
            .get(first as usize..last as usize)
            .ok_or(TypedStorageBuildError::Invariant)?
        {
            let valid = control
                .run_unit(|| {
                    triangle
                        .iter()
                        .all(|&corner| corner >= start && corner < end)
                        && triangle[0] != triangle[1]
                        && triangle[1] != triangle[2]
                        && triangle[2] != triangle[0]
                })
                .map_err(TypedStorageBuildError::Control)?;
            if !valid {
                return Err(TypedStorageBuildError::Invariant);
            }
        }
        start = end;
    }
    if start != primitive.corner_count {
        return Err(TypedStorageBuildError::Invariant);
    }
    Ok(())
}

fn sparse_lookup<C: TypedStorageBuildControl>(
    indices: &[u32],
    source: usize,
    control: &mut C,
) -> EditResult<(Option<usize>, u64), C::Error> {
    let source = u32::try_from(source).map_err(|_| overflow())?;
    let mut first = 0;
    let mut end = indices.len();
    let mut comparisons = 0_u64;
    while first < end {
        let middle = first + (end - first) / 2;
        let order = control
            .run_unit(|| indices[middle].cmp(&source))
            .map_err(TypedStorageBuildError::Control)?;
        add_work(&mut comparisons, 1)?;
        match order {
            std::cmp::Ordering::Less => first = middle + 1,
            std::cmp::Ordering::Greater => end = middle,
            std::cmp::Ordering::Equal => return Ok((Some(middle), comparisons)),
        }
    }
    Ok((None, comparisons))
}

fn gather_dense<T: Copy, C: TypedStorageBuildControl>(
    destination: &mut Vec<T>,
    plan: CapacityPlan,
    source: &[T],
    lanes: usize,
    gather: &mut Gather<'_>,
    control: &mut C,
) -> EditResult<TableRange<T>, C::Error> {
    let start = destination.len();
    for output in 0..gather.output_count as usize {
        let element = control
            .run_unit(|| gather.source(output))
            .map_err(TypedStorageBuildError::Control)?
            .ok_or(TypedStorageBuildError::Invariant)?;
        let first = element.checked_mul(lanes).ok_or_else(overflow)?;
        let end = first.checked_add(lanes).ok_or_else(overflow)?;
        append(
            destination,
            plan,
            source
                .get(first..end)
                .ok_or(TypedStorageBuildError::Invariant)?,
            control,
        )?;
    }
    Ok(TableRange::new(start, destination.len() - start))
}

fn capacity<T, E>(count: usize) -> EditResult<CapacityPlan, E> {
    CapacityPlan::for_type::<T>(0, 0, count).map_err(TypedStorageBuildError::Capacity)
}

fn validate_dense_range<T, E>(
    range: TableRange<T>,
    source: &[T],
    attribute: StoredAttribute,
) -> EditResult<(), E> {
    let count = (attribute.element_count as usize)
        .checked_mul(usize::from(attribute.lanes))
        .ok_or_else(overflow)?;
    if range.len() != count || range.checked(source.len()).is_none() {
        return Err(TypedStorageBuildError::Invariant);
    }
    Ok(())
}

fn allocate<T, E>(plan: CapacityPlan) -> EditResult<Vec<T>, E> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| TypedStorageBuildError::AllocationFailed)?;
    plan.finish(0, values.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(values)
}

fn push<T, C: TypedStorageBuildControl>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
    value: T,
    control: &mut C,
) -> EditResult<(), C::Error> {
    if values.len() >= plan.desired_capacity() || values.len() == values.capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    control
        .run_unit(|| values.push(value))
        .map_err(TypedStorageBuildError::Control)
}

fn append<T: Copy, C: TypedStorageBuildControl>(
    destination: &mut Vec<T>,
    plan: CapacityPlan,
    source: &[T],
    control: &mut C,
) -> EditResult<TableRange<T>, C::Error> {
    let first = destination.len();
    for &value in source {
        push(destination, plan, value, control)?;
    }
    Ok(TableRange::new(first, source.len()))
}

fn copy_optional<T: Copy, C: TypedStorageBuildControl>(
    range: Option<TableRange<T>>,
    source: &[T],
    destination: &mut Vec<T>,
    plan: CapacityPlan,
    control: &mut C,
) -> EditResult<Option<TableRange<T>>, C::Error> {
    range
        .map(|range| {
            append(
                destination,
                plan,
                range.get(source).ok_or(TypedStorageBuildError::Invariant)?,
                control,
            )
        })
        .transpose()
}

fn overflow<E>() -> TypedStorageBuildError<E> {
    TypedStorageBuildError::Capacity(CapacityPlanError::CapacityOverflow)
}

fn add_count<E>(count: &mut usize, additional: usize) -> EditResult<(), E> {
    *count = count.checked_add(additional).ok_or_else(overflow)?;
    Ok(())
}

fn add_work<E>(count: &mut u64, additional: u64) -> EditResult<(), E> {
    *count = count.checked_add(additional).ok_or_else(overflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
    use crate::canonical_scene::store::typed_storage::{
        StoredAttributeSemantic, StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
    };

    const TRIANGLES: [[u32; 3]; 3] = [[0, 1, 2], [0, 2, 3], [4, 5, 6]];
    const FACE_OFFSETS: [u32; 3] = [0, 2, 3];
    const CORNER_ORIGINS: [usize; 9] = [0, 1, 2, 0, 2, 3, 4, 5, 6];

    #[derive(Default)]
    struct Control {
        completed: u64,
        strings: u64,
        reject_before: Option<u64>,
    }

    impl Control {
        fn step<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
            if self.reject_before == Some(self.completed) {
                return Err(self.completed);
            }
            let result = action();
            self.completed += 1;
            Ok(result)
        }
    }

    impl TypedStorageBuildControl for Control {
        type Error = u64;

        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
            self.step(action)
        }

        fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
            let result = self.step(action)?;
            self.strings += 1;
            Ok(result)
        }
    }

    impl TypedStorageValidationControl for Control {
        type Error = u64;

        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, u64> {
            self.step(action).map(Some)
        }
    }

    fn origins() -> TriangulationOrigins<'static> {
        TriangulationOrigins {
            triangles: &TRIANGLES,
            face_offsets: &FACE_OFFSETS,
        }
    }

    fn name(names: &mut Vec<u8>, text: &str) -> TextRange {
        let first = names.len();
        names.extend_from_slice(text.as_bytes());
        TextRange(TableRange::new(first, text.len()))
    }

    /// Two source faces, one unrelated implicit triangle primitive, and ordered typed pools.
    fn geometry() -> GeometryStorage {
        let mut names = Vec::new();
        let corner_f64 = name(&mut names, "edit:corner_f64");
        let corner_sparse = name(&mut names, "edit:corner_sparse");
        let face_dense = name(&mut names, "edit:face_dense");
        let mesh_name = name(&mut names, "original mesh");
        let mut f32_values = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0,
        ];
        for corner in 0..7 {
            f32_values.extend_from_slice(&[corner as f32, -(corner as f32), 1.0]);
        }
        f32_values.extend_from_slice(&[2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0]);
        GeometryStorage {
            logical_meshes: vec![StoredLogicalMesh {
                primitives: TableRange::new(0, 2),
                name: Some(mesh_name),
            }],
            primitives: vec![
                StoredPrimitive {
                    topology: TopologyKind::Polygons,
                    vertex_count: 5,
                    corner_count: 7,
                    face_count: 2,
                    polygon_boundaries: Some(TableRange::new(8, 2)),
                    indices: Some(TableRange::new(10, 7)),
                    material: Some(0),
                    attributes: TableRange::new(0, 6),
                },
                StoredPrimitive {
                    topology: TopologyKind::TriangleList,
                    vertex_count: 3,
                    corner_count: 3,
                    face_count: 1,
                    polygon_boundaries: None,
                    indices: None,
                    material: None,
                    attributes: TableRange::new(6, 1),
                },
            ],
            attributes: vec![
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Position,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 5,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(0, 15)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Normal,
                    domain: AttributeDomain::Corner,
                    lanes: 3,
                    element_count: 7,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(15, 21)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::SmoothingGroup,
                    domain: AttributeDomain::Face,
                    lanes: 1,
                    element_count: 2,
                    payload: StoredAttributePayload::SparseU32 {
                        indices: TableRange::new(0, 1),
                        values: TableRange::new(1, 1),
                        default: TableRange::new(2, 1),
                    },
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(corner_f64),
                    domain: AttributeDomain::Corner,
                    lanes: 2,
                    element_count: 7,
                    payload: StoredAttributePayload::DenseF64(TableRange::new(0, 14)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(corner_sparse),
                    domain: AttributeDomain::Corner,
                    lanes: 1,
                    element_count: 7,
                    payload: StoredAttributePayload::SparseI32 {
                        indices: TableRange::new(3, 3),
                        values: TableRange::new(0, 3),
                        default: TableRange::new(3, 1),
                    },
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(face_dense),
                    domain: AttributeDomain::Face,
                    lanes: 1,
                    element_count: 2,
                    payload: StoredAttributePayload::DenseU32(TableRange::new(6, 2)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Position,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 3,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(36, 9)),
                },
            ],
            mesh_instances: vec![StoredMeshInstance {
                owner_node: StoredNodeIndex(1),
                logical_mesh: StoredLogicalMeshIndex(0),
            }],
            names,
            f32_values,
            f64_values: vec![
                1.0e200, -0.0, 2.0e200, 0.0, 3.0e200, -0.0, 4.0e200, 0.0, 5.0e200, -0.0, 6.0e200,
                0.0, 7.0e200, -0.0,
            ],
            u32_values: vec![1, 7, 0, 0, 3, 6, 41, 42, 4, 3, 0, 1, 2, 3, 0, 3, 4],
            i32_values: vec![0, -30, 60, 0],
        }
    }

    #[test]
    fn rewrite_preserves_vertex_identity_f64_bits_and_explicit_sparse_defaults() {
        let source = geometry();
        assert_eq!(
            source.validate(&mut Control::default(), [1, 2, 7, 1], 2, 1),
            Ok(true)
        );
        let source_values = source.f64_values.as_ptr();
        let plan =
            GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                .expect("source-bound replacement must plan");
        assert_eq!(plan.output_face_count(), 3);
        assert_eq!(plan.output_corner_count(), 9);
        let mut build = Control::default();
        let output = plan
            .materialize(&mut build)
            .expect("typed replacement must materialize");
        assert_eq!(build.completed, plan.work());
        assert_eq!(build.strings, source.names.len() as u64);
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 2, 7, 1], 2, 1),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        assert_eq!(validation.completed, 106);
        assert_eq!(output.primitives[0].topology, TopologyKind::TriangleList);
        assert_eq!(output.primitives[0].material, source.primitives[0].material);
        assert_eq!(output.primitives[0].vertex_count, 5);
        assert_eq!(
            output.primitives[0]
                .indices
                .unwrap()
                .get(&output.u32_values),
            Some(&[0, 1, 2, 0, 2, 3, 0, 3, 4][..])
        );
        assert_eq!(output.primitives[1].topology, TopologyKind::TriangleList);
        assert!(output.primitives[1].indices.is_none());
        assert_eq!(output.names, source.names);
        assert_eq!(output.logical_meshes[0].primitives.start(), 0);
        assert_eq!(output.logical_meshes[0].primitives.len(), 2);
        assert_eq!(
            output.mesh_instances[0].owner_node,
            source.mesh_instances[0].owner_node
        );

        for (row, expected_start, expected_length) in [(0, 0, 15), (6, 36, 9)] {
            let StoredAttributePayload::DenseF32(range) = output.attributes[row].payload else {
                panic!("Vertex positions must keep their scalar kind");
            };
            assert_eq!(
                range.get(&output.f32_values),
                Some(&source.f32_values[expected_start..expected_start + expected_length])
            );
        }
        let StoredAttributePayload::DenseF32(normals) = output.attributes[1].payload else {
            panic!("authored corner normals must remain dense F32");
        };
        let normals = normals.get(&output.f32_values).unwrap();
        for (destination, &origin) in CORNER_ORIGINS.iter().enumerate() {
            assert_eq!(
                &normals[destination * 3..destination * 3 + 3],
                &source.f32_values[15 + origin * 3..18 + origin * 3]
            );
        }
        let StoredAttributePayload::DenseF64(f64s) = output.attributes[3].payload else {
            panic!("custom F64 data must retain its scalar kind");
        };
        let f64s = f64s.get(&output.f64_values).unwrap();
        for (destination, &origin) in CORNER_ORIGINS.iter().enumerate() {
            for lane in 0..2 {
                assert_eq!(
                    f64s[destination * 2 + lane].to_bits(),
                    source.f64_values[origin * 2 + lane].to_bits()
                );
            }
        }
        let StoredAttributePayload::SparseI32 {
            indices,
            values,
            default,
        } = output.attributes[4].payload
        else {
            panic!("sparse Corner data must remain sparse");
        };
        assert_eq!(indices.get(&output.u32_values), Some(&[0, 3, 5, 8][..]));
        assert_eq!(values.get(&output.i32_values), Some(&[0, 0, -30, 60][..]));
        assert_eq!(default.get(&output.i32_values), Some(&[0][..]));
        let StoredAttributePayload::SparseU32 {
            indices,
            values,
            default,
        } = output.attributes[2].payload
        else {
            panic!("sparse Face smoothing data must remain sparse");
        };
        assert_eq!(indices.get(&output.u32_values), Some(&[2][..]));
        assert_eq!(values.get(&output.u32_values), Some(&[7][..]));
        assert_eq!(default.get(&output.u32_values), Some(&[0][..]));
        let StoredAttributePayload::DenseU32(values) = output.attributes[5].payload else {
            panic!("Face values must preserve their encoding");
        };
        assert_eq!(values.get(&output.u32_values), Some(&[41, 41, 42][..]));
        assert_eq!(source.f64_values.as_ptr(), source_values);
        assert_eq!(source.primitives[0].topology, TopologyKind::Polygons);
        assert_eq!(source.primitives[0].corner_count, 7);
        assert_eq!(
            source.u32_values,
            [1, 7, 0, 0, 3, 6, 41, 42, 4, 3, 0, 1, 2, 3, 0, 3, 4]
        );
    }

    #[test]
    fn sparse_f64_copy_and_remap_preserve_bits_explicit_defaults_and_work_counts() {
        let tuples = [
            [-0.0, f64::from_bits(1)],
            [f64::from_bits(1.0_f64.to_bits() + 1), 1.0e250],
            [0.0, -f64::from_bits(1)],
        ];
        for (domain, row, count, source_indices, expected_indices, expected_tuples) in [
            (
                AttributeDomain::Corner,
                4,
                7,
                &[0, 3, 6][..],
                &[0, 3, 5, 8][..],
                &[0, 0, 1, 2][..],
            ),
            (
                AttributeDomain::Vertex,
                4,
                5,
                &[0, 3, 4][..],
                &[0, 3, 4][..],
                &[0, 1, 2][..],
            ),
            (
                AttributeDomain::Face,
                5,
                2,
                &[0, 1][..],
                &[0, 1, 2][..],
                &[0, 0, 1][..],
            ),
        ] {
            let mut source = geometry();
            let indices = match source.attributes[row].payload {
                StoredAttributePayload::SparseI32 { indices, .. } => {
                    source.i32_values.clear();
                    indices
                }
                StoredAttributePayload::DenseU32(indices) => indices,
                _ => unreachable!(),
            };
            let index_range = indices.checked(source.u32_values.len()).unwrap();
            source.u32_values[index_range].copy_from_slice(source_indices);
            let value_start = source.f64_values.len();
            let value_count = source_indices.len() * 2;
            source
                .f64_values
                .extend_from_slice(&tuples.as_flattened()[..value_count]);
            source.f64_values.extend_from_slice(&tuples[0]);
            source.attributes[row].domain = domain;
            source.attributes[row].lanes = 2;
            source.attributes[row].element_count = count;
            source.attributes[row].payload = StoredAttributePayload::SparseF64 {
                indices,
                values: TableRange::new(value_start, value_count),
                default: TableRange::new(value_start + value_count, 2),
            };
            let source_bits: Vec<_> = source.f64_values.iter().map(|v| v.to_bits()).collect();
            assert_eq!(
                source.validate(&mut Control::default(), [1, 2, 7, 1], 2, 1),
                Ok(true)
            );
            let plan =
                GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                    .unwrap();
            assert_eq!(
                plan.capacities()[6].desired_capacity(),
                CORNER_ORIGINS.len() * 2 + expected_tuples.len() * 2 + 2
            );
            let mut build = Control::default();
            let output = plan.materialize(&mut build).unwrap();
            assert_eq!(build.completed, plan.work());
            let mut validation = Control::default();
            assert_eq!(
                output.validate(&mut validation, [1, 2, 7, 1], 2, 1),
                Ok(true)
            );
            assert_eq!(validation.completed, plan.validation_work());
            let mut correspondence = Control::default();
            assert_eq!(
                plan.validate_correspondence(&output, &mut correspondence),
                Ok(true)
            );
            assert_eq!(correspondence.completed, plan.correspondence_work());
            assert_eq!(
                output.attributes[row].semantic,
                source.attributes[row].semantic
            );
            assert_eq!(output.attributes[row].domain, domain);
            assert_eq!(output.attributes[row].lanes, 2);
            let StoredAttributePayload::SparseF64 {
                indices,
                values,
                default,
            } = output.attributes[row].payload
            else {
                panic!("sparse F64 must retain its encoding");
            };
            assert_eq!(indices.get(&output.u32_values), Some(expected_indices));
            assert_eq!(values.start() + values.len(), default.start());
            assert_eq!(
                values
                    .get(&output.f64_values)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected_tuples
                    .iter()
                    .flat_map(|&index| tuples[index].map(f64::to_bits))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                default
                    .get(&output.f64_values)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                tuples[0].map(f64::to_bits)
            );
            for scalar in [values.start(), default.start()] {
                let mut corrupted = plan.materialize(&mut Control::default()).unwrap();
                corrupted.f64_values[scalar] = 0.0;
                assert_eq!(
                    plan.validate_correspondence(&corrupted, &mut Control::default()),
                    Ok(false)
                );
            }
            assert_eq!(
                source
                    .f64_values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                source_bits
            );
        }
    }

    #[test]
    fn origin_planning_allocates_nothing_and_rejects_cross_face_or_incomplete_relations() {
        let source = geometry();
        let mut plan = None;
        let allocations = allocation_counter::measure(|| {
            plan = Some(
                GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                    .expect("valid origin relation must plan"),
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        assert!(plan.is_some());

        let invalid_triangles = [
            [[0, 1, 4], [0, 2, 3], [4, 5, 6]],
            [[0, 1, 1], [0, 2, 3], [4, 5, 6]],
            [[0, 1, 2], [0, 2, 3], [4, 5, 7]],
        ];
        for triangles in &invalid_triangles {
            assert!(matches!(
                GeometryTriangulationPlan::try_new(
                    &source,
                    0,
                    TriangulationOrigins {
                        triangles,
                        face_offsets: &FACE_OFFSETS
                    },
                    &mut Control::default(),
                ),
                Err(TypedStorageBuildError::Invariant)
            ));
        }
        for offsets in [[1, 2, 3], [0, 1, 3], [0, 3, 2], [0, 2, 4]] {
            assert!(matches!(
                GeometryTriangulationPlan::try_new(
                    &source,
                    0,
                    TriangulationOrigins {
                        triangles: &TRIANGLES,
                        face_offsets: &offsets
                    },
                    &mut Control::default(),
                ),
                Err(TypedStorageBuildError::Invariant)
            ));
        }
    }

    #[test]
    fn triangulation_copies_tangent_declarations_with_corner_payloads_and_schema_text() {
        use crate::canonical_scene::store::typed_storage::{
            StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
        };
        let basis = "test:polygon-basis";
        let encoding = "test:polygon-sign";
        let declarations = Some(StoredTangentConventions {
            basis: StoredTangentBasis::Namespaced {
                name: TextRange(TableRange::new(0, basis.len())),
                version: std::num::NonZeroU32::new(2).unwrap(),
            },
            encoding: StoredTangentEncoding::Namespaced {
                name: TextRange(TableRange::new(basis.len(), encoding.len())),
                version: std::num::NonZeroU32::new(3).unwrap(),
            },
        });
        let source = GeometryStorage {
            logical_meshes: vec![StoredLogicalMesh {
                primitives: TableRange::new(0, 1),
                name: None,
            }],
            primitives: vec![StoredPrimitive {
                topology: TopologyKind::Polygons,
                vertex_count: 4,
                corner_count: 4,
                face_count: 1,
                polygon_boundaries: Some(TableRange::new(0, 1)),
                indices: Some(TableRange::new(1, 4)),
                material: None,
                attributes: TableRange::new(0, 2),
            }],
            attributes: vec![
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Position,
                    tangent_conventions: None,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 4,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(0, 12)),
                },
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Tangent,
                    tangent_conventions: declarations,
                    domain: AttributeDomain::Corner,
                    lanes: 4,
                    element_count: 4,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(12, 16)),
                },
            ],
            mesh_instances: vec![],
            names: [basis.as_bytes(), encoding.as_bytes()].concat(),
            f32_values: vec![0.0; 28],
            f64_values: vec![],
            u32_values: vec![4, 0, 1, 2, 3],
            i32_values: vec![],
        };
        assert_eq!(
            source.validate(&mut Control::default(), [1, 1, 2, 0], 1, 0),
            Ok(true)
        );
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let offsets = [0, 2];
        let plan = GeometryTriangulationPlan::try_new(
            &source,
            0,
            TriangulationOrigins {
                triangles: &triangles,
                face_offsets: &offsets,
            },
            &mut Control::default(),
        )
        .unwrap();
        let mut output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(output.attributes[1].element_count, 6);
        assert_eq!(output.attributes[1].tangent_conventions, declarations);
        assert_eq!(output.names, source.names);
        assert_eq!(
            output.validate(&mut Control::default(), [1, 1, 2, 0], 1, 0),
            Ok(true)
        );
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(true)
        );
        output.attributes[1]
            .tangent_conventions
            .as_mut()
            .unwrap()
            .basis = StoredTangentBasis::Unknown;
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
    }

    #[test]
    fn every_planning_and_materialization_action_is_interruptible() {
        let source = geometry();
        let mut planning = Control::default();
        let plan = GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut planning)
            .expect("valid relation must plan");
        for completed in 0..planning.completed {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(matches!(GeometryTriangulationPlan::try_new(
                &source, 0, origins(), &mut control,
            ), Err(TypedStorageBuildError::Control(at)) if at == completed));
            assert_eq!(control.completed, completed);
        }
        for completed in 0..plan.work() {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(matches!(plan.materialize(&mut control),
                Err(TypedStorageBuildError::Control(at)) if at == completed));
            assert_eq!(control.completed, completed);
        }
        assert_eq!(source.primitives[0].topology, TopologyKind::Polygons);
        assert_eq!(source.attributes[4].element_count, 7);
    }
}
