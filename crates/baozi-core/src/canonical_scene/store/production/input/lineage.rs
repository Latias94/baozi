use std::num::NonZeroU32;

use crate::FormatId;
use crate::namespaced::NamespacedName;
use crate::provenance::{
    ContentDigest, OperationEvidenceSummary, OperationKind, OptionsDigest, ResourceIdentity,
    SourceDisplayName, SourceObjectLocator,
};

use super::NodeLocalId;

/// Why an otherwise valid resource identity is not replay-stable across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) enum SessionOpaqueReason {
    CallerLabel,
    Generated,
    UnregisteredResolver,
    SessionFallback,
}

/// Replay-stability evidence kept separate from resource text and content identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) enum ResourceIdentityStability {
    Stable {
        owner: NamespacedName,
        schema: NamespacedName,
        version: NonZeroU32,
    },
    SessionOpaque(SessionOpaqueReason),
}

impl ResourceIdentityStability {
    pub(in crate::canonical_scene::store) const fn stable(
        owner: NamespacedName,
        schema: NamespacedName,
        version: NonZeroU32,
    ) -> Self {
        Self::Stable {
            owner,
            schema,
            version,
        }
    }

    pub(in crate::canonical_scene::store) const fn session_opaque(
        reason: SessionOpaqueReason,
    ) -> Self {
        Self::SessionOpaque(reason)
    }
}

/// Identity of an observed resource within one profile authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct ResourceIdentityLocalId(u32);

impl ResourceIdentityLocalId {
    pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One observed resource and its independent replay-stability classification.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ObservedResourceAuthoring<'input> {
    identity: &'input ResourceIdentity,
    stability: &'input ResourceIdentityStability,
}

impl<'input> ObservedResourceAuthoring<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        identity: &'input ResourceIdentity,
        stability: &'input ResourceIdentityStability,
    ) -> Self {
        Self {
            identity,
            stability,
        }
    }

    pub(in crate::canonical_scene::store) const fn identity(self) -> &'input ResourceIdentity {
        self.identity
    }

    pub(in crate::canonical_scene::store) const fn stability(
        self,
    ) -> &'input ResourceIdentityStability {
        self.stability
    }
}

/// Identity of a source object within one profile authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct SourceIdentityLocalId(u32);

impl SourceIdentityLocalId {
    pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One source object identity before references are compiled to retained rows.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct SourceIdentityAuthoring<'input> {
    resource: ResourceIdentityLocalId,
    format: &'input FormatId,
    locator: &'input SourceObjectLocator,
    display_name: Option<&'input SourceDisplayName>,
    content_digest: Option<ContentDigest>,
}

impl<'input> SourceIdentityAuthoring<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        resource: ResourceIdentityLocalId,
        format: &'input FormatId,
        locator: &'input SourceObjectLocator,
    ) -> Self {
        Self {
            resource,
            format,
            locator,
            display_name: None,
            content_digest: None,
        }
    }

    pub(in crate::canonical_scene::store) const fn with_display_name(
        mut self,
        display_name: &'input SourceDisplayName,
    ) -> Self {
        self.display_name = Some(display_name);
        self
    }

    pub(in crate::canonical_scene::store) const fn with_content_digest(
        mut self,
        content_digest: ContentDigest,
    ) -> Self {
        self.content_digest = Some(content_digest);
        self
    }

    pub(in crate::canonical_scene::store) const fn resource(self) -> ResourceIdentityLocalId {
        self.resource
    }

    pub(in crate::canonical_scene::store) const fn format(self) -> &'input FormatId {
        self.format
    }

    pub(in crate::canonical_scene::store) const fn locator(self) -> &'input SourceObjectLocator {
        self.locator
    }

    pub(in crate::canonical_scene::store) const fn display_name(
        self,
    ) -> Option<&'input SourceDisplayName> {
        self.display_name
    }

    pub(in crate::canonical_scene::store) const fn content_digest(self) -> Option<ContentDigest> {
        self.content_digest
    }
}

/// Identity of a provenance node within one topologically ordered authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct ProvenanceLocalId(u32);

impl ProvenanceLocalId {
    pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One operation node with ordered parent references and bounded durable evidence.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProvenanceOperationAuthoring<'input> {
    kind: &'input OperationKind,
    options_digest: OptionsDigest,
    parents: &'input [ProvenanceLocalId],
    evidence: &'input [OperationEvidenceSummary],
}

impl<'input> ProvenanceOperationAuthoring<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        kind: &'input OperationKind,
        options_digest: OptionsDigest,
        parents: &'input [ProvenanceLocalId],
        evidence: &'input [OperationEvidenceSummary],
    ) -> Self {
        Self {
            kind,
            options_digest,
            parents,
            evidence,
        }
    }

    pub(in crate::canonical_scene::store) const fn kind(self) -> &'input OperationKind {
        self.kind
    }

    pub(in crate::canonical_scene::store) const fn options_digest(self) -> OptionsDigest {
        self.options_digest
    }

    pub(in crate::canonical_scene::store) const fn parents(self) -> &'input [ProvenanceLocalId] {
        self.parents
    }

    pub(in crate::canonical_scene::store) const fn evidence(
        self,
    ) -> &'input [OperationEvidenceSummary] {
        self.evidence
    }
}

/// A source leaf or operation node in producer-supplied topological order.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum ProvenanceNodeAuthoring<'input> {
    Source(SourceIdentityLocalId),
    Operation(ProvenanceOperationAuthoring<'input>),
}

impl<'input> ProvenanceNodeAuthoring<'input> {
    pub(in crate::canonical_scene::store) const fn source(source: SourceIdentityLocalId) -> Self {
        Self::Source(source)
    }

    pub(in crate::canonical_scene::store) const fn operation(
        operation: ProvenanceOperationAuthoring<'input>,
    ) -> Self {
        Self::Operation(operation)
    }
}

/// Every canonical owner class that must carry one non-empty provenance row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) enum ProfileOwner {
    Scene,
    SyntheticRoot,
    Node(NodeLocalId),
    Camera(u32),
    Light(u32),
    Image(u32),
    Sampler(u32),
    Texture(u32),
    TextureBinding(u32),
    Material(u32),
    LogicalMesh(u32),
    Primitive(u32),
    Attribute(u32),
    MeshInstance(u32),
    Skin(u32),
    MorphDefinition(u32),
    MorphDelta(u32),
    AnimationClip(u32),
    AnimationChannel(u32),
}

/// One owner row before source-local nodes are normalized to canonical node indices.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct OwnerProvenanceAuthoring<'input> {
    owner: ProfileOwner,
    provenance: &'input [ProvenanceLocalId],
}

impl<'input> OwnerProvenanceAuthoring<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        owner: ProfileOwner,
        provenance: &'input [ProvenanceLocalId],
    ) -> Self {
        Self { owner, provenance }
    }

    pub(in crate::canonical_scene::store) const fn owner(self) -> ProfileOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn provenance(self) -> &'input [ProvenanceLocalId] {
        self.provenance
    }
}

/// Borrowed retained-profile input with identity, lineage, and owner tables kept distinct.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct LineageAuthoringBatch<'input> {
    resources: &'input [ObservedResourceAuthoring<'input>],
    source_identities: &'input [SourceIdentityAuthoring<'input>],
    provenance_nodes: &'input [ProvenanceNodeAuthoring<'input>],
    owner_provenance: &'input [OwnerProvenanceAuthoring<'input>],
}

impl<'input> LineageAuthoringBatch<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        resources: &'input [ObservedResourceAuthoring<'input>],
        source_identities: &'input [SourceIdentityAuthoring<'input>],
        provenance_nodes: &'input [ProvenanceNodeAuthoring<'input>],
        owner_provenance: &'input [OwnerProvenanceAuthoring<'input>],
    ) -> Self {
        Self {
            resources,
            source_identities,
            provenance_nodes,
            owner_provenance,
        }
    }

    pub(in crate::canonical_scene::store) const fn resources(
        self,
    ) -> &'input [ObservedResourceAuthoring<'input>] {
        self.resources
    }

    pub(in crate::canonical_scene::store) const fn source_identities(
        self,
    ) -> &'input [SourceIdentityAuthoring<'input>] {
        self.source_identities
    }

    pub(in crate::canonical_scene::store) const fn provenance_nodes(
        self,
    ) -> &'input [ProvenanceNodeAuthoring<'input>] {
        self.provenance_nodes
    }

    pub(in crate::canonical_scene::store) const fn owner_provenance(
        self,
    ) -> &'input [OwnerProvenanceAuthoring<'input>] {
        self.owner_provenance
    }
}
