//! Retention of the actual typed profile owners through the single central storage admission.

use baozi_operation::__retained_store_integration::activation::CanonicalLane;
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, StorePlanError, StoreRuntimeError,
};

use crate::canonical_scene::store::production::ValidatedProfileBatch;
use crate::canonical_scene::store::profile_provenance::issue_profile_segment_nonce;
use crate::canonical_scene::store::typed_storage::profile::build::{
    LineageStoragePlan, MetadataStoragePlan, ProfileStatePlan,
};
use crate::canonical_scene::store::typed_storage::profile::{
    LineageCounts, ProfileStorage, StoredProfileSegment,
};
use crate::canonical_scene::store::validation::ProfileMeteredCounts;

use super::geometry::map_build;

#[derive(Clone, Copy)]
pub(super) struct ProfileMaterializationPlans {
    lineage: LineageStoragePlan,
    metadata: MetadataStoragePlan,
    state: ProfileStatePlan,
    counts: ProfileMeteredCounts,
    maximum_depth: u32,
}

impl ProfileMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedProfileBatch<'_, R>) -> Result<Self, StorePlanError> {
        let shape = batch.retention_shape();
        Ok(Self {
            lineage: LineageStoragePlan::for_batch(batch)?,
            metadata: MetadataStoragePlan::for_batch(batch)?,
            state: ProfileStatePlan::for_batch(batch)?,
            counts: ProfileMeteredCounts {
                segment_count: 1,
                resource_count: batch.resource_count(),
                source_count: batch.source_count(),
                provenance_count: batch.provenance_node_count(),
                operation_count: batch.operation_count(),
                evidence_count: shape.evidence_count(),
                parent_count: shape.parent_count(),
                wide_count: shape.wide_count(),
                lineage_text_bytes: shape.lineage_text_bytes(),
                lineage_digest_bytes: shape.lineage_digest_bytes(),
                owner_count: u32::try_from(batch.owner_count())
                    .map_err(|_| StorePlanError::SizeOverflow)?,
                attachment_count: shape.attachment_count(),
                metadata_entry_count: batch.metadata_entry_count(),
                metadata_text_bytes: shape.metadata_text_bytes(),
                metadata_payload_bytes: shape.metadata_payload_bytes(),
                metadata_bool_count: shape.metadata_bool_count(),
                metadata_i64_count: shape.metadata_i64_count(),
                metadata_u64_count: shape.metadata_u64_count(),
                metadata_f64_count: shape.metadata_f64_count(),
                metadata_f32_count: shape.metadata_f32_count(),
            },
            maximum_depth: batch.maximum_depth(),
        })
    }

    pub(super) const fn metered_counts(self) -> ProfileMeteredCounts {
        self.counts
    }

    pub(super) const fn maximum_provenance_depth(self) -> u32 {
        self.maximum_depth
    }

    pub(super) fn validation_work_units(self) -> Result<u64, StorePlanError> {
        self.counts
            .validation_work()
            .ok_or(StorePlanError::SizeOverflow)
    }
}

pub(super) fn retain_profile<O, PP, U, L, R>(
    lane: &mut CanonicalLane<'_, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    #[cfg(test)] fault: &mut ProfileBuildFault,
) -> Result<ProfileStorage, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    #[cfg(test)]
    let selected_fault = std::mem::replace(fault, ProfileBuildFault::None);
    let lineage = lane.try_retain_storage(
        plans.lineage.capacities(),
        plans.lineage.work().map_err(StoreRuntimeError::Plan)?,
        plans.lineage.text_bytes() as u64,
        |control| {
            #[allow(unused_mut)]
            let mut lineage = plans
                .lineage
                .build(batch, nonce, control)
                .map_err(map_build)?;
            #[cfg(test)]
            selected_fault.alter_lineage(&mut lineage);
            Ok(lineage)
        },
    )?;
    let metadata = lane.try_retain_storage(
        plans.metadata.capacities(),
        plans.metadata.work().map_err(StoreRuntimeError::Plan)?,
        plans.metadata.text_bytes() as u64,
        |control| plans.metadata.build(batch, control).map_err(map_build),
    )?;
    let state = lane.try_retain_storage_prepared(
        plans.state.capacities(),
        plans.state.work().map_err(StoreRuntimeError::Plan)?,
        0,
        |_| {
            let mut segments = plans.state.allocate_segments().map_err(map_build)?;
            segments.push(StoredProfileSegment {
                lineage,
                prefixes: LineageCounts::default(),
            });
            Ok(segments)
        },
        |segments, control| {
            #[allow(unused_mut)]
            let mut state = plans
                .state
                .build_fresh(segments, batch, metadata.as_ref(), control)
                .map_err(map_build)?;
            #[cfg(test)]
            selected_fault.alter_state(&mut state)?;
            Ok(state)
        },
    )?;
    Ok(ProfileStorage { state, metadata })
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileBuildFault {
    None,
    SegmentPrefixes,
    ForeignSegmentNonce,
    SourceResource,
    ProvenanceDepth,
    OwnerIdentity,
    OwnerRange,
    OwnerDepthOverstated,
    OwnerDepthUnderstated,
    MetadataOwner,
    MetadataRange,
    MetadataValueShape,
}

#[cfg(test)]
impl ProfileBuildFault {
    pub(super) const fn none() -> Self {
        Self::None
    }

    pub(super) const ALL: [Self; 11] = [
        Self::SegmentPrefixes,
        Self::ForeignSegmentNonce,
        Self::SourceResource,
        Self::ProvenanceDepth,
        Self::OwnerIdentity,
        Self::OwnerRange,
        Self::OwnerDepthOverstated,
        Self::OwnerDepthUnderstated,
        Self::MetadataOwner,
        Self::MetadataRange,
        Self::MetadataValueShape,
    ];

    fn alter_lineage(
        self,
        lineage: &mut crate::canonical_scene::store::typed_storage::profile::LineageSegment,
    ) {
        match self {
            Self::SourceResource => {
                if let Some(source) = lineage.sources.first_mut() {
                    source.resource = u32::MAX;
                }
            }
            Self::ProvenanceDepth => {
                if let Some(node) = lineage.provenance.first_mut() {
                    node.depth = node.depth.saturating_add(1);
                }
            }
            _ => {}
        }
    }

    fn alter_state(
        self,
        state: &mut crate::canonical_scene::store::typed_storage::profile::ProfileState,
    ) -> Result<(), StoreRuntimeError> {
        use crate::canonical_scene::store::production::CanonicalProfileOwner;
        use crate::canonical_scene::store::profile_provenance::{
            ProfileSegmentAddressDomain, ProfileSegmentNonce,
        };
        use crate::canonical_scene::store::typed_storage::profile::StoredMetadataValue;
        use crate::canonical_scene::store::typed_storage::{TableRange, TextRange};
        match self {
            Self::SegmentPrefixes => {
                if let Some(segment) = state.segments.first_mut() {
                    segment.prefixes.resources = 1;
                }
            }
            Self::ForeignSegmentNonce => {
                if let (Some(segment), Some(reference)) =
                    (state.segments.first(), state.attachments.first_mut())
                {
                    let nonce = ProfileSegmentNonce::from_stored(
                        segment.lineage.as_ref().nonce.get() ^ u64::MAX,
                    )
                    .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)?;
                    let count = u32::try_from(segment.lineage.as_ref().provenance.len())
                        .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
                    *reference = ProfileSegmentAddressDomain::new(0, nonce, 0, count)
                        .reference(reference.local())
                        .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)?;
                }
            }
            Self::OwnerIdentity => {
                if let Some(owner) = state.owners.first_mut() {
                    owner.owner = CanonicalProfileOwner::SyntheticRoot;
                }
            }
            Self::OwnerRange => {
                if let Some(owner) = state.owners.first_mut() {
                    owner.attachments = TableRange::new(0, 0);
                }
            }
            Self::OwnerDepthOverstated => {
                if let Some(owner) = state.owners.first_mut() {
                    owner.maximum_depth = owner.maximum_depth.saturating_add(1);
                }
            }
            Self::OwnerDepthUnderstated => {
                if let Some(owner) = state.owners.first_mut() {
                    owner.maximum_depth = owner.maximum_depth.saturating_sub(1);
                }
            }
            Self::MetadataOwner => {
                if let Some(entry) = state.metadata_entries.first_mut() {
                    entry.owner = CanonicalProfileOwner::Node(0);
                }
            }
            Self::MetadataRange => {
                if let Some(entry) = state.metadata_entries.first_mut() {
                    entry.key = TextRange(TableRange::new(usize::MAX, 1));
                }
            }
            Self::MetadataValueShape => {
                if let Some(entry) = state.metadata_entries.first_mut() {
                    entry.value = StoredMetadataValue::Vec3(TableRange::new(0, 2));
                }
            }
            Self::None | Self::SourceResource | Self::ProvenanceDepth => {}
        }
        Ok(())
    }
}
