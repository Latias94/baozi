use super::{
    AuthoredTransform, EntityAllocation, EntityCollectionError, EntityName,
    EntityRetainedFootprintError, MAX_ENTITY_EXTENSION_ENTRIES, MAX_NODE_CHILDREN,
    try_collect_bounded, unwrap_uncontrolled_retained_footprint,
};
use crate::extension::{
    ControlledRetainedFootprintError, ExtensionEntry, ExtensionEntryValidationError,
    extension_entries_retained_allocation_bytes_controlled, validate_unique_extension_entry_keys,
};
use crate::identity::{CameraIndex, LightIndex, NodeIndex};
use crate::math::Mat4;
use crate::provenance::ProvenanceAttachments;
use crate::shared_storage::{SharedStorage, SharedStorageError, SharedStorageFootprintError};
use std::convert::Infallible;
use std::fmt;
use std::mem::size_of;
use thiserror::Error;

/// One hierarchy node.
///
/// Parent and mesh-instance lookups are derived scene indexes. The only
/// canonical hierarchy edge stored here is the ordered child list.
#[derive(Debug, PartialEq)]
struct NodePayload {
    name: Option<EntityName>,
    children: Vec<NodeIndex>,
    camera: Option<CameraIndex>,
    light: Option<LightIndex>,
    provenance: ProvenanceAttachments,
    extensions: Vec<ExtensionEntry>,
}

pub struct Node {
    authored_transform: AuthoredTransform,
    payload: SharedStorage<NodePayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodePreparedFootprint {
    value_storage_bytes: u64,
    nested_payload_bytes: u64,
}

pub(crate) struct NodePreparedParts {
    pub(crate) name: Option<EntityName>,
    pub(crate) authored_transform: AuthoredTransform,
    pub(crate) children: Vec<NodeIndex>,
    pub(crate) camera: Option<CameraIndex>,
    pub(crate) light: Option<LightIndex>,
    pub(crate) provenance: ProvenanceAttachments,
    pub(crate) extensions: Vec<ExtensionEntry>,
}

impl NodePreparedFootprint {
    pub(crate) const fn value_storage_bytes(self) -> u64 {
        self.value_storage_bytes
    }

    pub(crate) const fn nested_payload_bytes(self) -> u64 {
        self.nested_payload_bytes
    }
}

impl Node {
    /// Creates a node and validates all leaf-level invariants.
    pub fn try_new(
        name: Option<EntityName>,
        authored_transform: AuthoredTransform,
        children: impl IntoIterator<Item = NodeIndex>,
        camera: Option<CameraIndex>,
        light: Option<LightIndex>,
        provenance: impl Into<ProvenanceAttachments>,
        extensions: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, NodeError> {
        let children =
            try_collect_bounded(children, MAX_NODE_CHILDREN, EntityAllocation::NodeChildren)?;
        let extensions = try_collect_bounded(
            extensions,
            MAX_ENTITY_EXTENSION_ENTRIES,
            EntityAllocation::Extensions,
        )?;
        let payload =
            prepare_node_payload(name, children, camera, light, provenance.into(), extensions)?;
        Self::try_from_payload(authored_transform, payload)
    }

    pub(crate) fn try_from_prepared_parts(
        parts: NodePreparedParts,
    ) -> Result<(Self, NodePreparedFootprint), NodeError> {
        let NodePreparedParts {
            name,
            authored_transform,
            children,
            camera,
            light,
            provenance,
            extensions,
        } = parts;
        let payload = prepare_node_payload(name, children, camera, light, provenance, extensions)?;
        let nested_payload_bytes = u64::try_from(node_payload_nested_allocation_bytes(&payload)?)
            .map_err(|_| NodeError::PayloadAccountingOverflow)?;
        let node = Self::try_from_payload(authored_transform, payload)?;
        let value_storage_bytes = u64::try_from(node.payload.retained_allocation_bytes())
            .map_err(|_| NodeError::PayloadAccountingOverflow)?;

        Ok((
            node,
            NodePreparedFootprint {
                value_storage_bytes,
                nested_payload_bytes,
            },
        ))
    }

    pub(crate) fn value_storage_layout_bytes() -> Result<u64, NodeError> {
        SharedStorage::<NodePayload>::checked_layout_footprint()
            .map_err(|SharedStorageFootprintError::LayoutOverflow| {
                NodeError::PayloadAccountingOverflow
            })
            .and_then(|bytes| {
                u64::try_from(bytes).map_err(|_| NodeError::PayloadAccountingOverflow)
            })
    }

    fn try_from_payload(
        authored_transform: AuthoredTransform,
        payload: NodePayload,
    ) -> Result<Self, NodeError> {
        let payload = SharedStorage::try_new(payload).map_err(|error| match error {
            SharedStorageError::AllocationFailed => NodeError::PayloadAllocationFailed,
            SharedStorageError::AccountingOverflow => NodeError::PayloadAccountingOverflow,
        })?;
        Ok(Self {
            authored_transform,
            payload,
        })
    }

    pub fn name(&self) -> Option<&EntityName> {
        self.payload.as_ref().name.as_ref()
    }

    pub const fn authored_transform(&self) -> AuthoredTransform {
        self.authored_transform
    }

    pub fn local_matrix(&self) -> Mat4 {
        self.authored_transform.local_matrix()
    }

    pub fn children(&self) -> impl ExactSizeIterator<Item = NodeIndex> + '_ {
        self.payload.as_ref().children.iter().copied()
    }

    pub(crate) fn child_at(&self, index: usize) -> Option<NodeIndex> {
        self.payload.as_ref().children.get(index).copied()
    }

    pub fn camera_index(&self) -> Option<CameraIndex> {
        self.payload.as_ref().camera
    }

    pub fn light_index(&self) -> Option<LightIndex> {
        self.payload.as_ref().light
    }

    pub fn provenance(&self) -> &ProvenanceAttachments {
        &self.payload.as_ref().provenance
    }

    pub fn extensions(&self) -> impl ExactSizeIterator<Item = &ExtensionEntry> {
        self.payload.as_ref().extensions.iter()
    }

    pub(crate) fn with_authored_transform(&self, authored_transform: AuthoredTransform) -> Self {
        Self {
            authored_transform,
            payload: self.payload.clone(),
        }
    }

    pub(crate) fn fork(&self) -> Self {
        self.with_authored_transform(self.authored_transform)
    }

    #[cfg(test)]
    pub(crate) fn shares_payload_with(&self, other: &Self) -> bool {
        self.payload.ptr_eq(&other.payload)
    }

    pub(crate) fn retained_allocation_bytes(&self) -> Result<usize, EntityRetainedFootprintError> {
        unwrap_uncontrolled_retained_footprint(
            self.retained_allocation_bytes_controlled(&mut || Ok::<(), Infallible>(())),
        )
    }

    pub(crate) fn retained_allocation_bytes_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<usize, ControlledRetainedFootprintError<E>> {
        let payload = self.payload.as_ref();
        self.payload
            .retained_allocation_bytes()
            .checked_add(node_payload_nested_allocation_bytes_controlled(
                payload,
                charge_work,
            )?)
            .ok_or(ControlledRetainedFootprintError::AccountingOverflow)
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let payload = self.payload.as_ref();
        formatter
            .debug_struct("Node")
            .field("name", &payload.name)
            .field("authored_transform", &self.authored_transform)
            .field("children", &payload.children)
            .field("camera", &payload.camera)
            .field("light", &payload.light)
            .field("provenance", &payload.provenance)
            .field("extensions", &payload.extensions)
            .finish()
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.authored_transform == other.authored_transform
            && self.payload.as_ref() == other.payload.as_ref()
    }
}

/// Why a node could not enter canonical storage.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeError {
    #[error(transparent)]
    Extension(#[from] ExtensionEntryValidationError),

    #[error("node storage limit exceeded for {resource:?}; maximum {maximum}")]
    LimitExceeded {
        resource: EntityAllocation,
        maximum: usize,
    },

    #[error("node allocation failed for {resource:?}")]
    AllocationFailed { resource: EntityAllocation },
    #[error("node shared payload allocation failed")]
    PayloadAllocationFailed,
    #[error("node shared payload allocation accounting overflowed")]
    PayloadAccountingOverflow,
}

fn prepare_node_payload(
    name: Option<EntityName>,
    children: Vec<NodeIndex>,
    camera: Option<CameraIndex>,
    light: Option<LightIndex>,
    provenance: ProvenanceAttachments,
    extensions: Vec<ExtensionEntry>,
) -> Result<NodePayload, NodeError> {
    if children.len() > MAX_NODE_CHILDREN {
        return Err(NodeError::LimitExceeded {
            resource: EntityAllocation::NodeChildren,
            maximum: MAX_NODE_CHILDREN,
        });
    }
    if extensions.len() > MAX_ENTITY_EXTENSION_ENTRIES {
        return Err(NodeError::LimitExceeded {
            resource: EntityAllocation::Extensions,
            maximum: MAX_ENTITY_EXTENSION_ENTRIES,
        });
    }
    validate_unique_extension_entry_keys(&extensions)?;
    Ok(NodePayload {
        name,
        children,
        camera,
        light,
        provenance,
        extensions,
    })
}

fn node_payload_nested_allocation_bytes(payload: &NodePayload) -> Result<usize, NodeError> {
    unwrap_uncontrolled_retained_footprint(node_payload_nested_allocation_bytes_controlled(
        payload,
        &mut || Ok::<(), Infallible>(()),
    ))
    .map_err(|EntityRetainedFootprintError::Overflow| NodeError::PayloadAccountingOverflow)
}

fn node_payload_nested_allocation_bytes_controlled<E>(
    payload: &NodePayload,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, ControlledRetainedFootprintError<E>> {
    let child_bytes = payload
        .children
        .capacity()
        .checked_mul(size_of::<NodeIndex>())
        .ok_or(ControlledRetainedFootprintError::AccountingOverflow)?;
    let extension_bytes =
        extension_entries_retained_allocation_bytes_controlled(&payload.extensions, charge_work)?;
    payload
        .name
        .as_ref()
        .map_or(0, EntityName::capacity)
        .checked_add(child_bytes)
        .and_then(|bytes| bytes.checked_add(payload.provenance.retained_allocation_bytes()))
        .and_then(|bytes| bytes.checked_add(extension_bytes))
        .ok_or(ControlledRetainedFootprintError::AccountingOverflow)
}

impl From<EntityCollectionError> for NodeError {
    fn from(error: EntityCollectionError) -> Self {
        match error {
            EntityCollectionError::Extension(error) => Self::Extension(error),
            EntityCollectionError::LimitExceeded { resource, maximum } => {
                Self::LimitExceeded { resource, maximum }
            }
            EntityCollectionError::AllocationFailed { resource } => {
                Self::AllocationFailed { resource }
            }
        }
    }
}

#[cfg(test)]
mod tests;
