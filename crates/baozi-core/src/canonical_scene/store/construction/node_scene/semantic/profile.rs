use crate::canonical_scene::store::production::{CanonicalProfileOwner, SessionOpaqueReason};
use crate::canonical_scene::store::profile_provenance::ProfileProvenanceRef;
use crate::canonical_scene::store::typed_storage::TextRange;
use crate::canonical_scene::store::typed_storage::profile::{
    LineageCounts, LineageSegment, MetadataPayloads, ProfileState, ProfileStorage,
    StoredOperationEvidence, StoredOperationKind, StoredProfileSegment, StoredProvenance,
    StoredProvenanceKind, StoredResource, StoredResourceStability, StoredSource,
    StoredSourceLocator,
};
use crate::provenance::{DigestAlgorithm, SourceObjectKind};

use super::{NodeSceneSemanticResult, invariant};

mod metadata;

use metadata::MetadataCursor;
pub(in crate::canonical_scene::store) use metadata::{
    NodeSceneProfileMetadataArray, NodeSceneProfileMetadataArrayKind,
    NodeSceneProfileMetadataEntry, NodeSceneProfileMetadataValue,
};

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileProvenanceIndices<'scene> {
    references: &'scene [ProfileProvenanceRef],
    profile: &'scene ProfileStorage,
}

impl<'scene> NodeSceneProfileProvenanceIndices<'scene> {
    const fn new(
        profile: &'scene ProfileStorage,
        references: &'scene [ProfileProvenanceRef],
    ) -> Self {
        Self {
            references,
            profile,
        }
    }

    pub(in crate::canonical_scene::store) const fn len(self) -> usize {
        self.references.len()
    }

    pub(in crate::canonical_scene::store) fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = Option<u32>> + 'scene {
        self.references
            .iter()
            .map(move |reference| self.profile.resolve(*reference).map(|(index, _)| index))
    }
}

impl std::fmt::Debug for NodeSceneProfileProvenanceIndices<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<'left, 'right> PartialEq<NodeSceneProfileProvenanceIndices<'right>>
    for NodeSceneProfileProvenanceIndices<'left>
{
    fn eq(&self, other: &NodeSceneProfileProvenanceIndices<'right>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for NodeSceneProfileProvenanceIndices<'_> {}

#[derive(Clone, Copy)]
pub(super) struct NodeSceneProfileTables<'scene> {
    profile: &'scene ProfileStorage,
    state: &'scene ProfileState,
    metadata: &'scene MetadataPayloads,
}

impl<'scene> NodeSceneProfileTables<'scene> {
    pub(super) fn try_new(profile: &'scene ProfileStorage) -> NodeSceneSemanticResult<Self> {
        let state = profile.state.as_ref();
        if state.segments.is_empty() || state.owners.len() < 2 {
            return Err(invariant(
                "validated typed profile is missing segments or required owners",
            ));
        }
        Ok(Self {
            profile,
            state,
            metadata: profile.metadata.as_ref(),
        })
    }

    pub(super) fn into_cursor(self) -> NodeSceneProfileCursor<'scene> {
        NodeSceneProfileCursor {
            tables: self,
            resources: LineageCursor::default(),
            sources: LineageCursor::default(),
            provenance: LineageCursor::default(),
            evidence: LineageCursor::default(),
            next_operation: 0,
            next_owner: 0,
            next_attachment: 0,
            previous_owner: None,
            metadata: MetadataCursor::default(),
        }
    }
}

/// A call opens at most one segment or reads one semantic row. Segment framing
/// produces a checkpoint so snapshot work can yield even through empty domains.
pub(in crate::canonical_scene::store) enum NodeSceneProfileStep<T> {
    Checkpoint,
    Item(T),
    End,
}

#[derive(Default)]
struct LineageCursor<'scene> {
    next_segment: usize,
    next_row: usize,
    prefixes: LineageCounts,
    active: Option<&'scene StoredProfileSegment>,
    finished: bool,
    #[cfg(test)]
    decoded_rows: usize,
    #[cfg(test)]
    segment_checkpoints: usize,
}

impl<'scene> LineageCursor<'scene> {
    fn step<Row: 'scene, View>(
        &mut self,
        profile: &'scene ProfileStorage,
        rows: impl FnOnce(&'scene LineageSegment) -> &'scene [Row],
        read: impl FnOnce(
            &'scene StoredProfileSegment,
            usize,
            &'scene Row,
        ) -> NodeSceneSemanticResult<View>,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<View>> {
        if self.finished {
            return Ok(NodeSceneProfileStep::End);
        }
        if let Some(segment) = self.active {
            let rows = rows(segment.lineage.as_ref());
            if let Some(row) = rows.get(self.next_row) {
                let value = read(segment, self.next_row, row)?;
                self.next_row += 1;
                #[cfg(test)]
                {
                    self.decoded_rows += 1;
                }
                return Ok(NodeSceneProfileStep::Item(value));
            }
            if self.next_row != rows.len() {
                return Err(invariant("profile lineage cursor exceeds its typed rows"));
            }
            self.active = None;
        }
        let state = profile.state.as_ref();
        if self.next_segment == state.segments.len() {
            if self.prefixes != state.lineage_counts {
                return Err(invariant(
                    "validated profile segment totals disagree with typed storage",
                ));
            }
            self.finished = true;
            return Ok(NodeSceneProfileStep::End);
        }
        let segment = state
            .segments
            .get(self.next_segment)
            .ok_or_else(|| invariant("profile lineage cursor exceeds its typed segments"))?;
        let counts = segment.lineage.as_ref().counts();
        if segment.prefixes != self.prefixes || counts == LineageCounts::default() {
            return Err(invariant(
                "validated profile segment prefixes are inconsistent",
            ));
        }
        self.prefixes = self
            .prefixes
            .checked_add(counts)
            .ok_or_else(|| invariant("validated profile segment totals overflow"))?;
        self.next_segment += 1;
        self.next_row = 0;
        self.active = Some(segment);
        #[cfg(test)]
        {
            self.segment_checkpoints += 1;
        }
        Ok(NodeSceneProfileStep::Checkpoint)
    }
}

#[must_use = "the profile cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneProfileCursor<'scene> {
    tables: NodeSceneProfileTables<'scene>,
    resources: LineageCursor<'scene>,
    sources: LineageCursor<'scene>,
    provenance: LineageCursor<'scene>,
    evidence: LineageCursor<'scene>,
    next_operation: usize,
    next_owner: usize,
    next_attachment: usize,
    previous_owner: Option<CanonicalProfileOwner>,
    metadata: MetadataCursor<'scene>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileMetadataLanes<'scene> {
    text: Option<&'scene str>,
    bytes: Option<&'scene [u8]>,
    bools: Option<&'scene [u8]>,
    i64s: Option<&'scene [i64]>,
    u64s: Option<&'scene [u64]>,
    f64s: Option<&'scene [f64]>,
    f32s: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneProfileMetadataLanes<'scene> {
    pub(in crate::canonical_scene::store) const fn text(self) -> Option<&'scene str> {
        self.text
    }

    pub(in crate::canonical_scene::store) const fn bytes(self) -> Option<&'scene [u8]> {
        self.bytes
    }

    pub(in crate::canonical_scene::store) const fn bools(self) -> Option<&'scene [u8]> {
        self.bools
    }

    pub(in crate::canonical_scene::store) const fn i64s(self) -> Option<&'scene [i64]> {
        self.i64s
    }

    pub(in crate::canonical_scene::store) const fn u64s(self) -> Option<&'scene [u64]> {
        self.u64s
    }

    pub(in crate::canonical_scene::store) const fn f64s(self) -> Option<&'scene [f64]> {
        self.f64s
    }

    pub(in crate::canonical_scene::store) const fn f32s(self) -> Option<&'scene [f32]> {
        self.f32s
    }
}

impl<'scene> NodeSceneProfileCursor<'scene> {
    pub(in crate::canonical_scene::store) const fn resource_count(&self) -> usize {
        self.tables.state.lineage_counts.resources
    }

    pub(in crate::canonical_scene::store) const fn source_count(&self) -> usize {
        self.tables.state.lineage_counts.sources
    }

    pub(in crate::canonical_scene::store) const fn provenance_count(&self) -> usize {
        self.tables.state.lineage_counts.provenance
    }

    pub(in crate::canonical_scene::store) const fn operation_count(&self) -> usize {
        self.tables.state.lineage_counts.operations
    }

    pub(in crate::canonical_scene::store) const fn evidence_count(&self) -> usize {
        self.tables.state.lineage_counts.evidence
    }

    pub(in crate::canonical_scene::store) const fn parent_count(&self) -> usize {
        self.tables.state.lineage_counts.parents
    }

    pub(in crate::canonical_scene::store) const fn wide_count(&self) -> usize {
        self.tables.state.lineage_counts.wide
    }

    pub(in crate::canonical_scene::store) const fn lineage_text_bytes(&self) -> usize {
        self.tables.state.lineage_counts.text
    }

    pub(in crate::canonical_scene::store) const fn lineage_digest_bytes(&self) -> usize {
        self.tables.state.lineage_counts.digests
    }

    pub(in crate::canonical_scene::store) const fn owner_count(&self) -> usize {
        self.tables.state.owners.len()
    }

    pub(in crate::canonical_scene::store) const fn attachment_count(&self) -> usize {
        self.tables.state.attachments.len()
    }

    pub(in crate::canonical_scene::store) const fn metadata_entry_count(&self) -> usize {
        self.tables.state.metadata_entries.len()
    }

    pub(in crate::canonical_scene::store) fn metadata_lanes(
        &self,
    ) -> NodeSceneProfileMetadataLanes<'scene> {
        let pools = self.tables.metadata;
        // Empty pools are absent lanes in the semantic transcript.
        NodeSceneProfileMetadataLanes {
            text: (!pools.text.is_empty()).then_some(pools.text.as_str()),
            bytes: (!pools.bytes.is_empty()).then_some(pools.bytes.as_slice()),
            bools: (!pools.bools.is_empty()).then_some(pools.bools.as_slice()),
            i64s: (!pools.i64s.is_empty()).then_some(pools.i64s.as_slice()),
            u64s: (!pools.u64s.is_empty()).then_some(pools.u64s.as_slice()),
            f64s: (!pools.f64s.is_empty()).then_some(pools.f64s.as_slice()),
            f32s: (!pools.f32s.is_empty()).then_some(pools.f32s.as_slice()),
        }
    }

    pub(in crate::canonical_scene::store) fn step_resource(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileResource<'scene>>> {
        self.resources.step(
            self.tables.profile,
            |segment| &segment.resources,
            read_resource,
        )
    }

    pub(in crate::canonical_scene::store) fn step_source(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileSource<'scene>>> {
        self.sources
            .step(self.tables.profile, |segment| &segment.sources, read_source)
    }

    pub(in crate::canonical_scene::store) fn step_provenance(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileProvenance<'scene>>> {
        let profile = self.tables.profile;
        let next_operation = &mut self.next_operation;
        self.provenance.step(
            profile,
            |segment| &segment.provenance,
            |segment, row, value| read_provenance(profile, segment, row, value, next_operation),
        )
    }

    pub(in crate::canonical_scene::store) fn step_evidence(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileEvidence<'scene>>> {
        self.evidence.step(
            self.tables.profile,
            |segment| &segment.evidence,
            read_evidence,
        )
    }

    pub(in crate::canonical_scene::store) fn next_owner(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileOwner<'scene>>> {
        if self.next_owner == self.owner_count() {
            return Ok(None);
        }
        let row = self
            .tables
            .state
            .owners
            .get(self.next_owner)
            .ok_or_else(|| invariant("profile owner cursor exceeds its typed rows"))?;
        require_owner_order(self.next_owner, row.owner, self.previous_owner)?;
        if row.attachments.is_empty() || row.attachments.start() != self.next_attachment {
            return Err(invariant(
                "validated profile attachment range is not canonical",
            ));
        }
        let attachments = row
            .attachments
            .get(&self.tables.state.attachments)
            .ok_or_else(|| invariant("validated profile attachment range is unreadable"))?;
        self.next_attachment = checked_add(row.attachments.start(), attachments.len())?;
        self.next_owner += 1;
        self.previous_owner = Some(row.owner);
        Ok(Some(NodeSceneProfileOwner {
            owner: row.owner,
            attachment_offset: row.attachments.start(),
            attachments: NodeSceneProfileProvenanceIndices::new(self.tables.profile, attachments),
        }))
    }

    pub(in crate::canonical_scene::store) fn next_metadata_entry(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileMetadataEntry<'scene>>> {
        self.metadata.next(self.tables.state, self.tables.metadata)
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        // Observing End also closes each domain's segment totals, including empty domains.
        if !self.resources.finished
            || !self.sources.finished
            || !self.provenance.finished
            || !self.evidence.finished
            || self.next_operation != self.operation_count()
        {
            return Err(invariant(
                "profile cursor ended before all lineage rows were exhausted",
            ));
        }
        if self.next_owner != self.owner_count() || self.next_attachment != self.attachment_count()
        {
            return Err(invariant(
                "profile cursor ended before all owners and attachments were exhausted",
            ));
        }
        self.metadata
            .finish(self.tables.state, self.tables.metadata)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn decoded_lineage_rows_for_test(
        &self,
    ) -> [usize; 4] {
        [
            self.resources.decoded_rows,
            self.sources.decoded_rows,
            self.provenance.decoded_rows,
            self.evidence.decoded_rows,
        ]
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn lineage_segment_checkpoints_for_test(
        &self,
    ) -> [usize; 4] {
        [
            self.resources.segment_checkpoints,
            self.sources.segment_checkpoints,
            self.provenance.segment_checkpoints,
            self.evidence.segment_checkpoints,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileOwner<'scene> {
    owner: CanonicalProfileOwner,
    attachment_offset: usize,
    attachments: NodeSceneProfileProvenanceIndices<'scene>,
}

impl<'scene> NodeSceneProfileOwner<'scene> {
    pub(in crate::canonical_scene::store) const fn owner(self) -> CanonicalProfileOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn attachment_offset(self) -> usize {
        self.attachment_offset
    }

    pub(in crate::canonical_scene::store) const fn attachments(
        self,
    ) -> NodeSceneProfileProvenanceIndices<'scene> {
        self.attachments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileResourceStability<'scene> {
    Stable {
        owner: &'scene str,
        schema: &'scene str,
        schema_version: u32,
    },
    SessionOpaque(SessionOpaqueReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileResource<'scene> {
    index: usize,
    identity: &'scene str,
    stability: NodeSceneProfileResourceStability<'scene>,
}

impl<'scene> NodeSceneProfileResource<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn identity(self) -> &'scene str {
        self.identity
    }

    pub(in crate::canonical_scene::store) const fn stability(
        self,
    ) -> NodeSceneProfileResourceStability<'scene> {
        self.stability
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileSourceLocator<'scene> {
    Indexed {
        kind: SourceObjectKind,
        index: u64,
    },
    Namespaced {
        schema: &'scene str,
        schema_version: u32,
        value: &'scene str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileDigest<'scene> {
    algorithm: DigestAlgorithm,
    bytes: &'scene [u8],
}

impl<'scene> NodeSceneProfileDigest<'scene> {
    pub(in crate::canonical_scene::store) const fn algorithm(self) -> DigestAlgorithm {
        self.algorithm
    }

    pub(in crate::canonical_scene::store) const fn bytes(self) -> &'scene [u8] {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileSource<'scene> {
    index: usize,
    resource: usize,
    format: &'scene str,
    locator: NodeSceneProfileSourceLocator<'scene>,
    display_name: Option<&'scene str>,
    content_digest: Option<NodeSceneProfileDigest<'scene>>,
}

impl<'scene> NodeSceneProfileSource<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn resource(self) -> usize {
        self.resource
    }

    pub(in crate::canonical_scene::store) const fn format(self) -> &'scene str {
        self.format
    }

    pub(in crate::canonical_scene::store) const fn locator(
        self,
    ) -> NodeSceneProfileSourceLocator<'scene> {
        self.locator
    }

    pub(in crate::canonical_scene::store) const fn display_name(self) -> Option<&'scene str> {
        self.display_name
    }

    pub(in crate::canonical_scene::store) const fn content_digest(
        self,
    ) -> Option<NodeSceneProfileDigest<'scene>> {
        self.content_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileOperationKind<'scene> {
    Generated,
    Default,
    Normalize,
    Repair,
    Split,
    Merge,
    Convert,
    Transform,
    Optimize,
    Namespaced(&'scene str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileOperation<'scene> {
    index: usize,
    kind: NodeSceneProfileOperationKind<'scene>,
    options_digest: NodeSceneProfileDigest<'scene>,
    evidence_offset: usize,
    evidence_count: usize,
}

impl<'scene> NodeSceneProfileOperation<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn kind(
        self,
    ) -> NodeSceneProfileOperationKind<'scene> {
        self.kind
    }

    pub(in crate::canonical_scene::store) const fn options_digest(
        self,
    ) -> NodeSceneProfileDigest<'scene> {
        self.options_digest
    }

    pub(in crate::canonical_scene::store) const fn evidence_offset(self) -> usize {
        self.evidence_offset
    }

    pub(in crate::canonical_scene::store) const fn evidence_count(self) -> usize {
        self.evidence_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileProvenanceKind<'scene> {
    Source { source: usize },
    Operation(NodeSceneProfileOperation<'scene>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileProvenance<'scene> {
    index: usize,
    depth: usize,
    parent_offset: usize,
    parents: NodeSceneProfileProvenanceIndices<'scene>,
    kind: NodeSceneProfileProvenanceKind<'scene>,
}

impl<'scene> NodeSceneProfileProvenance<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn depth(self) -> usize {
        self.depth
    }

    pub(in crate::canonical_scene::store) const fn parent_offset(self) -> usize {
        self.parent_offset
    }

    pub(in crate::canonical_scene::store) const fn parents(
        self,
    ) -> NodeSceneProfileProvenanceIndices<'scene> {
        self.parents
    }

    pub(in crate::canonical_scene::store) const fn kind(
        self,
    ) -> NodeSceneProfileProvenanceKind<'scene> {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileEvidence<'scene> {
    index: usize,
    code: &'scene str,
    occurrence_count: u64,
    digest: Option<NodeSceneProfileDigest<'scene>>,
}

impl<'scene> NodeSceneProfileEvidence<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn code(self) -> &'scene str {
        self.code
    }

    pub(in crate::canonical_scene::store) const fn occurrence_count(self) -> u64 {
        self.occurrence_count
    }

    pub(in crate::canonical_scene::store) const fn digest(
        self,
    ) -> Option<NodeSceneProfileDigest<'scene>> {
        self.digest
    }
}

fn read_resource<'scene>(
    segment: &'scene StoredProfileSegment,
    row: usize,
    resource: &'scene StoredResource,
) -> NodeSceneSemanticResult<NodeSceneProfileResource<'scene>> {
    let text = segment.lineage.as_ref().text.as_str();
    let stability = match resource.stability {
        StoredResourceStability::Stable {
            owner,
            schema,
            version,
        } => NodeSceneProfileResourceStability::Stable {
            owner: read_text(text, owner)?,
            schema: read_text(text, schema)?,
            schema_version: version.get(),
        },
        StoredResourceStability::SessionOpaque(reason) => {
            NodeSceneProfileResourceStability::SessionOpaque(reason)
        }
    };
    Ok(NodeSceneProfileResource {
        index: checked_add(segment.prefixes.resources, row)?,
        identity: read_text(text, resource.identity)?,
        stability,
    })
}

fn read_source<'scene>(
    segment: &'scene StoredProfileSegment,
    row: usize,
    source: &'scene StoredSource,
) -> NodeSceneSemanticResult<NodeSceneProfileSource<'scene>> {
    let lineage = segment.lineage.as_ref();
    let resource = index(source.resource)?;
    if resource >= checked_add(segment.prefixes.resources, lineage.resources.len())? {
        return Err(invariant(
            "validated profile source is outside resource space",
        ));
    }
    let text = lineage.text.as_str();
    let locator = match source.locator {
        StoredSourceLocator::Indexed { kind, index } => {
            NodeSceneProfileSourceLocator::Indexed { kind, index }
        }
        StoredSourceLocator::Namespaced {
            schema,
            version,
            value,
        } => NodeSceneProfileSourceLocator::Namespaced {
            schema: read_text(text, schema)?,
            schema_version: version.get(),
            value: read_text(text, value)?,
        },
    };
    Ok(NodeSceneProfileSource {
        index: checked_add(segment.prefixes.sources, row)?,
        resource,
        format: read_text(text, source.format)?,
        locator,
        display_name: source
            .display_name
            .map(|range| read_text(text, range))
            .transpose()?,
        content_digest: source
            .content_digest
            .as_ref()
            .map(|digest| NodeSceneProfileDigest {
                algorithm: digest.algorithm(),
                bytes: digest.as_bytes(),
            }),
    })
}

fn read_provenance<'scene>(
    profile: &'scene ProfileStorage,
    segment: &'scene StoredProfileSegment,
    row: usize,
    provenance: &'scene StoredProvenance,
    next_operation: &mut usize,
) -> NodeSceneSemanticResult<NodeSceneProfileProvenance<'scene>> {
    let lineage = segment.lineage.as_ref();
    let parents = provenance
        .parents
        .get(&lineage.parents)
        .ok_or_else(|| invariant("validated profile provenance parent range is unreadable"))?;
    let depth = index(provenance.depth)?;
    let kind = match provenance.kind {
        StoredProvenanceKind::Source(source) => {
            let source = index(source)?;
            if depth != 1
                || !parents.is_empty()
                || source >= checked_add(segment.prefixes.sources, lineage.sources.len())?
            {
                return Err(invariant(
                    "validated source provenance node is inconsistent",
                ));
            }
            NodeSceneProfileProvenanceKind::Source { source }
        }
        StoredProvenanceKind::Operation(operation) => {
            if depth == 0 {
                return Err(invariant("validated operation provenance depth is zero"));
            }
            let operation = read_operation(segment, index(operation)?)?;
            if operation.index != *next_operation {
                return Err(invariant(
                    "validated profile operations are not fully and uniquely covered",
                ));
            }
            *next_operation = checked_add(*next_operation, 1)?;
            NodeSceneProfileProvenanceKind::Operation(operation)
        }
    };
    Ok(NodeSceneProfileProvenance {
        index: checked_add(segment.prefixes.provenance, row)?,
        depth,
        parent_offset: checked_add(segment.prefixes.parents, provenance.parents.start())?,
        parents: NodeSceneProfileProvenanceIndices::new(profile, parents),
        kind,
    })
}

fn read_operation<'scene>(
    segment: &'scene StoredProfileSegment,
    row: usize,
) -> NodeSceneSemanticResult<NodeSceneProfileOperation<'scene>> {
    let lineage = segment.lineage.as_ref();
    let operation = lineage
        .operations
        .get(row)
        .ok_or_else(|| invariant("validated profile operation row is unreadable"))?;
    let kind = match operation.kind {
        StoredOperationKind::Generated => NodeSceneProfileOperationKind::Generated,
        StoredOperationKind::Default => NodeSceneProfileOperationKind::Default,
        StoredOperationKind::Normalize => NodeSceneProfileOperationKind::Normalize,
        StoredOperationKind::Repair => NodeSceneProfileOperationKind::Repair,
        StoredOperationKind::Split => NodeSceneProfileOperationKind::Split,
        StoredOperationKind::Merge => NodeSceneProfileOperationKind::Merge,
        StoredOperationKind::Convert => NodeSceneProfileOperationKind::Convert,
        StoredOperationKind::Transform => NodeSceneProfileOperationKind::Transform,
        StoredOperationKind::Optimize => NodeSceneProfileOperationKind::Optimize,
        StoredOperationKind::Namespaced(name) => {
            NodeSceneProfileOperationKind::Namespaced(read_text(&lineage.text, name)?)
        }
    };
    let evidence = operation
        .evidence
        .checked(lineage.evidence.len())
        .ok_or_else(|| invariant("validated profile operation evidence range is unreadable"))?;
    Ok(NodeSceneProfileOperation {
        index: checked_add(segment.prefixes.operations, row)?,
        kind,
        options_digest: NodeSceneProfileDigest {
            algorithm: operation.options.algorithm(),
            bytes: operation.options.as_bytes(),
        },
        evidence_offset: checked_add(segment.prefixes.evidence, evidence.start)?,
        evidence_count: operation.evidence.len(),
    })
}

fn read_evidence<'scene>(
    segment: &'scene StoredProfileSegment,
    row: usize,
    evidence: &'scene StoredOperationEvidence,
) -> NodeSceneSemanticResult<NodeSceneProfileEvidence<'scene>> {
    Ok(NodeSceneProfileEvidence {
        index: checked_add(segment.prefixes.evidence, row)?,
        code: read_text(&segment.lineage.as_ref().text, evidence.code)?,
        occurrence_count: evidence.occurrences.get(),
        digest: evidence
            .digest
            .as_ref()
            .map(|digest| NodeSceneProfileDigest {
                algorithm: digest.algorithm(),
                bytes: digest.as_bytes(),
            }),
    })
}

fn read_text(text: &str, range: TextRange) -> NodeSceneSemanticResult<&str> {
    range
        .get_str(text)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invariant("validated profile text range is unreadable or empty"))
}

fn checked_add(left: usize, right: usize) -> NodeSceneSemanticResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| invariant("validated profile count overflows"))
}

fn index(value: u32) -> NodeSceneSemanticResult<usize> {
    usize::try_from(value).map_err(|_| invariant("validated profile index does not fit usize"))
}

fn require_owner_order(
    row: usize,
    owner: CanonicalProfileOwner,
    previous: Option<CanonicalProfileOwner>,
) -> NodeSceneSemanticResult<()> {
    if row == 0 && owner != CanonicalProfileOwner::Scene
        || row == 1 && owner != CanonicalProfileOwner::SyntheticRoot
        || row > 1 && previous.is_none_or(|previous| owner <= previous)
    {
        return Err(invariant(
            "validated profile owners are not canonically ordered",
        ));
    }
    Ok(())
}
