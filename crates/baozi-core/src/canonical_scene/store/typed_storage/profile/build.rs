//! Capacity-qualified construction of actual typed profile tables.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::{
    ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentityStability, ValidatedProfileBatch,
};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};
use crate::provenance::{OperationKind, SourceObjectLocator};

use super::*;

mod metadata;
pub(in crate::canonical_scene::store) use metadata::{
    MetadataDescriptorCursor, MetadataStoragePlan,
};
mod state;
pub(in crate::canonical_scene::store) use state::ProfileStatePlan;

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct LineageStoragePlan {
    capacities: [CapacityPlan; 7],
    wide_values: usize,
    digest_bytes: usize,
}

impl LineageStoragePlan {
    pub(in crate::canonical_scene::store) fn for_batch<R>(
        batch: &ValidatedProfileBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let shape = batch.retention_shape();
        Ok(Self {
            capacities: [
                capacity::<StoredResource>(u64::from(batch.resource_count()))?,
                capacity::<StoredSource>(u64::from(batch.source_count()))?,
                capacity::<StoredProvenance>(u64::from(batch.provenance_node_count()))?,
                capacity::<StoredOperation>(u64::from(batch.operation_count()))?,
                capacity::<StoredOperationEvidence>(u64::from(shape.evidence_count()))?,
                capacity::<ProfileProvenanceRef>(u64::from(shape.parent_count()))?,
                capacity::<u8>(u64::from(shape.lineage_text_bytes()))?,
            ],
            wide_values: shape.wide_count() as usize,
            digest_bytes: shape.lineage_digest_bytes() as usize,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 7] {
        self.capacities
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities)
    }

    pub(in crate::canonical_scene::store) const fn text_bytes(self) -> usize {
        self.capacities[6].desired_capacity()
    }

    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedProfileBatch<'_, R>,
        nonce: ProfileSegmentNonce,
        control: &mut C,
    ) -> BuildResult<LineageSegment, C::Error> {
        let p = self.capacities;
        let mut storage = LineageSegment {
            nonce,
            resources: allocate(p[0])?,
            sources: allocate(p[1])?,
            provenance: allocate(p[2])?,
            operations: allocate(p[3])?,
            evidence: allocate(p[4])?,
            parents: allocate(p[5])?,
            text: allocate_text(p[6])?,
            wide_value_count: self.wide_values,
            digest_byte_count: self.digest_bytes,
        };
        let domain = ProfileSegmentAddressDomain::new(0, nonce, 0, batch.provenance_node_count());
        for resource in batch.lineage().resources().iter().copied() {
            let identity = text(
                &mut storage.text,
                p[6],
                resource.identity().as_str(),
                control,
            )?;
            let stability = match resource.stability() {
                ResourceIdentityStability::Stable {
                    owner,
                    schema,
                    version,
                } => StoredResourceStability::Stable {
                    owner: text(&mut storage.text, p[6], owner.as_str(), control)?,
                    schema: text(&mut storage.text, p[6], schema.as_str(), control)?,
                    version: *version,
                },
                ResourceIdentityStability::SessionOpaque(reason) => {
                    StoredResourceStability::SessionOpaque(*reason)
                }
            };
            push(
                &mut storage.resources,
                p[0],
                StoredResource {
                    identity,
                    stability,
                },
                control,
            )?;
        }
        for source in batch.lineage().source_identities().iter().copied() {
            let format = text(&mut storage.text, p[6], source.format().as_str(), control)?;
            let locator = match source.locator() {
                SourceObjectLocator::Indexed { kind, index } => StoredSourceLocator::Indexed {
                    kind: *kind,
                    index: *index,
                },
                SourceObjectLocator::Namespaced(locator) => StoredSourceLocator::Namespaced {
                    schema: text(
                        &mut storage.text,
                        p[6],
                        locator.schema_id().name().as_str(),
                        control,
                    )?,
                    version: locator.schema_id().version(),
                    value: text(&mut storage.text, p[6], locator.value().as_str(), control)?,
                },
            };
            let display_name = source
                .display_name()
                .map(|value| text(&mut storage.text, p[6], value.as_str(), control))
                .transpose()?;
            push(
                &mut storage.sources,
                p[1],
                StoredSource {
                    resource: source.resource().index(),
                    format,
                    locator,
                    display_name,
                    content_digest: source.content_digest(),
                },
                control,
            )?;
        }
        for (index, node) in batch
            .lineage()
            .provenance_nodes()
            .iter()
            .copied()
            .enumerate()
        {
            let index = u32::try_from(index).map_err(|_| TypedStorageBuildError::Invariant)?;
            let parents_start = storage.parents.len();
            let kind = match node {
                ProvenanceNodeAuthoring::Source(source) => {
                    StoredProvenanceKind::Source(source.index())
                }
                ProvenanceNodeAuthoring::Operation(operation) => {
                    for parent in operation.parents().iter().copied() {
                        let parent = domain
                            .reference(parent.index())
                            .ok_or(TypedStorageBuildError::Invariant)?;
                        push(&mut storage.parents, p[5], parent, control)?;
                    }
                    let kind = operation_kind(&mut storage.text, p[6], operation.kind(), control)?;
                    let evidence_start = storage.evidence.len();
                    for evidence in operation.evidence() {
                        let code =
                            text(&mut storage.text, p[6], evidence.code().as_str(), control)?;
                        push(
                            &mut storage.evidence,
                            p[4],
                            StoredOperationEvidence {
                                code,
                                occurrences: evidence.occurrence_count(),
                                digest: evidence.digest().copied(),
                            },
                            control,
                        )?;
                    }
                    let local = u32::try_from(storage.operations.len())
                        .map_err(|_| TypedStorageBuildError::Invariant)?;
                    let evidence =
                        TableRange::new(evidence_start, storage.evidence.len() - evidence_start);
                    push(
                        &mut storage.operations,
                        p[3],
                        StoredOperation {
                            kind,
                            options: operation.options_digest(),
                            evidence,
                        },
                        control,
                    )?;
                    StoredProvenanceKind::Operation(local)
                }
            };
            let depth = batch
                .provenance_depth(ProvenanceLocalId::new(index))
                .ok_or(TypedStorageBuildError::Invariant)?;
            let parents = TableRange::new(parents_start, storage.parents.len() - parents_start);
            push(
                &mut storage.provenance,
                p[2],
                StoredProvenance {
                    kind,
                    depth,
                    parents,
                },
                control,
            )?;
        }
        finish(&storage.resources, p[0])?;
        finish(&storage.sources, p[1])?;
        finish(&storage.provenance, p[2])?;
        finish(&storage.operations, p[3])?;
        finish(&storage.evidence, p[4])?;
        finish(&storage.parents, p[5])?;
        finish_text(&storage.text, p[6])?;
        Ok(storage)
    }
}

pub(super) fn text<C: TypedStorageBuildControl>(
    pool: &mut String,
    plan: CapacityPlan,
    value: &str,
    control: &mut C,
) -> BuildResult<TextRange, C::Error> {
    let start = pool.len();
    for character in value.chars() {
        let width = character.len_utf8();
        let end = pool
            .len()
            .checked_add(width)
            .ok_or(TypedStorageBuildError::Invariant)?;
        if end > plan.desired_capacity() || end > pool.capacity() {
            return Err(TypedStorageBuildError::Invariant);
        }
        // The shared control counts UTF-8 bytes. Admit every byte of one scalar before writing
        // it, keeping String valid even when a later control checkpoint rejects construction.
        for _ in 1..width {
            control
                .run_string_unit(|| ())
                .map_err(TypedStorageBuildError::Control)?;
        }
        control
            .run_string_unit(|| pool.push(character))
            .map_err(TypedStorageBuildError::Control)?;
    }
    Ok(TextRange(TableRange::new(start, value.len())))
}

fn operation_kind<C: TypedStorageBuildControl>(
    pool: &mut String,
    plan: CapacityPlan,
    kind: &OperationKind,
    control: &mut C,
) -> BuildResult<StoredOperationKind, C::Error> {
    Ok(match kind {
        OperationKind::Generated => StoredOperationKind::Generated,
        OperationKind::Default => StoredOperationKind::Default,
        OperationKind::Normalize => StoredOperationKind::Normalize,
        OperationKind::Repair => StoredOperationKind::Repair,
        OperationKind::Split => StoredOperationKind::Split,
        OperationKind::Merge => StoredOperationKind::Merge,
        OperationKind::Convert => StoredOperationKind::Convert,
        OperationKind::Transform => StoredOperationKind::Transform,
        OperationKind::Optimize => StoredOperationKind::Optimize,
        OperationKind::Namespaced(name) => {
            StoredOperationKind::Namespaced(text(pool, plan, name.as_str(), control)?)
        }
    })
}

pub(super) fn allocate_text<E>(plan: CapacityPlan) -> BuildResult<String, E> {
    let mut value = String::new();
    value
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| TypedStorageBuildError::AllocationFailed)?;
    plan.finish(0, value.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(value)
}

pub(super) fn finish_text<E>(value: &String, plan: CapacityPlan) -> BuildResult<(), E> {
    if value.len() != plan.desired_capacity() {
        return Err(TypedStorageBuildError::Invariant);
    }
    plan.finish(0, value.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(())
}
