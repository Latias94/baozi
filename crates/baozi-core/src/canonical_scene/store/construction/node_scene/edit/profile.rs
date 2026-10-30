//! Concrete profile edits against the final owner layout.

use baozi_operation::__retained_store_integration::activation::CanonicalLane;
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{
    CapacityPlan, OperationOwner, ProgressPhase, ProgressUnit, StorePlanError, StoreRuntimeError,
};

use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::production::{CanonicalProfileOwner, ProfileOwnerLayout};
use crate::canonical_scene::store::profile_provenance::{
    ProfileProvenanceRef, ProfileSegmentAddressDomain, issue_profile_segment_nonce,
};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};
use crate::canonical_scene::store::typed_storage::profile::build::ProfileStatePlan;
use crate::canonical_scene::store::typed_storage::profile::{
    LineageCounts, LineageSegment, ProfileState, ProfileStorage, StoredOperation,
    StoredOperationKind, StoredProfileOwner, StoredProfileSegment, StoredProvenance,
    StoredProvenanceKind,
};
use crate::canonical_scene::store::typed_storage::{GeometryStorage, TableRange};
use crate::canonical_scene::store::validation::ProfileMeteredCounts;
use crate::provenance::{DIGEST_LEN, OperationIdentity, OperationKind};

use super::super::geometry::map_build;
use super::super::{SceneEvidenceCounts, SceneStorage};
use super::plan::{CheckedCount, NodeSceneEditPlanError};

/// Each changed owner retains its own parent set.
#[derive(Debug, Clone, Copy)]
pub(super) enum ProfileEditTargets {
    Node(u32),
    Triangulation { primitive: usize },
}

impl ProfileEditTargets {
    fn selects(self, owner: CanonicalProfileOwner, geometry: &GeometryStorage) -> bool {
        match (self, owner) {
            (Self::Node(target), CanonicalProfileOwner::Node(node)) => target == node,
            (Self::Triangulation { primitive }, CanonicalProfileOwner::Primitive(index)) => {
                primitive == index as usize
            }
            (Self::Triangulation { primitive }, CanonicalProfileOwner::Attribute(index)) => {
                geometry
                    .primitives
                    .get(primitive)
                    .and_then(|primitive| primitive.attributes.checked(geometry.attributes.len()))
                    .is_some_and(|range| {
                        range.contains(&(index as usize))
                            && geometry
                                .attributes
                                .get(index as usize)
                                .is_some_and(|attribute| {
                                    matches!(
                                        attribute.domain,
                                        AttributeDomain::Corner | AttributeDomain::Face
                                    )
                                })
                    })
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NodeSceneEditProfilePlan {
    source_layout: ProfileOwnerLayout,
    output_layout: ProfileOwnerLayout,
    targets: ProfileEditTargets,
    lineage: [CapacityPlan; 3],
    state: ProfileStatePlan,
    pub(super) source_segment_count: CheckedCount,
    pub(super) target_count: CheckedCount,
    pub(super) target_attachment_count: CheckedCount,
    pub(super) output_attachment_count: CheckedCount,
    pub(super) output_counts: ProfileMeteredCounts,
    pub(super) maximum_depth: u32,
}

impl NodeSceneEditProfilePlan {
    pub(super) fn try_new(
        input: &SceneStorage,
        targets: ProfileEditTargets,
        output: SceneEvidenceCounts,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let invalid = || NodeSceneEditPlanError::InvalidSource {
            reason: "profile owners disagree with the final Scene layout",
        };
        let source_layout = input
            .proof
            .counts()
            .profile_owner_layout()
            .ok_or_else(invalid)?;
        let output_layout = output.profile_owner_layout().ok_or_else(invalid)?;
        let source = input.tables.profile.state.as_ref();
        let mut targets_count = 0_u32;
        let mut parents = 0_u32;
        let mut attachments = 0_u32;
        let mut maximum_depth = input.proof.profile_maximum_provenance_depth();
        for ordinal in 0..output_layout.owner_count() {
            poll()?;
            let owner = output_layout.owner_at(ordinal).ok_or_else(invalid)?;
            let source_ordinal = source_layout.ordinal(owner).ok_or_else(invalid)?;
            let source_owner = source
                .owners
                .get(source_ordinal as usize)
                .ok_or_else(invalid)?;
            if source_owner.owner != owner {
                return Err(invalid());
            }
            let source_count = u32::try_from(source_owner.attachments.len())
                .map_err(|_| StorePlanError::SizeOverflow)?;
            let count = if targets.selects(owner, input.read_geometry()) {
                targets_count = targets_count
                    .checked_add(1)
                    .ok_or(StorePlanError::SizeOverflow)?;
                parents = parents
                    .checked_add(source_count)
                    .ok_or(StorePlanError::SizeOverflow)?;
                let depth = source_owner
                    .maximum_depth
                    .checked_add(1)
                    .ok_or(StorePlanError::SizeOverflow)?;
                maximum_depth = maximum_depth.max(depth);
                1
            } else {
                source_count
            };
            attachments = attachments
                .checked_add(count)
                .ok_or(StorePlanError::SizeOverflow)?;
        }
        if targets_count == 0 {
            return Err(invalid());
        }
        let mut counts = input.proof.profile_counts();
        counts.segment_count = counts
            .segment_count
            .checked_add(1)
            .ok_or(StorePlanError::SizeOverflow)?;
        counts.provenance_count = counts
            .provenance_count
            .checked_add(targets_count)
            .ok_or(StorePlanError::SizeOverflow)?;
        counts.operation_count = counts
            .operation_count
            .checked_add(targets_count)
            .ok_or(StorePlanError::SizeOverflow)?;
        counts.parent_count = counts
            .parent_count
            .checked_add(parents)
            .ok_or(StorePlanError::SizeOverflow)?;
        counts.lineage_digest_bytes = counts
            .lineage_digest_bytes
            .checked_add(
                targets_count
                    .checked_mul(DIGEST_LEN as u32)
                    .ok_or(StorePlanError::SizeOverflow)?,
            )
            .ok_or(StorePlanError::SizeOverflow)?;
        counts.owner_count =
            u32::try_from(output_layout.owner_count()).map_err(|_| StorePlanError::SizeOverflow)?;
        counts.attachment_count = attachments;
        Ok(Self {
            source_layout,
            output_layout,
            targets,
            lineage: [
                capacity::<StoredProvenance>(u64::from(targets_count))?,
                capacity::<StoredOperation>(u64::from(targets_count))?,
                capacity::<ProfileProvenanceRef>(u64::from(parents))?,
            ],
            state: ProfileStatePlan::for_output(
                u64::from(counts.segment_count),
                u64::from(counts.owner_count),
                u64::from(attachments),
                u64::from(counts.metadata_entry_count),
            )?,
            source_segment_count: CheckedCount::try_from_usize(
                source.segments.len(),
                "source segments",
            )?,
            target_count: CheckedCount::try_from_u32(targets_count, "changed profile owners")?,
            target_attachment_count: CheckedCount::try_from_u32(
                parents,
                "changed owners' parents",
            )?,
            output_attachment_count: CheckedCount::try_from_u32(attachments, "output attachments")?,
            output_counts: counts,
            maximum_depth,
        })
    }

    fn source_owner(
        self,
        source: &ProfileState,
        output_owner: CanonicalProfileOwner,
    ) -> Option<&StoredProfileOwner> {
        let ordinal = usize::try_from(self.source_layout.ordinal(output_owner)?).ok()?;
        source
            .owners
            .get(ordinal)
            .filter(|row| row.owner == output_owner)
    }

    fn lineage_work(self) -> Result<u64, StorePlanError> {
        count_work(self.lineage)?
            .checked_add(self.output_layout.owner_count())
            .ok_or(StorePlanError::SizeOverflow)
    }

    fn build_lineage<C: TypedStorageBuildControl>(
        self,
        input: &SceneStorage,
        operation: &OperationIdentity,
        nonce: crate::canonical_scene::store::profile_provenance::ProfileSegmentNonce,
        control: &mut C,
    ) -> BuildResult<LineageSegment, C::Error> {
        let mut provenance = allocate(self.lineage[0])?;
        let mut operations = allocate(self.lineage[1])?;
        let mut parents = allocate(self.lineage[2])?;
        let source = input.tables.profile.state.as_ref();
        let kind = match operation.kind() {
            OperationKind::Transform => StoredOperationKind::Transform,
            OperationKind::Split => StoredOperationKind::Split,
            _ => return Err(TypedStorageBuildError::Invariant),
        };
        for ordinal in 0..self.output_layout.owner_count() {
            let owner = control
                .run_unit(|| self.output_layout.owner_at(ordinal))
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
            if !self.targets.selects(owner, input.read_geometry()) {
                continue;
            }
            let row = self
                .source_owner(source, owner)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let values = row
                .attachments
                .get(&source.attachments)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = parents.len();
            for &parent in values {
                #[allow(unused_mut)]
                let mut parent = parent;
                #[cfg(test)]
                crate::canonical_scene::store::profile_provenance::apply_profile_reference_corruption_for_test(&mut parent);
                push(&mut parents, self.lineage[2], parent, control)?;
            }
            let local =
                u32::try_from(operations.len()).map_err(|_| TypedStorageBuildError::Invariant)?;
            push(
                &mut operations,
                self.lineage[1],
                StoredOperation {
                    kind,
                    options: *operation.options_digest(),
                    evidence: TableRange::new(0, 0),
                },
                control,
            )?;
            push(
                &mut provenance,
                self.lineage[0],
                StoredProvenance {
                    kind: StoredProvenanceKind::Operation(local),
                    depth: row
                        .maximum_depth
                        .checked_add(1)
                        .ok_or(TypedStorageBuildError::Invariant)?,
                    parents: TableRange::new(start, parents.len() - start),
                },
                control,
            )?;
        }
        finish(&provenance, self.lineage[0])?;
        finish(&operations, self.lineage[1])?;
        finish(&parents, self.lineage[2])?;
        Ok(LineageSegment {
            nonce,
            resources: Vec::new(),
            sources: Vec::new(),
            provenance,
            operations,
            evidence: Vec::new(),
            parents,
            text: String::new(),
            wide_value_count: 0,
            digest_byte_count: self
                .target_count
                .as_usize()
                .checked_mul(DIGEST_LEN)
                .ok_or(TypedStorageBuildError::Invariant)?,
        })
    }

    fn build_state<C: TypedStorageBuildControl>(
        self,
        input: &SceneStorage,
        mut segments: Vec<StoredProfileSegment>,
        control: &mut C,
    ) -> BuildResult<ProfileState, C::Error> {
        let lineage_counts = self.state.initialize_segments(&mut segments, control)?;
        let p = self.state.capacities();
        let mut owners = allocate(p[1])?;
        let mut attachments = allocate(p[2])?;
        let mut metadata_entries = allocate(p[3])?;
        let source = input.tables.profile.state.as_ref();
        let segment = segments.last().ok_or(TypedStorageBuildError::Invariant)?;
        let domain = ProfileSegmentAddressDomain::new(
            self.source_segment_count.as_u32(),
            segment.lineage.as_ref().nonce,
            u32::try_from(segment.prefixes.provenance)
                .map_err(|_| TypedStorageBuildError::Invariant)?,
            self.target_count.as_u32(),
        );
        let mut local = 0;
        for ordinal in 0..self.output_layout.owner_count() {
            let owner = self
                .output_layout
                .owner_at(ordinal)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let row = self
                .source_owner(source, owner)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = attachments.len();
            let maximum_depth = if self.targets.selects(owner, input.read_geometry()) {
                let reference = domain
                    .reference(local)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                local = local
                    .checked_add(1)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                push(&mut attachments, p[2], reference, control)?;
                row.maximum_depth
                    .checked_add(1)
                    .ok_or(TypedStorageBuildError::Invariant)?
            } else {
                for &reference in row
                    .attachments
                    .get(&source.attachments)
                    .ok_or(TypedStorageBuildError::Invariant)?
                {
                    push(&mut attachments, p[2], reference, control)?;
                }
                row.maximum_depth
            };
            push(
                &mut owners,
                p[1],
                StoredProfileOwner {
                    owner,
                    attachments: TableRange::new(start, attachments.len() - start),
                    maximum_depth,
                },
                control,
            )?;
        }
        if local != self.target_count.as_u32() {
            return Err(TypedStorageBuildError::Invariant);
        }
        for &entry in &source.metadata_entries {
            if !self.output_layout.contains(entry.owner) {
                return Err(TypedStorageBuildError::Invariant);
            }
            push(&mut metadata_entries, p[3], entry, control)?;
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
}

pub(super) fn retain_edited_profile<O, PP, U>(
    lane: &mut CanonicalLane<'_, O, PP, U, CanonicalInheritanceSource>,
    input: &SceneStorage,
    plan: NodeSceneEditProfilePlan,
    operation: &OperationIdentity,
) -> Result<ProfileStorage, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    let lineage = lane.try_retain_storage(
        plan.lineage,
        plan.lineage_work().map_err(StoreRuntimeError::Plan)?,
        0,
        |control| {
            plan.build_lineage(input, operation, nonce, control)
                .map_err(map_build)
        },
    )?;
    let metadata = lane.try_share_storage(&input.tables.profile.metadata)?;
    let state = lane.try_retain_storage_prepared(
        plan.state.capacities(),
        plan.state.work().map_err(StoreRuntimeError::Plan)?,
        0,
        |lane| {
            let mut segments = plan.state.allocate_segments().map_err(map_build)?;
            for segment in &input.tables.profile.state.as_ref().segments {
                segments.push(StoredProfileSegment {
                    lineage: lane.try_share_storage(&segment.lineage)?,
                    prefixes: segment.prefixes,
                });
            }
            segments.push(StoredProfileSegment {
                lineage,
                prefixes: LineageCounts::default(),
            });
            Ok(segments)
        },
        |segments, control| {
            plan.build_state(input, segments, control)
                .map_err(map_build)
        },
    )?;
    Ok(ProfileStorage { state, metadata })
}
