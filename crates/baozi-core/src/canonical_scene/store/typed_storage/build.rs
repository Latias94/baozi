//! Capacity-qualified builders called only inside central retained admission.

use baozi_operation::{CapacityPlan, CapacityPlanError, StorePlanError};

use crate::canonical_scene::store::production::{
    GeometryAttributePayloadAuthoring, GeometryTopologyAuthoring, Presence, ValidatedGeometryBatch,
    ValidatedNodeBatch,
};

use super::*;

#[derive(Debug, thiserror::Error)]
pub(in crate::canonical_scene::store) enum TypedStorageBuildError<E> {
    #[error("typed Scene storage allocation failed")]
    AllocationFailed,
    #[error("typed Scene storage capacity violated its admitted plan")]
    Capacity(#[source] CapacityPlanError),
    #[error("typed Scene storage disagrees with its sealed construction description")]
    Invariant,
    #[error("typed Scene storage construction was interrupted")]
    Control(#[source] E),
}

pub(in crate::canonical_scene::store) type BuildResult<T, E> = Result<T, TypedStorageBuildError<E>>;

/// Action-scoped work adapter; admission and counters stay in operation.
pub(in crate::canonical_scene::store) trait TypedStorageBuildControl {
    type Error;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error>;
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error>;
}

impl<C: TypedStorageBuildControl + ?Sized> TypedStorageBuildControl for &mut C {
    type Error = C::Error;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        C::run_unit(self, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        C::run_string_unit(self, action)
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeStoragePlan {
    nodes: CapacityPlan,
    children: CapacityPlan,
    names: CapacityPlan,
}

impl NodeStoragePlan {
    pub(in crate::canonical_scene::store) fn for_storage(
        source: &NodeStorage,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            nodes: capacity::<StoredNode>(source.nodes.len() as u64)?,
            children: capacity::<StoredNodeIndex>(source.children.len() as u64)?,
            names: capacity::<u8>(source.names.len() as u64)?,
        })
    }

    pub(in crate::canonical_scene::store) fn rewrite<C: TypedStorageBuildControl>(
        self,
        source: &NodeStorage,
        target_row: usize,
        replacement: AuthoredTransform,
        mut control: C,
    ) -> BuildResult<NodeStorage, C::Error> {
        let mut nodes = allocate(self.nodes)?;
        let mut children = allocate(self.children)?;
        let mut names = allocate(self.names)?;
        let target = source
            .nodes
            .get(target_row)
            .ok_or(TypedStorageBuildError::Invariant)?;
        if target_row == 0 {
            return Err(TypedStorageBuildError::Invariant);
        }
        let replacement =
            StoredTransform::from_authored(replacement).ok_or(TypedStorageBuildError::Invariant)?;
        let transform_scalar_count = source
            .transform_scalar_count
            .checked_sub(target.transform.scalar_count())
            .and_then(|count| count.checked_add(replacement.scalar_count()))
            .ok_or(TypedStorageBuildError::Invariant)?;
        for (index, row) in source.nodes.iter().copied().enumerate() {
            let row = if index == target_row {
                StoredNode {
                    transform: replacement,
                    ..row
                }
            } else {
                row
            };
            push(&mut nodes, self.nodes, row, &mut control)?;
        }
        append(&mut children, self.children, &source.children, &mut control)?;
        for &byte in &source.names {
            if names.len() >= self.names.desired_capacity() || names.len() == names.capacity() {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_string_unit(|| names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        finish(&nodes, self.nodes)?;
        finish(&children, self.children)?;
        finish(&names, self.names)?;
        Ok(NodeStorage {
            transform_scalar_count,
            nodes,
            children,
            names,
        })
    }

    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedNodeBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            nodes: capacity::<StoredNode>(batch.canonical_node_count())?,
            children: capacity::<StoredNodeIndex>(batch.authored_node_count() as u64)?,
            names: capacity::<u8>(batch.node_name_byte_count())?,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 3] {
        [self.nodes, self.children, self.names]
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities())
    }

    pub(in crate::canonical_scene::store) fn validation_work(self) -> Result<u64, StorePlanError> {
        count_work([self.nodes, self.children])
    }

    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.names.desired_capacity()
    }

    /// The caller must admit every capacity before invoking this builder.
    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedNodeBatch<'_, R>,
        mut unit: C,
    ) -> BuildResult<NodeStorage, C::Error> {
        let mut storage = NodeStorage {
            transform_scalar_count: usize::try_from(
                batch
                    .matrix_scalar_count()
                    .checked_add(batch.trs_scalar_count())
                    .ok_or(TypedStorageBuildError::Invariant)?,
            )
            .map_err(|_| TypedStorageBuildError::Invariant)?,
            nodes: allocate(self.nodes)?,
            children: allocate(self.children)?,
            names: allocate(self.names)?,
        };
        push(
            &mut storage.nodes,
            self.nodes,
            StoredNode {
                children: TableRange::new(0, batch.source_root_count() as usize),
                transform: StoredTransform::Synthetic,
                name: None,
                camera: None,
                light: None,
            },
            &mut unit,
        )?;
        for &local in batch.canonical_order() {
            let child = batch
                .canonical_index_by_local()
                .get(local)
                .copied()
                .ok_or(TypedStorageBuildError::Invariant)?;
            push(
                &mut storage.children,
                self.children,
                StoredNodeIndex(child),
                &mut unit,
            )?;
        }
        let mut child_offset = batch.source_root_count() as usize;
        for &local in batch.canonical_order() {
            let source = batch
                .input()
                .nodes()
                .get(local)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let name = append_name(&mut storage.names, self.names, source.name(), &mut unit)?;
            let child_count = batch
                .child_count_by_local(local)
                .ok_or(TypedStorageBuildError::Invariant)? as usize;
            let row = StoredNode {
                children: TableRange::new(child_offset, child_count),
                transform: StoredTransform::from_authored(source.authored_transform())
                    .ok_or(TypedStorageBuildError::Invariant)?,
                name,
                camera: match source.camera() {
                    Presence::Absent => None,
                    Presence::Present(id) => Some(id.index()),
                },
                light: match source.light() {
                    Presence::Absent => None,
                    Presence::Present(id) => Some(id.index()),
                },
            };
            push(&mut storage.nodes, self.nodes, row, &mut unit)?;
            child_offset = child_offset
                .checked_add(child_count)
                .ok_or(TypedStorageBuildError::Invariant)?;
        }
        if child_offset != storage.children.len() {
            return Err(TypedStorageBuildError::Invariant);
        }
        finish(&storage.nodes, self.nodes)?;
        finish(&storage.children, self.children)?;
        finish(&storage.names, self.names)?;
        Ok(storage)
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryStoragePlan {
    logical_meshes: CapacityPlan,
    primitives: CapacityPlan,
    attributes: CapacityPlan,
    mesh_instances: CapacityPlan,
    names: CapacityPlan,
    f32_values: CapacityPlan,
    f64_values: CapacityPlan,
    u32_values: CapacityPlan,
    i32_values: CapacityPlan,
    scalar_work: u64,
}

impl GeometryStoragePlan {
    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedGeometryBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let name_bytes = u64::from(batch.logical_mesh_name_byte_count())
            .checked_add(u64::from(batch.attribute_semantic_name_byte_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            logical_meshes: capacity::<StoredLogicalMesh>(u64::from(batch.logical_mesh_count()))?,
            primitives: capacity::<StoredPrimitive>(u64::from(batch.primitive_count()))?,
            attributes: capacity::<StoredAttribute>(u64::from(batch.attribute_count()))?,
            mesh_instances: capacity::<StoredMeshInstance>(u64::from(batch.mesh_instance_count()))?,
            names: capacity::<u8>(name_bytes)?,
            f32_values: capacity::<f32>(batch.f32_scalar_count())?,
            f64_values: capacity::<f64>(batch.f64_scalar_count())?,
            u32_values: capacity::<u32>(batch.u32_scalar_count())?,
            i32_values: capacity::<i32>(batch.i32_scalar_count())?,
            scalar_work: batch
                .f32_scalar_count()
                .checked_add(batch.f64_scalar_count())
                .and_then(|count| count.checked_add(batch.u32_scalar_count()))
                .and_then(|count| count.checked_add(batch.i32_scalar_count()))
                .ok_or(StorePlanError::SizeOverflow)?,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 9] {
        [
            self.logical_meshes,
            self.primitives,
            self.attributes,
            self.mesh_instances,
            self.names,
            self.f32_values,
            self.f64_values,
            self.u32_values,
            self.i32_values,
        ]
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities())
    }

    pub(in crate::canonical_scene::store) fn validation_work(self) -> Result<u64, StorePlanError> {
        let rows = count_work([
            self.logical_meshes,
            self.attributes,
            self.mesh_instances,
            self.primitives,
            self.primitives,
        ])?;
        rows.checked_add(self.scalar_work)
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_count(self) -> u32 {
        self.logical_meshes.desired_capacity() as u32
    }
    pub(in crate::canonical_scene::store) fn primitive_count(self) -> u32 {
        self.primitives.desired_capacity() as u32
    }
    pub(in crate::canonical_scene::store) fn attribute_count(self) -> u32 {
        self.attributes.desired_capacity() as u32
    }
    pub(in crate::canonical_scene::store) fn mesh_instance_count(self) -> u32 {
        self.mesh_instances.desired_capacity() as u32
    }
    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.names.desired_capacity()
    }

    /// The caller must admit every capacity before invoking this builder.
    pub(in crate::canonical_scene::store) fn build<R, NR, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedGeometryBatch<'_, R>,
        nodes: &ValidatedNodeBatch<'_, NR>,
        mut unit: C,
    ) -> BuildResult<GeometryStorage, C::Error> {
        let mut storage = GeometryStorage {
            logical_meshes: allocate(self.logical_meshes)?,
            primitives: allocate(self.primitives)?,
            attributes: allocate(self.attributes)?,
            mesh_instances: allocate(self.mesh_instances)?,
            names: allocate(self.names)?,
            f32_values: allocate(self.f32_values)?,
            f64_values: allocate(self.f64_values)?,
            u32_values: allocate(self.u32_values)?,
            i32_values: allocate(self.i32_values)?,
        };
        for index in 0..batch.attribute_count() as usize {
            let attribute = batch
                .attribute(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let custom = match attribute.semantic() {
                AttributeSemantic::Custom(name) => append_name(
                    &mut storage.names,
                    self.names,
                    Presence::Present(name.as_str()),
                    &mut unit,
                )?
                .ok_or(TypedStorageBuildError::Invariant)?,
                _ => TextRange(TableRange::new(0, 0)),
            };
            let payload =
                self.append_payload(&mut storage, attribute.authored_payload(), &mut unit)?;
            push(
                &mut storage.attributes,
                self.attributes,
                StoredAttribute {
                    semantic: StoredAttributeSemantic::from_authored(attribute.semantic(), custom),
                    domain: attribute.domain(),
                    lanes: attribute.lanes(),
                    element_count: attribute.element_count(),
                    payload,
                },
                &mut unit,
            )?;
        }
        for index in 0..batch.primitive_count() as usize {
            let primitive = batch
                .primitive(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let polygon_boundaries = match primitive.authored_topology() {
                GeometryTopologyAuthoring::Polygons(values) => Some(append(
                    &mut storage.u32_values,
                    self.u32_values,
                    values,
                    &mut unit,
                )?),
                _ => None,
            };
            let indices = match primitive.authored_indices() {
                Presence::Absent => None,
                Presence::Present(values) => Some(append(
                    &mut storage.u32_values,
                    self.u32_values,
                    values,
                    &mut unit,
                )?),
            };
            push(
                &mut storage.primitives,
                self.primitives,
                StoredPrimitive {
                    topology: primitive.topology(),
                    vertex_count: primitive.vertex_count(),
                    corner_count: primitive.corner_count(),
                    face_count: primitive.face_count(),
                    polygon_boundaries,
                    indices,
                    material: primitive.material().map(|id| id.index()),
                    attributes: TableRange::new(
                        primitive.attribute_range().offset() as usize,
                        primitive.attribute_range().count() as usize,
                    ),
                },
                &mut unit,
            )?;
        }
        for mesh in batch.input().logical_meshes() {
            let name = append_name(&mut storage.names, self.names, mesh.name(), &mut unit)?;
            push(
                &mut storage.logical_meshes,
                self.logical_meshes,
                StoredLogicalMesh {
                    primitives: TableRange::new(
                        mesh.primitives().offset() as usize,
                        mesh.primitives().count() as usize,
                    ),
                    name,
                },
                &mut unit,
            )?;
        }
        for instance in batch.input().mesh_instances() {
            let owner = nodes
                .canonical_index_by_local()
                .get(instance.owner().index() as usize)
                .copied()
                .ok_or(TypedStorageBuildError::Invariant)?;
            push(
                &mut storage.mesh_instances,
                self.mesh_instances,
                StoredMeshInstance {
                    owner_node: StoredNodeIndex(owner),
                    logical_mesh: StoredLogicalMeshIndex(instance.logical_mesh().index()),
                },
                &mut unit,
            )?;
        }
        finish(&storage.logical_meshes, self.logical_meshes)?;
        finish(&storage.primitives, self.primitives)?;
        finish(&storage.attributes, self.attributes)?;
        finish(&storage.mesh_instances, self.mesh_instances)?;
        finish(&storage.names, self.names)?;
        finish(&storage.f32_values, self.f32_values)?;
        finish(&storage.f64_values, self.f64_values)?;
        finish(&storage.u32_values, self.u32_values)?;
        finish(&storage.i32_values, self.i32_values)?;
        Ok(storage)
    }

    fn append_payload<C: TypedStorageBuildControl>(
        self,
        storage: &mut GeometryStorage,
        payload: GeometryAttributePayloadAuthoring<'_>,
        unit: &mut C,
    ) -> BuildResult<StoredAttributePayload, C::Error> {
        Ok(match payload {
            GeometryAttributePayloadAuthoring::DenseF32 {
                lanes,
                source_values,
                source_indices,
            } => {
                let start = storage.f32_values.len();
                match source_indices {
                    Presence::Absent => {
                        append(
                            &mut storage.f32_values,
                            self.f32_values,
                            source_values,
                            unit,
                        )?;
                    }
                    Presence::Present(indices) => {
                        for &index in indices {
                            let offset = (index as usize)
                                .checked_mul(usize::from(lanes))
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            let end = offset
                                .checked_add(usize::from(lanes))
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            let values = source_values
                                .get(offset..end)
                                .ok_or(TypedStorageBuildError::Invariant)?;
                            append(&mut storage.f32_values, self.f32_values, values, unit)?;
                        }
                    }
                }
                StoredAttributePayload::DenseF32(TableRange::new(
                    start,
                    storage.f32_values.len() - start,
                ))
            }
            GeometryAttributePayloadAuthoring::DenseF64 { values, .. } => {
                StoredAttributePayload::DenseF64(append(
                    &mut storage.f64_values,
                    self.f64_values,
                    values,
                    unit,
                )?)
            }
            GeometryAttributePayloadAuthoring::DenseU32 { values, .. } => {
                StoredAttributePayload::DenseU32(append(
                    &mut storage.u32_values,
                    self.u32_values,
                    values,
                    unit,
                )?)
            }
            GeometryAttributePayloadAuthoring::DenseI32 { values, .. } => {
                StoredAttributePayload::DenseI32(append(
                    &mut storage.i32_values,
                    self.i32_values,
                    values,
                    unit,
                )?)
            }
            GeometryAttributePayloadAuthoring::SparseF32 {
                indices,
                values,
                default,
                ..
            } => StoredAttributePayload::SparseF32 {
                indices: append(&mut storage.u32_values, self.u32_values, indices, unit)?,
                values: append(&mut storage.f32_values, self.f32_values, values, unit)?,
                default: append(&mut storage.f32_values, self.f32_values, default, unit)?,
            },
            GeometryAttributePayloadAuthoring::SparseU32 {
                indices,
                values,
                default,
                ..
            } => StoredAttributePayload::SparseU32 {
                indices: append(&mut storage.u32_values, self.u32_values, indices, unit)?,
                values: append(&mut storage.u32_values, self.u32_values, values, unit)?,
                default: append(&mut storage.u32_values, self.u32_values, default, unit)?,
            },
            GeometryAttributePayloadAuthoring::SparseI32 {
                indices,
                values,
                default,
                ..
            } => StoredAttributePayload::SparseI32 {
                indices: append(&mut storage.u32_values, self.u32_values, indices, unit)?,
                values: append(&mut storage.i32_values, self.i32_values, values, unit)?,
                default: append(&mut storage.i32_values, self.i32_values, default, unit)?,
            },
        })
    }
}

pub(in crate::canonical_scene::store) fn capacity<T>(
    count: u64,
) -> Result<CapacityPlan, StorePlanError> {
    CapacityPlan::for_type::<T>(
        0,
        0,
        usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
    )
    .map_err(StorePlanError::Capacity)
}

pub(in crate::canonical_scene::store) fn count_work<const N: usize>(
    plans: [CapacityPlan; N],
) -> Result<u64, StorePlanError> {
    plans.into_iter().try_fold(0_u64, |sum, plan| {
        sum.checked_add(
            u64::try_from(plan.desired_capacity()).map_err(|_| StorePlanError::SizeOverflow)?,
        )
        .ok_or(StorePlanError::SizeOverflow)
    })
}

pub(in crate::canonical_scene::store) fn allocate<T, E>(
    plan: CapacityPlan,
) -> BuildResult<Vec<T>, E> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| TypedStorageBuildError::AllocationFailed)?;
    plan.finish(0, values.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(values)
}

pub(in crate::canonical_scene::store) fn push<T, C: TypedStorageBuildControl>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
    value: T,
    unit: &mut C,
) -> BuildResult<(), C::Error> {
    if values.len() >= plan.desired_capacity() || values.len() == values.capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    unit.run_unit(|| values.push(value))
        .map_err(TypedStorageBuildError::Control)?;
    Ok(())
}

pub(super) fn append<T: Copy, C: TypedStorageBuildControl>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
    source: &[T],
    unit: &mut C,
) -> BuildResult<TableRange<T>, C::Error> {
    let start = values.len();
    for &value in source {
        push(values, plan, value, unit)?;
    }
    Ok(TableRange::new(start, source.len()))
}

pub(super) fn append_name<C: TypedStorageBuildControl>(
    text: &mut Vec<u8>,
    plan: CapacityPlan,
    name: Presence<&str>,
    unit: &mut C,
) -> BuildResult<Option<TextRange>, C::Error> {
    let Presence::Present(name) = name else {
        return Ok(None);
    };
    let start = text.len();
    for byte in name.bytes() {
        if text.len() >= plan.desired_capacity() || text.len() == text.capacity() {
            return Err(TypedStorageBuildError::Invariant);
        }
        unit.run_string_unit(|| text.push(byte))
            .map_err(TypedStorageBuildError::Control)?;
    }
    Ok(Some(TextRange(TableRange::new(start, name.len()))))
}

pub(in crate::canonical_scene::store) fn finish<T, E>(
    values: &Vec<T>,
    plan: CapacityPlan,
) -> BuildResult<(), E> {
    if values.len() != plan.desired_capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    plan.finish(0, values.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(())
}
