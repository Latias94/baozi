//! Source-bound coordinate conversion of authored node transforms.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::build::{
    NodeStoragePlan, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, finish,
};
use super::super::validation::TypedStorageValidationControl;
use super::super::{NodeStorage, StoredNode, StoredTransform};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum NodeSpacePlanError<E> {
    Control(E),
    Numeric {
        node: usize,
        source: NumericConversionError,
    },
    Plan(StorePlanError),
    Invariant,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSpacePlan<'a> {
    source: &'a NodeStorage,
    conversion: SpaceConversion,
    shape: NodeStoragePlan,
    work: u64,
}

impl<'a> NodeSpacePlan<'a> {
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a NodeStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, NodeSpacePlanError<C::Error>> {
        let shape = NodeStoragePlan::for_storage(source).map_err(NodeSpacePlanError::Plan)?;
        let work = shape.work().map_err(NodeSpacePlanError::Plan)?;
        for (row, node) in source.nodes.iter().enumerate() {
            let valid = control
                .run_unit(|| {
                    if row == 0 {
                        return matches!(node.transform, StoredTransform::Synthetic)
                            .then_some(())
                            .ok_or(NodeSpacePlanError::Invariant);
                    }
                    let authored = node
                        .transform
                        .authored()
                        .ok_or(NodeSpacePlanError::Invariant)?;
                    conversion
                        .authored_transform(authored)
                        .map(|_| ())
                        .map_err(|source| NodeSpacePlanError::Numeric {
                            node: row - 1,
                            source,
                        })
                })
                .map_err(NodeSpacePlanError::Control)?;
            valid?;
        }
        Ok(Self {
            source,
            conversion,
            shape,
            work,
        })
    }

    pub(in crate::canonical_scene::store) fn capacities(self) -> [CapacityPlan; 3] {
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

    pub(in crate::canonical_scene::store) const fn conversion(self) -> SpaceConversion {
        self.conversion
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> Result<NodeStorage, TypedStorageBuildError<C::Error>> {
        let [node_capacity, child_capacity, name_capacity] = self.capacities();
        let mut nodes = allocate(node_capacity)?;
        let mut children = allocate(child_capacity)?;
        let mut names = allocate(name_capacity)?;
        for (index, &row) in self.source.nodes.iter().enumerate() {
            if nodes.len() >= node_capacity.desired_capacity() || nodes.len() == nodes.capacity() {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_unit(|| {
                    let row = if index == 0 {
                        row
                    } else {
                        let authored = row
                            .transform
                            .authored()
                            .ok_or(TypedStorageBuildError::Invariant)?;
                        let converted = self
                            .conversion
                            .authored_transform(authored)
                            .map_err(|_| TypedStorageBuildError::Invariant)?;
                        StoredNode {
                            transform: StoredTransform::from_authored(converted)
                                .ok_or(TypedStorageBuildError::Invariant)?,
                            ..row
                        }
                    };
                    nodes.push(row);
                    Ok(())
                })
                .map_err(TypedStorageBuildError::Control)??;
        }
        append(
            &mut children,
            child_capacity,
            &self.source.children,
            control,
        )?;
        for &byte in &self.source.names {
            if names.len() >= name_capacity.desired_capacity() || names.len() == names.capacity() {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_string_unit(|| names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        finish(&nodes, node_capacity)?;
        finish(&children, child_capacity)?;
        finish(&names, name_capacity)?;
        Ok(NodeStorage {
            transform_scalar_count: self.source.transform_scalar_count,
            nodes,
            children,
            names,
        })
    }

    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &NodeStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if actual.nodes.len() != self.source.nodes.len()
            || actual.children.len() != self.source.children.len()
            || actual.names.len() != self.source.names.len()
            || actual.transform_scalar_count != self.source.transform_scalar_count
        {
            return Ok(false);
        }
        for (index, (source, actual)) in self.source.nodes.iter().zip(&actual.nodes).enumerate() {
            if control.run_unit(|| {
                if source.children != actual.children
                    || source.name != actual.name
                    || source.camera != actual.camera
                    || source.light != actual.light
                {
                    return false;
                }
                if index == 0 {
                    return matches!(source.transform, StoredTransform::Synthetic)
                        && matches!(actual.transform, StoredTransform::Synthetic);
                }
                source
                    .transform
                    .authored()
                    .and_then(|source| self.conversion.authored_transform(source).ok())
                    .zip(actual.transform.authored())
                    .is_some_and(|(expected, actual)| expected.has_same_authored_bits(actual))
            })? != Some(true)
            {
                return Ok(false);
            }
        }
        for (&source, &actual) in self.source.children.iter().zip(&actual.children) {
            if control.run_unit(|| source == actual)? != Some(true) {
                return Ok(false);
            }
        }
        for (&source, &actual) in self.source.names.iter().zip(&actual.names) {
            if control.run_unit(|| source == actual)? != Some(true) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
