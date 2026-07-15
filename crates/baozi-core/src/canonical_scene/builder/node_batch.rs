use super::*;
use crate::identity::SceneStorageIndex;
use std::convert::Infallible;
use std::fmt;
use std::marker::PhantomData;

#[derive(Debug)]
struct CanonicalNodeBatchBrand {
    // Distinct live batches need distinct addresses, so the brand must not be a ZST.
    _non_zst_identity: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CanonicalNodeSlot<'batch> {
    index: NodeIndex,
    brand: &'batch CanonicalNodeBatchBrand,
    invariant: PhantomData<fn(&'batch CanonicalNodeBatchBrand) -> &'batch CanonicalNodeBatchBrand>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CanonicalNodeBatchPlan {
    generation: SceneGenerationId,
    collection: CapacityPlan,
    value_storage_bytes_per_node: u64,
    value_storage_envelope_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalNodeBatchAllocationEvidence {
    collection_retained_allocation_bytes: u64,
    value_storage_retained_allocation_bytes: u64,
    nested_payload_retained_allocation_bytes: u64,
}

impl CanonicalNodeBatchAllocationEvidence {
    pub(crate) const fn collection_retained_allocation_bytes(self) -> u64 {
        self.collection_retained_allocation_bytes
    }

    pub(crate) const fn value_storage_retained_allocation_bytes(self) -> u64 {
        self.value_storage_retained_allocation_bytes
    }

    pub(crate) const fn nested_payload_retained_allocation_bytes(self) -> u64 {
        self.nested_payload_retained_allocation_bytes
    }
}

impl CanonicalNodeBatchPlan {
    pub(crate) fn collection_envelope_bytes(self) -> Result<u64, CanonicalSceneBuildError> {
        self.collection.envelope_delta_bytes().map_err(|cause| {
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            }
        })
    }

    pub(crate) const fn value_storage_envelope_bytes(self) -> u64 {
        self.value_storage_envelope_bytes
    }

    pub(crate) const fn node_count(self) -> usize {
        self.collection.desired_capacity()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CanonicalNodeChildrenPlan<'batch> {
    brand: &'batch CanonicalNodeBatchBrand,
    capacity: CapacityPlan,
    invariant: PhantomData<fn(&'batch CanonicalNodeBatchBrand) -> &'batch CanonicalNodeBatchBrand>,
}

impl CanonicalNodeChildrenPlan<'_> {
    pub(crate) fn envelope_bytes(self) -> Result<u64, CanonicalSceneBuildError> {
        self.capacity.envelope_delta_bytes().map_err(|cause| {
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            }
        })
    }

    pub(crate) const fn count(self) -> usize {
        self.capacity.desired_capacity()
    }
}

pub(crate) struct CanonicalPreparedNodeChildren<'batch> {
    indices: Vec<NodeIndex>,
    brand: &'batch CanonicalNodeBatchBrand,
    retained_allocation_bytes: u64,
    invariant: PhantomData<fn(&'batch CanonicalNodeBatchBrand) -> &'batch CanonicalNodeBatchBrand>,
}

impl CanonicalPreparedNodeChildren<'_> {
    pub(crate) const fn retained_allocation_bytes(&self) -> u64 {
        self.retained_allocation_bytes
    }
}

pub(crate) struct CanonicalNodeDescriptor<'batch> {
    name: Option<EntityName>,
    authored_transform: AuthoredTransform,
    children: CanonicalPreparedNodeChildren<'batch>,
    camera: Option<CameraIndex>,
    light: Option<LightIndex>,
    provenance: ProvenanceAttachments,
    extensions: Vec<ExtensionEntry>,
}

impl<'batch> CanonicalNodeDescriptor<'batch> {
    fn into_prepared_parts(self) -> NodePreparedParts {
        let Self {
            name,
            authored_transform,
            children,
            camera,
            light,
            provenance,
            extensions,
        } = self;
        NodePreparedParts {
            name,
            authored_transform,
            children: children.indices,
            camera,
            light,
            provenance,
            extensions,
        }
    }
}

struct NodeBatchStaging<C> {
    // Staged storage must drop before the companion that covers it.
    nodes: Vec<Node>,
    #[cfg(test)]
    storage_drop_probe: Option<NodeBatchStorageDropProbe>,
    companion: C,
    root: Option<NodeIndex>,
    collection_retained_allocation_bytes: u64,
    value_storage_retained_allocation_bytes: u64,
    nested_payload_retained_allocation_bytes: u64,
}

#[cfg(test)]
struct NodeBatchStorageDropProbe(std::sync::Arc<std::sync::atomic::AtomicU8>);

#[cfg(test)]
impl Drop for NodeBatchStorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.swap(1, std::sync::atomic::Ordering::SeqCst),
            0,
            "staged node storage must begin the coupled drop sequence"
        );
    }
}

pub(crate) struct CanonicalNodeBatch<'batch, C = ()> {
    brand: &'batch CanonicalNodeBatchBrand,
    staging: &'batch mut NodeBatchStaging<C>,
    node_count: usize,
    value_storage_bytes_per_node: u64,
    value_storage_envelope_bytes: u64,
    invariant: PhantomData<fn(&'batch CanonicalNodeBatchBrand) -> &'batch CanonicalNodeBatchBrand>,
}

impl<'batch, C> CanonicalNodeBatch<'batch, C> {
    pub(crate) fn slot(
        &self,
        batch_ordinal: usize,
    ) -> Result<CanonicalNodeSlot<'batch>, CanonicalSceneBuildError> {
        if batch_ordinal >= self.node_count {
            return Err(CanonicalSceneBuildError::NodeBatchSlotOutOfRange {
                actual: batch_ordinal,
                node_count: self.node_count,
            });
        }
        Ok(CanonicalNodeSlot {
            index: NodeIndex::try_from(batch_ordinal)?,
            brand: self.brand,
            invariant: PhantomData,
        })
    }

    pub(crate) fn node_children_plan(
        &self,
        count: usize,
    ) -> Result<CanonicalNodeChildrenPlan<'batch>, CanonicalSceneBuildError> {
        if count > crate::entity::MAX_NODE_CHILDREN {
            return Err(NodeError::LimitExceeded {
                resource: crate::entity::EntityAllocation::NodeChildren,
                maximum: crate::entity::MAX_NODE_CHILDREN,
            }
            .into());
        }
        let capacity = CapacityPlan::for_type::<NodeIndex>(0, 0, count).map_err(|cause| {
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            }
        })?;
        Ok(CanonicalNodeChildrenPlan {
            brand: self.brand,
            capacity,
            invariant: PhantomData,
        })
    }

    pub(crate) fn prepare_node_children<I>(
        &self,
        plan: CanonicalNodeChildrenPlan<'batch>,
        children: I,
    ) -> Result<CanonicalPreparedNodeChildren<'batch>, CanonicalSceneBuildError>
    where
        I: IntoIterator<Item = CanonicalNodeSlot<'batch>>,
        I::IntoIter: ExactSizeIterator,
    {
        if !std::ptr::eq(plan.brand, self.brand) {
            return Err(CanonicalSceneBuildError::ForeignNodeBatchSlot);
        }
        let expected_count = plan.count();
        let children = children.into_iter();
        let actual_count = children.len();
        if actual_count != expected_count {
            return Err(CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
                expected: expected_count,
                actual: actual_count,
            });
        }
        plan.capacity.verify_preflight(0, 0).map_err(|cause| {
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            }
        })?;
        let mut indices = Vec::new();
        indices
            .try_reserve_exact(plan.capacity.allocator_additional())
            .map_err(|_| CanonicalSceneBuildError::AllocationFailed {
                collection: CanonicalSceneCollection::Nodes,
            })?;
        let retained_allocation_bytes = plan
            .capacity
            .finish(indices.len(), indices.capacity())
            .map_err(|cause| CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            })?;
        for child in children {
            let actual = indices.len().checked_add(1).ok_or(
                CanonicalSceneBuildError::CollectionSizeOverflow {
                    collection: CanonicalSceneCollection::Nodes,
                },
            )?;
            if actual > expected_count {
                return Err(CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
                    expected: expected_count,
                    actual,
                });
            }
            self.validate_slot(child)?;
            indices.push(child.index);
        }
        if indices.len() != expected_count {
            return Err(CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
                expected: expected_count,
                actual: indices.len(),
            });
        }
        Ok(CanonicalPreparedNodeChildren {
            indices,
            brand: self.brand,
            retained_allocation_bytes,
            invariant: PhantomData,
        })
    }

    pub(crate) fn initialize_node(
        &mut self,
        slot: CanonicalNodeSlot<'batch>,
        descriptor: CanonicalNodeDescriptor<'batch>,
    ) -> Result<(), CanonicalSceneBuildError> {
        self.validate_slot(slot)?;
        let expected = self.staging.nodes.len();
        let actual = slot.index.try_to_usize()?;
        if actual != expected {
            return Err(CanonicalSceneBuildError::NodeBatchInitializationOrder {
                expected,
                actual,
            });
        }
        if !std::ptr::eq(descriptor.children.brand, self.brand) {
            return Err(CanonicalSceneBuildError::ForeignNodeBatchSlot);
        }
        let next_node_count = self.staging.nodes.len().checked_add(1).ok_or(
            CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            },
        )?;
        if next_node_count > self.staging.nodes.capacity() {
            return Err(CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause: CapacityPlanError::InvalidState {
                    len: next_node_count,
                    capacity: self.staging.nodes.capacity(),
                },
            });
        }

        let expected_value_storage = self
            .staging
            .value_storage_retained_allocation_bytes
            .checked_add(self.value_storage_bytes_per_node)
            .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            })?;
        if expected_value_storage > self.value_storage_envelope_bytes {
            return Err(CanonicalSceneBuildError::PreparedAllocationUnderestimated {
                collection: CanonicalSceneCollection::Nodes,
                allocation: CanonicalPreparedAllocation::ValueStorage,
                admitted: self.value_storage_envelope_bytes,
                actual: expected_value_storage,
            });
        }

        let (node, footprint) = Node::try_from_prepared_parts(descriptor.into_prepared_parts())?;
        let value_storage_retained_allocation_bytes = self
            .staging
            .value_storage_retained_allocation_bytes
            .checked_add(footprint.value_storage_bytes())
            .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            })?;
        if value_storage_retained_allocation_bytes > self.value_storage_envelope_bytes {
            return Err(CanonicalSceneBuildError::PreparedAllocationUnderestimated {
                collection: CanonicalSceneCollection::Nodes,
                allocation: CanonicalPreparedAllocation::ValueStorage,
                admitted: self.value_storage_envelope_bytes,
                actual: value_storage_retained_allocation_bytes,
            });
        }
        let nested_payload_retained_allocation_bytes = self
            .staging
            .nested_payload_retained_allocation_bytes
            .checked_add(footprint.nested_payload_bytes())
            .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            })?;

        self.staging.nodes.push(node);
        self.staging.value_storage_retained_allocation_bytes =
            value_storage_retained_allocation_bytes;
        self.staging.nested_payload_retained_allocation_bytes =
            nested_payload_retained_allocation_bytes;
        Ok(())
    }

    pub(crate) fn set_root(
        &mut self,
        root: CanonicalNodeSlot<'batch>,
    ) -> Result<(), CanonicalSceneBuildError> {
        self.validate_slot(root)?;
        if self.staging.root.is_some() {
            return Err(CanonicalSceneBuildError::DuplicateNodeBatchRoot);
        }
        self.staging.root = Some(root.index);
        Ok(())
    }

    pub(crate) fn companion_mut(&mut self) -> &mut C {
        &mut self.staging.companion
    }

    #[cfg(test)]
    fn set_storage_drop_probe_for_test(
        &mut self,
        probe: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        self.staging.storage_drop_probe = Some(NodeBatchStorageDropProbe(probe));
    }

    fn validate_slot(
        &self,
        slot: CanonicalNodeSlot<'batch>,
    ) -> Result<(), CanonicalSceneBuildError> {
        if !std::ptr::eq(slot.brand, self.brand) {
            return Err(CanonicalSceneBuildError::ForeignNodeBatchSlot);
        }
        let batch_ordinal = slot.index.try_to_usize()?;
        if batch_ordinal >= self.node_count {
            return Err(CanonicalSceneBuildError::NodeBatchSlotOutOfRange {
                actual: batch_ordinal,
                node_count: self.node_count,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn foreign_slot_for_test(&self, batch_ordinal: usize) -> CanonicalNodeSlot<'batch> {
        static FOREIGN_BRAND: CanonicalNodeBatchBrand = CanonicalNodeBatchBrand {
            _non_zst_identity: 1,
        };
        CanonicalNodeSlot {
            index: NodeIndex::try_from(batch_ordinal).expect("test slot should fit"),
            brand: &FOREIGN_BRAND,
            invariant: PhantomData,
        }
    }
}

pub(crate) struct PreparedCanonicalNodeBatch<'owner, C = ()> {
    // Published storage must drop before the companion that covers it.
    staging: NodeBatchStaging<C>,
    owner: &'owner mut CanonicalSceneBuilder,
    root: NodeIndex,
}

impl<C> fmt::Debug for PreparedCanonicalNodeBatch<'_, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedCanonicalNodeBatch")
            .field("node_count", &self.staging.nodes.len())
            .field("root", &self.root)
            .field(
                "collection_retained_allocation_bytes",
                &self.staging.collection_retained_allocation_bytes,
            )
            .field(
                "value_storage_retained_allocation_bytes",
                &self.staging.value_storage_retained_allocation_bytes,
            )
            .field(
                "nested_payload_retained_allocation_bytes",
                &self.staging.nested_payload_retained_allocation_bytes,
            )
            .finish_non_exhaustive()
    }
}

impl<C> PreparedCanonicalNodeBatch<'_, C> {
    pub(crate) const fn allocation_evidence(&self) -> CanonicalNodeBatchAllocationEvidence {
        CanonicalNodeBatchAllocationEvidence {
            collection_retained_allocation_bytes: self.staging.collection_retained_allocation_bytes,
            value_storage_retained_allocation_bytes: self
                .staging
                .value_storage_retained_allocation_bytes,
            nested_payload_retained_allocation_bytes: self
                .staging
                .nested_payload_retained_allocation_bytes,
        }
    }

    pub(crate) const fn collection_retained_allocation_bytes(&self) -> u64 {
        self.staging.collection_retained_allocation_bytes
    }

    pub(crate) const fn value_storage_retained_allocation_bytes(&self) -> u64 {
        self.staging.value_storage_retained_allocation_bytes
    }

    pub(crate) const fn nested_payload_retained_allocation_bytes(&self) -> u64 {
        self.staging.nested_payload_retained_allocation_bytes
    }

    pub(crate) const fn companion(&self) -> &C {
        &self.staging.companion
    }

    pub(crate) fn companion_mut(&mut self) -> &mut C {
        &mut self.staging.companion
    }

    /// Transfers the companion before entering the infallible storage-publication region.
    ///
    /// On success, `publish_companion` must leave the companion safe to drop because the node
    /// storage becomes builder-owned immediately afterward. On error or panic, the prepared node
    /// storage remains coupled to the companion and drops first.
    pub(crate) fn try_commit_coupled<E>(
        mut self,
        publish_companion: impl FnOnce(&mut C, CanonicalNodeBatchAllocationEvidence) -> Result<(), E>,
    ) -> Result<CanonicalNodeBatchCommit, E> {
        let evidence = self.allocation_evidence();
        publish_companion(&mut self.staging.companion, evidence)?;
        Ok(self.commit_after_companion_publication())
    }

    fn commit_after_companion_publication(self) -> CanonicalNodeBatchCommit {
        let Self {
            staging,
            owner,
            root,
        } = self;
        let NodeBatchStaging {
            nodes,
            #[cfg(test)]
            storage_drop_probe,
            companion,
            root: _,
            collection_retained_allocation_bytes: _,
            value_storage_retained_allocation_bytes: _,
            nested_payload_retained_allocation_bytes: _,
        } = staging;
        let generation = owner.data.generation;
        let node_count = nodes.len();
        owner.data.nodes = nodes;
        owner.data.root = Some(root);
        #[cfg(test)]
        drop(storage_drop_probe);
        drop(companion);
        CanonicalNodeBatchCommit {
            generation,
            node_count,
            root,
        }
    }

    #[cfg(test)]
    fn nodes_ptr_for_test(&self) -> *const Node {
        self.staging.nodes.as_ptr()
    }
}

impl PreparedCanonicalNodeBatch<'_, ()> {
    pub(crate) fn commit(self) -> CanonicalNodeBatchCommit {
        match self.try_commit_coupled(|(), _evidence| Ok::<(), Infallible>(())) {
            Ok(commit) => commit,
            Err(error) => match error {},
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CanonicalNodeBatchCommit {
    generation: SceneGenerationId,
    node_count: usize,
    root: NodeIndex,
}

impl CanonicalNodeBatchCommit {
    pub(crate) const fn root(self) -> NodeId {
        self.root.handle(self.generation)
    }

    pub(crate) fn node_id(self, batch_ordinal: usize) -> Option<NodeId> {
        if batch_ordinal >= self.node_count {
            return None;
        }
        NodeIndex::try_from(batch_ordinal)
            .ok()
            .map(|index| index.handle(self.generation))
    }
}

impl CanonicalSceneBuilder {
    pub(crate) fn node_batch_plan(
        &self,
        node_count: usize,
    ) -> Result<CanonicalNodeBatchPlan, CanonicalSceneBuildError> {
        self.ensure_pristine_node_batch_target()?;
        if node_count == 0 {
            return Err(CanonicalSceneBuildError::EmptyNodeBatch);
        }
        NodeIndex::try_from(node_count - 1)?;
        let collection = CapacityPlan::for_type::<Node>(0, 0, node_count).map_err(|cause| {
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            }
        })?;
        let node_count_u64 = u64::try_from(node_count).map_err(|_| {
            CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            }
        })?;
        let value_storage_bytes_per_node = Node::value_storage_layout_bytes()?;
        let value_storage_envelope_bytes = value_storage_bytes_per_node
            .checked_mul(node_count_u64)
            .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow {
                collection: CanonicalSceneCollection::Nodes,
            })?;
        Ok(CanonicalNodeBatchPlan {
            generation: self.data.generation,
            collection,
            value_storage_bytes_per_node,
            value_storage_envelope_bytes,
        })
    }

    pub(crate) fn prepare_node_batch(
        &mut self,
        plan: CanonicalNodeBatchPlan,
        build: impl for<'batch> FnOnce(
            &mut CanonicalNodeBatch<'batch>,
        ) -> Result<(), CanonicalSceneBuildError>,
    ) -> Result<PreparedCanonicalNodeBatch<'_>, CanonicalSceneBuildError> {
        self.prepare_node_batch_coupled(plan, (), |batch| build(batch))
    }

    pub(crate) fn prepare_node_batch_coupled<C>(
        &mut self,
        plan: CanonicalNodeBatchPlan,
        companion: C,
        build: impl for<'batch> FnOnce(
            &mut CanonicalNodeBatch<'batch, C>,
        ) -> Result<(), CanonicalSceneBuildError>,
    ) -> Result<PreparedCanonicalNodeBatch<'_, C>, CanonicalSceneBuildError> {
        self.ensure_pristine_node_batch_target()?;
        if plan.generation != self.data.generation {
            return Err(CanonicalSceneBuildError::ForeignNodeBatchPlan);
        }
        let node_count = plan.node_count();
        plan.collection
            .verify_preflight(self.data.nodes.len(), self.data.nodes.capacity())
            .map_err(|cause| CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            })?;

        let collection_admitted = plan.collection_envelope_bytes()?;
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(plan.collection.allocator_additional())
            .map_err(|_| CanonicalSceneBuildError::AllocationFailed {
                collection: CanonicalSceneCollection::Nodes,
            })?;
        let collection_retained_allocation_bytes = plan
            .collection
            .finish(nodes.len(), nodes.capacity())
            .map_err(|cause| CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause,
            })?;
        if collection_retained_allocation_bytes > collection_admitted {
            return Err(CanonicalSceneBuildError::PreparedAllocationUnderestimated {
                collection: CanonicalSceneCollection::Nodes,
                allocation: CanonicalPreparedAllocation::Collection,
                admitted: collection_admitted,
                actual: collection_retained_allocation_bytes,
            });
        }

        let mut staging = NodeBatchStaging {
            nodes,
            #[cfg(test)]
            storage_drop_probe: None,
            companion,
            root: None,
            collection_retained_allocation_bytes,
            value_storage_retained_allocation_bytes: 0,
            nested_payload_retained_allocation_bytes: 0,
        };
        let brand = CanonicalNodeBatchBrand {
            _non_zst_identity: 0,
        };
        {
            let mut batch = CanonicalNodeBatch {
                brand: &brand,
                staging: &mut staging,
                node_count,
                value_storage_bytes_per_node: plan.value_storage_bytes_per_node,
                value_storage_envelope_bytes: plan.value_storage_envelope_bytes,
                invariant: PhantomData,
            };
            build(&mut batch)?;
        }

        if staging.nodes.len() != node_count {
            return Err(CanonicalSceneBuildError::IncompleteNodeBatch {
                expected: node_count,
                actual: staging.nodes.len(),
            });
        }
        let root = staging
            .root
            .take()
            .ok_or(CanonicalSceneBuildError::MissingNodeBatchRoot)?;
        Ok(PreparedCanonicalNodeBatch {
            staging,
            owner: self,
            root,
        })
    }

    fn ensure_pristine_node_batch_target(&self) -> Result<(), CanonicalSceneBuildError> {
        if !self.data.nodes.is_empty()
            || self.data.nodes.capacity() != 0
            || self.data.root.is_some()
        {
            return Err(CanonicalSceneBuildError::NodeBatchNotPristine {
                length: self.data.nodes.len(),
                capacity: self.data.nodes.capacity(),
                root_set: self.data.root.is_some(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
