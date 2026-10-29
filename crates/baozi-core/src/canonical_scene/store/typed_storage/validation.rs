//! Central validation over the actual retained tables, without immutable row projections.

use baozi_operation::__retained_store_integration::activation::CanonicalValidationSession;
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreWorkError};

use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::namespaced::{MAX_NAMESPACED_NAME_LEN, NamespacedName};

use super::*;

pub(in crate::canonical_scene::store) trait TypedStorageValidationControl {
    type Error;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error>;
}

impl<O, PP, U> TypedStorageValidationControl
    for CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = StoreWorkError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        self.run_validation_unit(action)
    }
}

impl NodeStorage {
    /// Breadth-first canonical ranges prove a forest without allocating a second node table.
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        expected_nodes: usize,
        expected_roots: usize,
        expected_name_bytes: usize,
        camera_placements: &mut [u32],
        light_placements: &mut [u32],
    ) -> Result<bool, C::Error> {
        if self.nodes.len() != expected_nodes
            || expected_nodes == 0
            || self.children.len() != expected_nodes - 1
            || self.names.len() != expected_name_bytes
        {
            return Ok(false);
        }
        let mut next_child = 0;
        let mut next_name = 0;
        let mut transform_scalars = 0_usize;
        for (index, row) in self.nodes.iter().enumerate() {
            let valid = control.run_unit(|| {
                if row.children.start() != next_child
                    || row.children.checked(self.children.len()).is_none()
                {
                    return false;
                }
                if index == 0 {
                    if row.children.len() != expected_roots
                        || row.name.is_some()
                        || row.camera.is_some()
                        || row.light.is_some()
                        || !matches!(row.transform, StoredTransform::Synthetic)
                    {
                        return false;
                    }
                } else {
                    if row.transform.authored().is_none()
                        || (!row.children.is_empty() && row.children.start() < index)
                    {
                        return false;
                    }
                    if let Some(name) = row.name {
                        if name.0.start() != next_name || name.0.len() > MAX_ENTITY_NAME_BYTES {
                            return false;
                        }
                        let Some(text) = name.get(&self.names) else {
                            return false;
                        };
                        if validate_entity_name_str(text).is_err() {
                            return false;
                        }
                        next_name += name.0.len();
                    }
                    for (attachment, counts) in [
                        (row.camera, &mut *camera_placements),
                        (row.light, &mut *light_placements),
                    ] {
                        if let Some(attachment) = attachment {
                            let Some(count) = counts.get_mut(attachment as usize) else {
                                return false;
                            };
                            let Some(updated) = count.checked_add(1) else {
                                return false;
                            };
                            *count = updated;
                        }
                    }
                }
                let Some(total) = transform_scalars.checked_add(row.transform.scalar_count())
                else {
                    return false;
                };
                transform_scalars = total;
                next_child += row.children.len();
                true
            })?;
            if valid != Some(true) {
                return Ok(false);
            }
        }
        for (index, child) in self.children.iter().enumerate() {
            if control.run_unit(|| child.0 as usize == index + 1)? != Some(true) {
                return Ok(false);
            }
        }
        for &placements in camera_placements.iter().chain(light_placements.iter()) {
            if control.run_unit(|| placements != 0)? != Some(true) {
                return Ok(false);
            }
        }
        Ok(next_child == self.children.len()
            && next_name == self.names.len()
            && transform_scalars == self.transform_scalar_count)
    }
}

/// Only mutable traversal cursors; the semantic rows and payloads remain borrowed in place.
#[derive(Default)]
struct PoolCursors {
    f32_values: usize,
    f64_values: usize,
    u32_values: usize,
    i32_values: usize,
    names: usize,
}

impl GeometryStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        expected_rows: [usize; 4],
        node_count: usize,
        material_count: usize,
    ) -> Result<bool, C::Error> {
        if expected_rows
            != [
                self.logical_meshes.len(),
                self.primitives.len(),
                self.attributes.len(),
                self.mesh_instances.len(),
            ]
        {
            return Ok(false);
        }
        let mut pools = PoolCursors::default();
        let mut next_attribute = 0;
        for primitive in &self.primitives {
            if control.run_unit(|| {
                if primitive.attributes.start() != next_attribute
                    || primitive
                        .attributes
                        .checked(self.attributes.len())
                        .is_none()
                    || primitive
                        .material
                        .is_some_and(|index| index as usize >= material_count)
                {
                    return false;
                }
                next_attribute += primitive.attributes.len();
                true
            })? != Some(true)
            {
                return Ok(false);
            }
            let Some(attributes) = primitive.attributes.get(&self.attributes) else {
                return Ok(false);
            };
            let mut previous = None;
            let mut has_position = false;
            for attribute in attributes {
                if control.run_unit(|| {
                    let Some(key) = self.attribute_key(attribute) else {
                        return false;
                    };
                    if previous.is_some_and(|prior| prior >= key) || !attribute_schema(attribute) {
                        return false;
                    }
                    previous = Some(key);
                    has_position |= attribute.semantic == StoredAttributeSemantic::Position;
                    let cardinality = match attribute.domain {
                        AttributeDomain::Vertex => primitive.vertex_count,
                        AttributeDomain::Corner => primitive.corner_count,
                        AttributeDomain::Face => primitive.face_count,
                        AttributeDomain::Instance => return false,
                    };
                    attribute.element_count == cardinality
                })? != Some(true)
                {
                    return Ok(false);
                }
                if !self.validate_payload(attribute, &mut pools, control)? {
                    return Ok(false);
                }
            }
            if primitive.vertex_count != 0 && !has_position {
                return Ok(false);
            }
        }
        if next_attribute != self.attributes.len() {
            return Ok(false);
        }

        // Primitive U32 payloads follow all attribute pools in the one canonical pool layout.
        for primitive in &self.primitives {
            let mut polygon_total = 0_u32;
            let polygon_faces = if let Some(range) = primitive.polygon_boundaries {
                let Some(values) = consume_range(range, &self.u32_values, &mut pools.u32_values)
                else {
                    return Ok(false);
                };
                for &count in values {
                    if control.run_unit(|| {
                        if count < 3 {
                            return false;
                        }
                        let Some(total) = polygon_total.checked_add(count) else {
                            return false;
                        };
                        polygon_total = total;
                        true
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
                Some(values.len())
            } else {
                None
            };
            let index_count = if let Some(range) = primitive.indices {
                let Some(values) = consume_range(range, &self.u32_values, &mut pools.u32_values)
                else {
                    return Ok(false);
                };
                for &index in values {
                    if control.run_unit(|| index < primitive.vertex_count)? != Some(true) {
                        return Ok(false);
                    }
                }
                values.len()
            } else {
                primitive.vertex_count as usize
            };
            if control.run_unit(|| {
                index_count == primitive.corner_count as usize
                    && topology_shape(primitive, polygon_faces, polygon_total)
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        let mut next_primitive = 0;
        for mesh in &self.logical_meshes {
            if control.run_unit(|| {
                if mesh.primitives.is_empty()
                    || mesh.primitives.start() != next_primitive
                    || mesh.primitives.checked(self.primitives.len()).is_none()
                {
                    return false;
                }
                if let Some(name) = mesh.name {
                    if name.0.start() != pools.names || name.0.len() > MAX_ENTITY_NAME_BYTES {
                        return false;
                    }
                    let Some(text) = name.get(&self.names) else {
                        return false;
                    };
                    if validate_entity_name_str(text).is_err() {
                        return false;
                    }
                    pools.names += name.0.len();
                }
                next_primitive += mesh.primitives.len();
                true
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        if next_primitive != self.primitives.len() {
            return Ok(false);
        }
        for instance in &self.mesh_instances {
            if control.run_unit(|| {
                instance.owner_node.0 != 0
                    && (instance.owner_node.0 as usize) < node_count
                    && (instance.logical_mesh.0 as usize) < self.logical_meshes.len()
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        Ok([
            pools.f32_values,
            pools.f64_values,
            pools.u32_values,
            pools.i32_values,
            pools.names,
        ] == [
            self.f32_values.len(),
            self.f64_values.len(),
            self.u32_values.len(),
            self.i32_values.len(),
            self.names.len(),
        ])
    }

    fn attribute_key<'a>(
        &'a self,
        attribute: &StoredAttribute,
    ) -> Option<(u8, u32, &'a str, AttributeDomain)> {
        let (kind, set, name) = match attribute.semantic {
            StoredAttributeSemantic::Position => (0, 0, ""),
            StoredAttributeSemantic::Normal => (1, 0, ""),
            StoredAttributeSemantic::Tangent => (2, 0, ""),
            StoredAttributeSemantic::Texcoord(set) => (3, set, ""),
            StoredAttributeSemantic::Color(set) => (4, set, ""),
            StoredAttributeSemantic::SmoothingGroup => (5, 0, ""),
            StoredAttributeSemantic::Custom(range) => {
                if range.0.len() > MAX_NAMESPACED_NAME_LEN {
                    return None;
                }
                let name = range.get(&self.names)?;
                NamespacedName::new(name).ok()?;
                (6, 0, name)
            }
        };
        Some((kind, set, name, attribute.domain))
    }

    fn validate_payload<C: TypedStorageValidationControl>(
        &self,
        attribute: &StoredAttribute,
        pools: &mut PoolCursors,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if let StoredAttributeSemantic::Custom(name) = attribute.semantic {
            if name.0.start() != pools.names || name.get(&self.names).is_none() {
                return Ok(false);
            }
            pools.names += name.0.len();
        }
        let lanes = usize::from(attribute.lanes);
        let Some(dense_count) = (attribute.element_count as usize).checked_mul(lanes) else {
            return Ok(false);
        };
        macro_rules! dense {
            ($range:expr, $values:ident, $check:expr) => {{
                let Some(values) = consume_range($range, &self.$values, &mut pools.$values) else {
                    return Ok(false);
                };
                if values.len() != dense_count || !visit(values, control, $check)? {
                    return Ok(false);
                }
            }};
        }
        macro_rules! sparse {
            ($indices:expr, $values:expr, $default:expr, $pool:ident, $check:expr) => {{
                let Some(indices) =
                    consume_range($indices, &self.u32_values, &mut pools.u32_values)
                else {
                    return Ok(false);
                };
                let mut prior = None;
                for &index in indices {
                    if control.run_unit(|| {
                        let valid = index < attribute.element_count
                            && prior.is_none_or(|prior| prior < index);
                        prior = Some(index);
                        valid
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
                let Some(values) = consume_range($values, &self.$pool, &mut pools.$pool) else {
                    return Ok(false);
                };
                let Some(default) = consume_range($default, &self.$pool, &mut pools.$pool) else {
                    return Ok(false);
                };
                if indices.len().checked_mul(lanes) != Some(values.len())
                    || default.len() != lanes
                    || !visit(values, control, $check)?
                    || !visit(default, control, $check)?
                {
                    return Ok(false);
                }
            }};
        }
        match attribute.payload {
            StoredAttributePayload::DenseF32(range) => dense!(range, f32_values, f32::is_finite),
            StoredAttributePayload::DenseF64(range) => dense!(range, f64_values, f64::is_finite),
            StoredAttributePayload::DenseU32(range) => dense!(range, u32_values, |_| true),
            StoredAttributePayload::DenseI32(range) => dense!(range, i32_values, |_| true),
            StoredAttributePayload::SparseF32 {
                indices,
                values,
                default,
            } => sparse!(indices, values, default, f32_values, f32::is_finite),
            StoredAttributePayload::SparseU32 {
                indices,
                values,
                default,
            } => sparse!(indices, values, default, u32_values, |_| true),
            StoredAttributePayload::SparseI32 {
                indices,
                values,
                default,
            } => sparse!(indices, values, default, i32_values, |_| true),
        }
        Ok(true)
    }
}

fn consume_range<'a, T>(
    range: TableRange<T>,
    values: &'a [T],
    cursor: &mut usize,
) -> Option<&'a [T]> {
    if range.start() != *cursor {
        return None;
    }
    let slice = range.get(values)?;
    *cursor = cursor.checked_add(range.len())?;
    Some(slice)
}

fn visit<T: Copy, C: TypedStorageValidationControl>(
    values: &[T],
    control: &mut C,
    check: impl Fn(T) -> bool,
) -> Result<bool, C::Error> {
    for &value in values {
        if control.run_unit(|| check(value))? != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn attribute_schema(attribute: &StoredAttribute) -> bool {
    let f32_payload = matches!(
        attribute.payload,
        StoredAttributePayload::DenseF32(_) | StoredAttributePayload::SparseF32 { .. }
    );
    let u32_payload = matches!(
        attribute.payload,
        StoredAttributePayload::DenseU32(_) | StoredAttributePayload::SparseU32 { .. }
    );
    let vertex_or_corner = matches!(
        attribute.domain,
        AttributeDomain::Vertex | AttributeDomain::Corner
    );
    match attribute.semantic {
        StoredAttributeSemantic::Position => {
            attribute.domain == AttributeDomain::Vertex && f32_payload && attribute.lanes == 3
        }
        StoredAttributeSemantic::Normal => vertex_or_corner && f32_payload && attribute.lanes == 3,
        StoredAttributeSemantic::Tangent => vertex_or_corner && f32_payload && attribute.lanes == 4,
        StoredAttributeSemantic::Texcoord(_) => {
            vertex_or_corner && f32_payload && (1..=3).contains(&attribute.lanes)
        }
        StoredAttributeSemantic::Color(_) => {
            attribute.domain != AttributeDomain::Instance
                && f32_payload
                && matches!(attribute.lanes, 3 | 4)
        }
        StoredAttributeSemantic::SmoothingGroup => {
            attribute.domain == AttributeDomain::Face && u32_payload && attribute.lanes == 1
        }
        StoredAttributeSemantic::Custom(_) => {
            attribute.domain != AttributeDomain::Instance && (1..=16).contains(&attribute.lanes)
        }
    }
}

fn topology_shape(
    primitive: &StoredPrimitive,
    polygon_faces: Option<usize>,
    polygon_total: u32,
) -> bool {
    let corners = primitive.corner_count;
    let faces = match primitive.topology {
        TopologyKind::Points => corners,
        TopologyKind::LineList if corners.is_multiple_of(2) => corners / 2,
        TopologyKind::LineStrip => corners.saturating_sub(1),
        TopologyKind::LineLoop => {
            if corners < 2 {
                0
            } else {
                corners
            }
        }
        TopologyKind::TriangleList if corners.is_multiple_of(3) => corners / 3,
        TopologyKind::TriangleStrip | TopologyKind::TriangleFan => corners.saturating_sub(2),
        TopologyKind::Polygons if polygon_total == corners => {
            let Some(faces) = polygon_faces.and_then(|count| u32::try_from(count).ok()) else {
                return false;
            };
            faces
        }
        _ => return false,
    };
    faces == primitive.face_count
        && (primitive.topology == TopologyKind::Polygons) == polygon_faces.is_some()
}
