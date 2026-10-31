//! Current-generation owner layout, shared lineage descriptors, and metadata descriptors.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::ValidatedProfileBatch;
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};

use super::super::*;
use super::MetadataDescriptorCursor;

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProfileStatePlan {
    capacities: [CapacityPlan; 4],
}

impl ProfileStatePlan {
    pub(in crate::canonical_scene::store) fn for_batch<R>(
        batch: &ValidatedProfileBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        Self::for_output(
            1,
            batch.owner_count(),
            batch.owner_attachment_count(),
            u64::from(batch.metadata_entry_count()),
        )
    }

    pub(in crate::canonical_scene::store) fn for_output(
        segments: u64,
        owners: u64,
        attachments: u64,
        metadata_entries: u64,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            capacities: [
                capacity::<StoredProfileSegment>(segments)?,
                capacity::<StoredProfileOwner>(owners)?,
                capacity::<ProfileProvenanceRef>(attachments)?,
                capacity::<StoredMetadataEntry>(metadata_entries)?,
            ],
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 4] {
        self.capacities
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities)
    }

    /// The caller holds this exact central admission while preparing the shared segment handles.
    pub(in crate::canonical_scene::store) fn allocate_segments<E>(
        self,
    ) -> BuildResult<Vec<StoredProfileSegment>, E> {
        allocate(self.capacities[0])
    }

    pub(in crate::canonical_scene::store) fn build_fresh<R, C: TypedStorageBuildControl>(
        self,
        mut segments: Vec<StoredProfileSegment>,
        batch: &ValidatedProfileBatch<'_, R>,
        metadata: &MetadataPayloads,
        control: &mut C,
    ) -> BuildResult<ProfileState, C::Error> {
        if segments.len() != 1 {
            return Err(TypedStorageBuildError::Invariant);
        }
        let lineage_counts = self.initialize_segments(&mut segments, control)?;
        let domain = segments[0]
            .domain(0)
            .ok_or(TypedStorageBuildError::Invariant)?;
        let p = self.capacities;
        let mut owners = allocate(p[1])?;
        let mut attachments = allocate(p[2])?;
        let mut metadata_entries = allocate(p[3])?;
        for ordinal in 0..batch.owner_count() {
            let (owner, values) = batch
                .owner_provenance_at(ordinal)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = attachments.len();
            let mut maximum_depth = 0;
            for value in values.iter().copied() {
                maximum_depth = maximum_depth.max(
                    batch
                        .provenance_depth(value)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                );
                let reference = domain
                    .reference(value.index())
                    .ok_or(TypedStorageBuildError::Invariant)?;
                push(&mut attachments, p[2], reference, control)?;
            }
            let range = TableRange::new(start, attachments.len() - start);
            push(
                &mut owners,
                p[1],
                StoredProfileOwner {
                    owner,
                    attachments: range,
                    maximum_depth,
                },
                control,
            )?;
        }
        let mut metadata_cursor = MetadataDescriptorCursor::default();
        for index in 0..p[3].desired_capacity() {
            let source = batch
                .metadata_entry(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let entry = metadata_cursor
                .entry(source.owner(), source.key().as_str(), source.value())
                .ok_or(TypedStorageBuildError::Invariant)?;
            push(&mut metadata_entries, p[3], entry, control)?;
        }
        if !metadata_cursor.matches(metadata) {
            return Err(TypedStorageBuildError::Invariant);
        }
        finish(&owners, p[1])?;
        finish(&attachments, p[2])?;
        finish(&metadata_entries, p[3])?;
        Ok(ProfileState {
            segments,
            owners,
            attachments,
            metadata_entries,
            lineage_counts,
        })
    }

    pub(in crate::canonical_scene::store) fn initialize_segments<C: TypedStorageBuildControl>(
        self,
        segments: &mut Vec<StoredProfileSegment>,
        control: &mut C,
    ) -> BuildResult<LineageCounts, C::Error> {
        finish(segments, self.capacities[0])?;
        let mut totals = LineageCounts::default();
        for segment in segments {
            let prefixes = totals;
            totals = control
                .run_unit(|| {
                    segment.prefixes = prefixes;
                    prefixes.checked_add(segment.lineage.as_ref().counts())
                })
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
        }
        Ok(totals)
    }
}
