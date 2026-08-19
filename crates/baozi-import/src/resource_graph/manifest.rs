use crate::dependency::{DependencyRequirement, DependencyRoleId};
use crate::metered_storage::AccountedVec;
use baozi_io::AssetPath;
use sha2::{Digest, Sha256};
use std::fmt;

mod encoding;

pub(super) use encoding::{ManifestEncodingError, encode_node_class, encode_relation_key};

pub(super) const MANIFEST_SCHEMA_VERSION: u64 = 1;
const MANIFEST_DOMAIN: &str = "baozi.input-manifest.v1";

/// A fixed SHA-256 digest used by the successful input manifest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestDigest([u8; 32]);

impl ManifestDigest {
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(super) const fn zero() -> Self {
        Self([0; 32])
    }
}

impl fmt::Debug for ManifestDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ManifestDigest")
            .field(&self.0)
            .finish()
    }
}

/// Why a resource identity cannot claim cross-session stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestSessionOpaqueReason {
    GeneratedReportIdentity,
    UnregisteredCallerIdentity,
    UnregisteredResolverIdentity,
}

/// Replay-stability evidence carried with one resource identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManifestResourceIdentityStability {
    Stable {
        owner: &'static str,
        schema: &'static str,
        version: u64,
    },
    SessionOpaque(ManifestSessionOpaqueReason),
}

/// A sanitized public resource identity and its separate replay-stability evidence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManifestResourceIdentity {
    value: AssetPath,
    stability: ManifestResourceIdentityStability,
}

impl ManifestResourceIdentity {
    pub(super) fn new(value: AssetPath, stability: ManifestResourceIdentityStability) -> Self {
        Self { value, stability }
    }

    pub const fn value(&self) -> &AssetPath {
        &self.value
    }

    pub const fn stability(&self) -> &ManifestResourceIdentityStability {
        &self.stability
    }
}

/// Stable public class for an unavailable logical relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestUnavailableClass {
    Reference,
    Policy,
    Io,
    Resource,
    Control,
    TrustedExtension,
    Contract,
}

impl ManifestUnavailableClass {
    const fn canonical_id(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Policy => "policy",
            Self::Io => "io",
            Self::Resource => "resource",
            Self::Control => "control",
            Self::TrustedExtension => "trusted-extension",
            Self::Contract => "contract",
        }
    }
}

/// Stable public code for an unavailable logical relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestUnavailableCode(&'static str);

impl ManifestUnavailableCode {
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    pub(super) const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

/// Why a session-dependent unavailable relation cannot be replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestSessionDependentReason {
    AcquisitionBudget,
    MutablePolicy,
    HostCallback,
    ProcessState,
    UnregisteredSemanticBasis,
}

/// Why an unavailable relation cannot be replayed from immutable proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestUnavailableReplay {
    UnprovenNegativeInput,
    SessionDependent(ManifestSessionDependentReason),
}

/// A logical reference after session normalization or rejection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManifestReference {
    Normalized(ManifestResourceIdentity),
    Rejected {
        class: ManifestUnavailableClass,
        code: ManifestUnavailableCode,
        reference_digest: ManifestDigest,
    },
}

/// Terminal outcome of one logical dependency relation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManifestRelationOutcome {
    Available {
        target_node_class_digest: ManifestDigest,
    },
    Unavailable {
        class: ManifestUnavailableClass,
        code: ManifestUnavailableCode,
        identity_evidence: Option<ManifestResourceIdentity>,
        replay: ManifestUnavailableReplay,
    },
}

/// One quotient relation emitted by a semantic node class.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManifestRelation {
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    reference: ManifestReference,
    outcome: ManifestRelationOutcome,
    multiplicity: u64,
}

impl ManifestRelation {
    pub(super) fn new(
        role: DependencyRoleId,
        requirement: DependencyRequirement,
        reference: ManifestReference,
        outcome: ManifestRelationOutcome,
        multiplicity: u64,
    ) -> Self {
        Self {
            role,
            requirement,
            reference,
            outcome,
            multiplicity,
        }
    }

    pub const fn role(&self) -> &DependencyRoleId {
        &self.role
    }

    pub const fn requirement(&self) -> DependencyRequirement {
        self.requirement
    }

    pub const fn reference(&self) -> &ManifestReference {
        &self.reference
    }

    pub const fn outcome(&self) -> &ManifestRelationOutcome {
        &self.outcome
    }

    pub const fn multiplicity(&self) -> u64 {
        self.multiplicity
    }
}

/// One semantic node class in a successful input manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestNodeClass {
    digest: ManifestDigest,
    identity: ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: Vec<ManifestRelation>,
}

impl ManifestNodeClass {
    pub(super) fn new(
        digest: ManifestDigest,
        identity: ManifestResourceIdentity,
        byte_length: u64,
        content_digest: ManifestDigest,
        outgoing_relations: Vec<ManifestRelation>,
    ) -> Self {
        Self {
            digest,
            identity,
            byte_length,
            content_digest,
            outgoing_relations,
        }
    }

    pub const fn digest(&self) -> ManifestDigest {
        self.digest
    }

    pub const fn identity(&self) -> &ManifestResourceIdentity {
        &self.identity
    }

    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub const fn content_digest(&self) -> ManifestDigest {
        self.content_digest
    }

    pub fn outgoing_relations(&self) -> &[ManifestRelation] {
        &self.outgoing_relations
    }
}

/// Exact semantic quotient counts emitted by the manifest finalizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceManifestCounts {
    pub(super) node_class_count: u64,
    pub(super) quotient_relation_record_count: u64,
    pub(super) relation_occurrence_count: u64,
    pub(super) available_relation_record_count: u64,
    pub(super) available_relation_occurrence_count: u64,
    pub(super) unavailable_relation_record_count: u64,
    pub(super) unavailable_relation_occurrence_count: u64,
    pub(super) maximum_depth: u64,
}

impl ResourceManifestCounts {
    pub const fn node_class_count(self) -> u64 {
        self.node_class_count
    }

    pub const fn quotient_relation_record_count(self) -> u64 {
        self.quotient_relation_record_count
    }

    pub const fn relation_occurrence_count(self) -> u64 {
        self.relation_occurrence_count
    }

    pub const fn available_relation_record_count(self) -> u64 {
        self.available_relation_record_count
    }

    pub const fn available_relation_occurrence_count(self) -> u64 {
        self.available_relation_occurrence_count
    }

    pub const fn unavailable_relation_record_count(self) -> u64 {
        self.unavailable_relation_record_count
    }

    pub const fn unavailable_relation_occurrence_count(self) -> u64 {
        self.unavailable_relation_occurrence_count
    }

    pub const fn maximum_depth(self) -> u64 {
        self.maximum_depth
    }
}

/// Why a successful manifest is observation-only rather than replayable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestObservedOnlyReason {
    UnstableResourceIdentity,
    UnprovenNegativeInput,
    SessionDependentUnavailable,
}

impl ManifestObservedOnlyReason {
    const fn canonical_id(self) -> &'static str {
        match self {
            Self::UnstableResourceIdentity => "unstable-resource-identity",
            Self::UnprovenNegativeInput => "unproven-negative-input",
            Self::SessionDependentUnavailable => "session-dependent-unavailable",
        }
    }
}

/// Cross-session replay classification for the complete successful manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputReplayStatus {
    Replayable,
    ObservedOnly(Vec<ManifestObservedOnlyReason>),
}

impl InputReplayStatus {
    pub fn observed_only_reasons(&self) -> &[ManifestObservedOnlyReason] {
        match self {
            Self::Replayable => &[],
            Self::ObservedOnly(reasons) => reasons,
        }
    }
}

/// Complete successful semantic input manifest.
#[must_use = "the manifest is the replay and dependency evidence for a successful import"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessfulInputManifest {
    schema_version: u64,
    digest: ManifestDigest,
    primary: ManifestDigest,
    node_classes: Vec<ManifestNodeClass>,
    counts: ResourceManifestCounts,
    replay_status: InputReplayStatus,
}

impl SuccessfulInputManifest {
    pub(super) fn new(
        digest: ManifestDigest,
        primary: ManifestDigest,
        node_classes: Vec<ManifestNodeClass>,
        counts: ResourceManifestCounts,
        replay_status: InputReplayStatus,
    ) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            digest,
            primary,
            node_classes,
            counts,
            replay_status,
        }
    }

    pub const fn schema_version(&self) -> u64 {
        self.schema_version
    }

    pub const fn digest(&self) -> ManifestDigest {
        self.digest
    }

    pub const fn primary(&self) -> ManifestDigest {
        self.primary
    }

    pub fn node_classes(&self) -> &[ManifestNodeClass] {
        &self.node_classes
    }

    pub const fn counts(&self) -> ResourceManifestCounts {
        self.counts
    }

    pub const fn replay_status(&self) -> &InputReplayStatus {
        &self.replay_status
    }

    #[cfg(test)]
    pub(super) fn canonical_bytes(&self) -> Result<Vec<u8>, ManifestEncodingError> {
        encoding::encode_manifest(
            self.primary,
            &self.node_classes,
            self.counts,
            &self.replay_status,
        )
    }

    pub(super) fn canonical_len(&self) -> Result<usize, ManifestEncodingError> {
        encoding::manifest_len(
            self.primary,
            &self.node_classes,
            self.counts,
            &self.replay_status,
        )
    }

    pub(super) fn write_canonical_bytes(
        &self,
        expected: usize,
        output: &mut AccountedVec<u8>,
    ) -> Result<(), ManifestEncodingError> {
        encoding::encode_manifest_into_accounted(
            self.primary,
            &self.node_classes,
            self.counts,
            &self.replay_status,
            expected,
            output,
        )
    }

    pub(super) fn with_digest(mut self, digest: ManifestDigest) -> Self {
        self.digest = digest;
        self
    }
}
