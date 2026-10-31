use std::num::NonZeroU32;
#[cfg(test)]
use std::sync::OnceLock;

use crate::FormatId;
use crate::namespaced::NamespacedName;
use crate::provenance::{
    ContentDigest, OperationEvidenceSummary, OperationKind, OptionsDigest, ResourceIdentity,
    SourceDisplayName, SourceObjectLocator,
};

use super::NodeLocalId;

/// Why an otherwise valid resource identity is not replay-stable across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionOpaqueReason {
    CallerLabel,
    Generated,
    UnregisteredResolver,
    SessionFallback,
}

/// A read-only replay-stability classification attached to an observed identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) enum ResourceIdentityStability {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "stable identity semantics remain available, but only the sealed fixture authority currently issues evidence"
        )
    )]
    Stable {
        owner: NamespacedName,
        schema: NamespacedName,
        version: NonZeroU32,
    },
    SessionOpaque(SessionOpaqueReason),
}

static SESSION_OPAQUE_CALLER_LABEL: ResourceIdentityStability =
    ResourceIdentityStability::SessionOpaque(SessionOpaqueReason::CallerLabel);
static SESSION_OPAQUE_GENERATED: ResourceIdentityStability =
    ResourceIdentityStability::SessionOpaque(SessionOpaqueReason::Generated);
static SESSION_OPAQUE_UNREGISTERED_RESOLVER: ResourceIdentityStability =
    ResourceIdentityStability::SessionOpaque(SessionOpaqueReason::UnregisteredResolver);
static SESSION_OPAQUE_SESSION_FALLBACK: ResourceIdentityStability =
    ResourceIdentityStability::SessionOpaque(SessionOpaqueReason::SessionFallback);

const fn session_opaque_stability(
    reason: SessionOpaqueReason,
) -> &'static ResourceIdentityStability {
    match reason {
        SessionOpaqueReason::CallerLabel => &SESSION_OPAQUE_CALLER_LABEL,
        SessionOpaqueReason::Generated => &SESSION_OPAQUE_GENERATED,
        SessionOpaqueReason::UnregisteredResolver => &SESSION_OPAQUE_UNREGISTERED_RESOLVER,
        SessionOpaqueReason::SessionFallback => &SESSION_OPAQUE_SESSION_FALLBACK,
    }
}

/// Sealed authority evidence that binds one exact observed resource identity.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StableResourceIdentityEvidence<'input> {
    identity: &'input ResourceIdentity,
    stability: &'static ResourceIdentityStability,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum StableResourceIdentityFixture {
    ProfileModelV1,
}

#[cfg(test)]
struct StableResourceIdentityFixtureAuthority {
    identity: ResourceIdentity,
    stability: ResourceIdentityStability,
}

#[cfg(test)]
fn profile_model_v1_authority() -> &'static StableResourceIdentityFixtureAuthority {
    static AUTHORITY: OnceLock<StableResourceIdentityFixtureAuthority> = OnceLock::new();
    AUTHORITY.get_or_init(|| StableResourceIdentityFixtureAuthority {
        identity: ResourceIdentity::new("fixtures/profile/stable-model.glb")
            .expect("stable fixture identity must be valid"),
        stability: ResourceIdentityStability::Stable {
            owner: NamespacedName::new("baozi-tests:profile-authority")
                .expect("stable fixture owner must be valid"),
            schema: NamespacedName::new("baozi-tests:profile-resource")
                .expect("stable fixture schema must be valid"),
            version: NonZeroU32::new(1).expect("stable fixture version must be nonzero"),
        },
    })
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn classify_stable_resource_identity_fixture(
    fixture: StableResourceIdentityFixture,
) -> StableResourceIdentityEvidence<'static> {
    let authority = match fixture {
        StableResourceIdentityFixture::ProfileModelV1 => profile_model_v1_authority(),
    };
    StableResourceIdentityEvidence {
        identity: &authority.identity,
        stability: &authority.stability,
    }
}

/// Identity of an observed resource within one profile authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceIdentityLocalId(u32);

impl ResourceIdentityLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One observed resource with an inseparable replay-stability classification.
#[derive(Debug, Clone, Copy)]
pub struct ObservedResourceAuthoring<'input> {
    identity: &'input ResourceIdentity,
    stability: &'static ResourceIdentityStability,
}

impl<'input> ObservedResourceAuthoring<'input> {
    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn stable(
        evidence: StableResourceIdentityEvidence<'input>,
    ) -> Self {
        Self {
            identity: evidence.identity,
            stability: evidence.stability,
        }
    }

    pub const fn session_opaque(
        identity: &'input ResourceIdentity,
        reason: SessionOpaqueReason,
    ) -> Self {
        Self {
            identity,
            stability: session_opaque_stability(reason),
        }
    }

    pub(in crate::canonical_scene::store) const fn identity(self) -> &'input ResourceIdentity {
        self.identity
    }

    pub(in crate::canonical_scene::store) const fn stability(
        self,
    ) -> &'static ResourceIdentityStability {
        self.stability
    }
}

/// Identity of a source object within one profile authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceIdentityLocalId(u32);

impl SourceIdentityLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One source object identity before references are compiled to retained rows.
#[derive(Debug, Clone, Copy)]
pub struct SourceIdentityAuthoring<'input> {
    resource: ResourceIdentityLocalId,
    format: &'input FormatId,
    locator: &'input SourceObjectLocator,
    display_name: Option<&'input SourceDisplayName>,
    content_digest: Option<ContentDigest>,
}

impl<'input> SourceIdentityAuthoring<'input> {
    pub const fn new(
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

    pub const fn with_display_name(mut self, display_name: &'input SourceDisplayName) -> Self {
        self.display_name = Some(display_name);
        self
    }

    pub const fn with_content_digest(mut self, content_digest: ContentDigest) -> Self {
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
pub struct ProvenanceLocalId(u32);

impl ProvenanceLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// One operation node with ordered parent references and bounded durable evidence.
#[derive(Debug, Clone, Copy)]
pub struct ProvenanceOperationAuthoring<'input> {
    kind: &'input OperationKind,
    options_digest: OptionsDigest,
    parents: &'input [ProvenanceLocalId],
    evidence: &'input [OperationEvidenceSummary],
}

impl<'input> ProvenanceOperationAuthoring<'input> {
    pub const fn new(
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
pub enum ProvenanceNodeAuthoring<'input> {
    Source(SourceIdentityLocalId),
    Operation(ProvenanceOperationAuthoring<'input>),
}

impl<'input> ProvenanceNodeAuthoring<'input> {
    pub const fn source(source: SourceIdentityLocalId) -> Self {
        Self::Source(source)
    }

    pub const fn operation(operation: ProvenanceOperationAuthoring<'input>) -> Self {
        Self::Operation(operation)
    }
}

/// Every canonical owner class that must carry one non-empty provenance row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProfileOwner {
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
pub struct OwnerProvenanceAuthoring<'input> {
    owner: ProfileOwner,
    provenance: &'input [ProvenanceLocalId],
}

impl<'input> OwnerProvenanceAuthoring<'input> {
    pub const fn new(owner: ProfileOwner, provenance: &'input [ProvenanceLocalId]) -> Self {
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
pub struct LineageAuthoringBatch<'input> {
    resources: &'input [ObservedResourceAuthoring<'input>],
    source_identities: &'input [SourceIdentityAuthoring<'input>],
    provenance_nodes: &'input [ProvenanceNodeAuthoring<'input>],
    owner_provenance: &'input [OwnerProvenanceAuthoring<'input>],
}

impl<'input> LineageAuthoringBatch<'input> {
    pub const fn new(
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
