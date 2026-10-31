//! Concrete profile edits against the final owner layout.

#[cfg(test)]
#[path = "normals_tests.rs"]
mod normals_tests;

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
use crate::canonical_scene::store::typed_storage::TableRange;
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, capacity, count_work,
    finish, push,
};
use crate::canonical_scene::store::typed_storage::edit::normals::NormalAttributeMapping;
use crate::canonical_scene::store::typed_storage::profile::build::ProfileStatePlan;
use crate::canonical_scene::store::typed_storage::profile::edit_metadata::{
    MetadataEditCursor, MetadataEditPlan,
};
use crate::canonical_scene::store::typed_storage::profile::{
    LineageCounts, LineageSegment, ProfileState, ProfileStorage, StoredOperation,
    StoredOperationKind, StoredProfileOwner, StoredProfileSegment, StoredProvenance,
    StoredProvenanceKind,
};
use crate::canonical_scene::store::validation::ProfileMeteredCounts;
use crate::provenance::{DIGEST_LEN, OperationIdentity, OperationKind};

use super::super::geometry::map_build;
use super::super::{SceneEvidenceCounts, SceneStorage};
use super::plan::{CheckedCount, NodeSceneEditPlanError};

/// Each changed owner retains its own parent set.
#[derive(Debug, Clone, Copy)]
pub(super) enum ProfileEditTargets {
    #[cfg(test)]
    Node(u32),
    Triangulation {
        primitive: usize,
    },
    Normals {
        primitive: u32,
        attributes: NormalAttributeMapping,
        smooth: bool,
    },
    Space(crate::space_conversion::SpaceConversion),
}

impl ProfileEditTargets {
    fn selects<C: TypedStorageBuildControl>(
        self,
        owner: CanonicalProfileOwner,
        input: &SceneStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if let Self::Space(conversion) = self
            && conversion.changes_coordinates()
        {
            use crate::canonical_scene::store::typed_storage::edit::{
                space_animation::channel_changes,
                space_deformation::{morph_delta_changes, skin_changes},
            };
            match owner {
                CanonicalProfileOwner::Skin(index) => {
                    return skin_changes(
                        input.read_deformation(),
                        index as usize,
                        conversion,
                        control,
                    );
                }
                CanonicalProfileOwner::MorphDelta(index) => {
                    return morph_delta_changes(
                        input.read_deformation(),
                        index as usize,
                        conversion,
                        control,
                    );
                }
                CanonicalProfileOwner::AnimationChannel(index) => {
                    return channel_changes(
                        input.read_animation(),
                        index as usize,
                        conversion,
                        control,
                    );
                }
                _ => {}
            }
        }
        let geometry = input.read_geometry();
        Ok(match (self, owner) {
            (Self::Space(conversion), CanonicalProfileOwner::Material(index)) => {
                crate::canonical_scene::store::typed_storage::edit::space_appearance::material_changes(
                    input.tables.appearance.rows.as_ref(),
                    index as usize,
                    conversion,
                )
            }
            (Self::Space(conversion), CanonicalProfileOwner::TextureBinding(index)) => {
                crate::canonical_scene::store::typed_storage::edit::space_appearance::binding_changes(
                    input.tables.appearance.rows.as_ref(),
                    index as usize,
                    conversion,
                )
            }
            (Self::Space(conversion), CanonicalProfileOwner::Camera(index)) => {
                crate::canonical_scene::store::typed_storage::edit::space_spatial::camera_changes(
                    input.tables.spatial.as_ref(),
                    index as usize,
                    conversion,
                )
            }
            (Self::Space(conversion), CanonicalProfileOwner::Light(index)) => {
                crate::canonical_scene::store::typed_storage::edit::space_spatial::light_changes(
                    input.tables.spatial.as_ref(),
                    index as usize,
                    conversion,
                )
            }
            (Self::Space(_), CanonicalProfileOwner::Scene) => true,
            (Self::Space(conversion), CanonicalProfileOwner::Node(index)) => {
                conversion.changes_coordinates()
                    && input
                        .read_nodes()
                        .nodes
                        .get(index as usize)
                        .and_then(|row| row.transform.authored())
                        .is_some_and(|authored| {
                            conversion
                                .authored_transform(authored)
                                .is_ok_and(|converted| !authored.has_same_authored_bits(converted))
                        })
            }
            (Self::Space(conversion), CanonicalProfileOwner::Primitive(index)) => geometry
                .primitives
                .get(index as usize)
                .is_some_and(|primitive| {
                    (conversion.changes_coordinates() && primitive.vertex_count != 0)
                        || (conversion.reverse_winding()
                            && primitive.topology == crate::geometry::TopologyKind::TriangleList
                            && primitive.corner_count != 0)
                }),
            (Self::Space(conversion), CanonicalProfileOwner::Attribute(index)) => {
                let Some(attribute) = geometry.attributes.get(index as usize) else {
                    return Ok(false);
                };
                if conversion.changes_coordinates() && matches!(attribute.semantic,
                    crate::canonical_scene::store::typed_storage::StoredAttributeSemantic::Position
                    | crate::canonical_scene::store::typed_storage::StoredAttributeSemantic::Normal
                    | crate::canonical_scene::store::typed_storage::StoredAttributeSemantic::Tangent) { return Ok(true); }
                if !conversion.reverse_winding() || attribute.domain != AttributeDomain::Corner {
                    return Ok(false);
                }
                // Validated u32 cardinalities bound this binary lookup to 32 comparisons.
                let primitive = geometry
                    .primitives
                    .partition_point(|primitive| primitive.attributes.start() <= index as usize);
                primitive
                    .checked_sub(1)
                    .and_then(|index| geometry.primitives.get(index))
                    .is_some_and(|primitive| {
                        primitive.topology == crate::geometry::TopologyKind::TriangleList
                            && primitive.corner_count != 0
                    })
            }
            #[cfg(test)]
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
            (Self::Normals { primitive, .. }, CanonicalProfileOwner::Primitive(index)) => {
                primitive == index
            }
            (Self::Normals { attributes, .. }, CanonicalProfileOwner::Attribute(index)) => {
                index == attributes.normal_attribute()
            }
            _ => false,
        })
    }

    /// Every payload-owner selector exhausts its own actual matrix/curve/delta range.
    /// Source central validation proves those ranges partition the corresponding pools.
    /// These are the additional controlled actions beyond the existing per-owner visits.
    fn selection_work(self, input: &SceneStorage) -> Result<u64, StorePlanError> {
        let Self::Space(conversion) = self else {
            return Ok(0);
        };
        if !conversion.changes_coordinates() {
            return Ok(0);
        }
        let deformation = input.read_deformation();
        let animation = input.read_animation();
        [
            deformation.skins.len(),
            deformation.inverse_bind_matrices.len(),
            deformation.morph_deltas.len(),
            deformation.morph_values.len() / 3,
            animation.channels.len(),
            animation.values.len(),
            animation.in_tangents.len(),
            animation.out_tangents.len(),
        ]
        .into_iter()
        .try_fold(0_u64, |work, count| {
            work.checked_add(count as u64)
                .ok_or(StorePlanError::SizeOverflow)
        })
    }

    fn source_owner(self, output: CanonicalProfileOwner) -> Option<CanonicalProfileOwner> {
        match (self, output) {
            (Self::Normals { attributes, .. }, CanonicalProfileOwner::Attribute(index)) => {
                attributes
                    .source_attribute(index)
                    .map(CanonicalProfileOwner::Attribute)
            }
            _ => Some(output),
        }
    }

    fn output_owner(self, source: CanonicalProfileOwner) -> Option<CanonicalProfileOwner> {
        match (self, source) {
            (Self::Normals { attributes, .. }, CanonicalProfileOwner::Attribute(index)) => {
                attributes
                    .output_attribute(index)
                    .map(CanonicalProfileOwner::Attribute)
            }
            _ => Some(source),
        }
    }

    fn deleted_owner(self) -> Option<CanonicalProfileOwner> {
        match self {
            Self::Normals { attributes, .. } => attributes
                .source_tangent()
                .filter(|&index| attributes.output_attribute(index).is_none())
                .map(CanonicalProfileOwner::Attribute),
            _ => None,
        }
    }
}

struct ProfilePlanningControl<'a, F>(&'a mut F);

impl<F: FnMut() -> Result<(), NodeSceneEditPlanError>> TypedStorageBuildControl
    for ProfilePlanningControl<'_, F>
{
    type Error = NodeSceneEditPlanError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        (self.0)()?;
        Ok(action())
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run_unit(action)
    }
}

fn profile_owner(
    source: &ProfileState,
    layout: ProfileOwnerLayout,
    owner: CanonicalProfileOwner,
) -> Option<&StoredProfileOwner> {
    source
        .owners
        .get(usize::try_from(layout.ordinal(owner)?).ok()?)
        .filter(|row| row.owner == owner)
}

/// A bounded merge of at most four proven source-owner attachment lists.
struct OwnerParents<'a> {
    lists: [&'a [ProfileProvenanceRef]; 4],
    maximum_depth: u32,
}

impl OwnerParents<'_> {
    fn iter(&self) -> impl Iterator<Item = ProfileProvenanceRef> + '_ {
        let mut next = [0_usize; 4];
        std::iter::from_fn(move || {
            let candidate = self
                .lists
                .iter()
                .zip(next)
                .filter_map(|(list, next)| list.get(next))
                .min_by_key(|reference| (reference.segment(), reference.local()))
                .copied()?;
            for (list, next) in self.lists.iter().zip(&mut next) {
                if list.get(*next) == Some(&candidate) {
                    *next += 1;
                }
            }
            Some(candidate)
        })
    }
}

fn owner_parents(
    input: &SceneStorage,
    layout: ProfileOwnerLayout,
    targets: ProfileEditTargets,
    output_owner: CanonicalProfileOwner,
) -> Option<OwnerParents<'_>> {
    let source = input.tables.profile.state.as_ref();
    let mut owners = [None; 4];
    if let (
        ProfileEditTargets::Normals {
            primitive,
            attributes,
            smooth,
        },
        CanonicalProfileOwner::Attribute(index),
    ) = (targets, output_owner)
        && index == attributes.normal_attribute()
    {
        owners[0] = Some(CanonicalProfileOwner::Primitive(primitive));
        owners[1] = Some(CanonicalProfileOwner::Attribute(
            attributes.source_position(),
        ));
        owners[2] = attributes
            .source_normal()
            .map(CanonicalProfileOwner::Attribute);
        owners[3] = if smooth {
            attributes
                .source_smoothing_group()
                .map(CanonicalProfileOwner::Attribute)
        } else {
            None
        };
    } else {
        owners[0] = Some(targets.source_owner(output_owner)?);
        if matches!(targets, ProfileEditTargets::Space(_)) {
            owners[1] = Some(CanonicalProfileOwner::Scene);
        }
    }
    let mut parents = OwnerParents {
        lists: [&[]; 4],
        maximum_depth: 0,
    };
    for (index, owner) in owners.into_iter().enumerate() {
        if let Some(owner) = owner {
            let row = profile_owner(source, layout, owner)?;
            parents.lists[index] = row.attachments.get(&source.attachments)?;
            parents.maximum_depth = parents.maximum_depth.max(row.maximum_depth);
        }
    }
    (parents.maximum_depth != 0).then_some(parents)
}

#[derive(Clone, Copy)]
pub(super) struct NodeSceneEditProfilePlan {
    source_layout: ProfileOwnerLayout,
    output_layout: ProfileOwnerLayout,
    targets: ProfileEditTargets,
    lineage: [CapacityPlan; 3],
    state: ProfileStatePlan,
    metadata: MetadataEditPlan,
    selection_work: u64,
    pub(super) source_segment_count: CheckedCount,
    pub(super) target_count: CheckedCount,
    #[cfg(test)]
    pub(super) target_attachment_count: CheckedCount,
    #[cfg(test)]
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
        let metadata = MetadataEditPlan::try_new(
            &input.tables.profile,
            targets.deleted_owner(),
            &mut ProfilePlanningControl(poll),
        )
        .map_err(|error| match error {
            TypedStorageBuildError::Control(cause) => cause,
            _ => invalid(),
        })?;
        let mut targets_count = 0_u32;
        let mut parents = 0_u32;
        let mut attachments = 0_u32;
        let mut maximum_depth = input.proof.profile_maximum_provenance_depth();
        for ordinal in 0..output_layout.owner_count() {
            poll()?;
            let owner = output_layout.owner_at(ordinal).ok_or_else(invalid)?;
            let count = if targets.selects(owner, input, &mut ProfilePlanningControl(poll))? {
                let sources =
                    owner_parents(input, source_layout, targets, owner).ok_or_else(invalid)?;
                let mut source_count = 0_u32;
                for _ in sources.iter() {
                    poll()?;
                    source_count = source_count
                        .checked_add(1)
                        .ok_or(StorePlanError::SizeOverflow)?;
                }
                targets_count = targets_count
                    .checked_add(1)
                    .ok_or(StorePlanError::SizeOverflow)?;
                parents = parents
                    .checked_add(source_count)
                    .ok_or(StorePlanError::SizeOverflow)?;
                let depth = sources
                    .maximum_depth
                    .checked_add(1)
                    .ok_or(StorePlanError::SizeOverflow)?;
                maximum_depth = maximum_depth.max(depth);
                1
            } else {
                let source_owner = targets
                    .source_owner(owner)
                    .and_then(|owner| profile_owner(source, source_layout, owner))
                    .ok_or_else(invalid)?;
                u32::try_from(source_owner.attachments.len())
                    .map_err(|_| StorePlanError::SizeOverflow)?
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
        counts = metadata.output_counts(counts);
        Ok(Self {
            source_layout,
            output_layout,
            targets,
            metadata,
            selection_work: targets.selection_work(input)?,
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
            #[cfg(test)]
            target_attachment_count: CheckedCount::try_from_u32(
                parents,
                "changed owners' parents",
            )?,
            #[cfg(test)]
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
            .and_then(|work| work.checked_add(self.selection_work))
            .ok_or(StorePlanError::SizeOverflow)
    }

    fn state_work(self, input: &SceneStorage) -> Result<u64, StorePlanError> {
        self.state
            .work()?
            .checked_add(input.tables.profile.state.as_ref().metadata_entries.len() as u64)
            .and_then(|work| work.checked_add(self.selection_work))
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
        let kind = match operation.kind() {
            OperationKind::Transform => StoredOperationKind::Transform,
            OperationKind::Split => StoredOperationKind::Split,
            OperationKind::Generated => StoredOperationKind::Generated,
            _ => return Err(TypedStorageBuildError::Invariant),
        };
        for ordinal in 0..self.output_layout.owner_count() {
            let owner = control
                .run_unit(|| self.output_layout.owner_at(ordinal))
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
            if !self
                .targets
                .selects(owner, input, control)
                .map_err(TypedStorageBuildError::Control)?
            {
                continue;
            }
            let sources = owner_parents(input, self.source_layout, self.targets, owner)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = parents.len();
            for parent in sources.iter() {
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
                    depth: sources
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
            let start = attachments.len();
            let maximum_depth = if self
                .targets
                .selects(owner, input, control)
                .map_err(TypedStorageBuildError::Control)?
            {
                let reference = domain
                    .reference(local)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                local = local
                    .checked_add(1)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                push(&mut attachments, p[2], reference, control)?;
                owner_parents(input, self.source_layout, self.targets, owner)
                    .ok_or(TypedStorageBuildError::Invariant)?
                    .maximum_depth
                    .checked_add(1)
                    .ok_or(TypedStorageBuildError::Invariant)?
            } else {
                let row = self
                    .targets
                    .source_owner(owner)
                    .and_then(|owner| self.source_owner(source, owner))
                    .ok_or(TypedStorageBuildError::Invariant)?;
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
        let mut cursor = MetadataEditCursor::new();
        for &entry in &source.metadata_entries {
            let entry = control
                .run_unit(|| {
                    let Some(owner) = self.targets.output_owner(entry.owner) else {
                        return Ok(None);
                    };
                    if !self.output_layout.contains(owner) {
                        return Err(TypedStorageBuildError::Invariant);
                    }
                    let mut entry = if self.metadata.rewrites_payloads() {
                        cursor
                            .remap(entry)
                            .ok_or(TypedStorageBuildError::Invariant)?
                    } else {
                        entry
                    };
                    entry.owner = owner;
                    Ok(Some(entry))
                })
                .map_err(TypedStorageBuildError::Control)??;
            if let Some(entry) = entry {
                push(&mut metadata_entries, p[3], entry, control)?;
            }
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
    let metadata = if plan.metadata.rewrites_payloads() {
        lane.try_retain_storage(
            plan.metadata.capacities(),
            plan.metadata.work().map_err(StoreRuntimeError::Plan)?,
            plan.metadata.text_bytes() as u64,
            |control| {
                plan.metadata
                    .build_payloads(&input.tables.profile, control)
                    .map_err(map_build)
            },
        )?
    } else {
        lane.try_share_storage(&input.tables.profile.metadata)?
    };
    let state = lane.try_retain_storage_prepared(
        plan.state.capacities(),
        plan.state_work(input).map_err(StoreRuntimeError::Plan)?,
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
