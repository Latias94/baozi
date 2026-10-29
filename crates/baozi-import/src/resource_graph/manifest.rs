use crate::dependency::{DependencyRequirement, DependencyRoleId};
use crate::metered_storage::AccountedVec;
use baozi_io::AssetPath;
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::fmt;

mod encoding;

pub(crate) use encoding::registered_negative_proof_canonical_len;
pub(super) use encoding::{
    ManifestEncodingError, encode_node_class_into_accounted, encode_relation_key_into_accounted,
    node_class_len, registered_negative_proof_replay, relation_key_len,
};

pub(super) const MANIFEST_SCHEMA_VERSION: u64 = 1;
const MANIFEST_DOMAIN: &str = "baozi.input-manifest.v1";

/// A fixed SHA-256 digest used by the successful input manifest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestDigest([u8; 32]);

impl ManifestDigest {
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[cfg(test)]
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(super) const fn zero() -> Self {
        Self([0; 32])
    }

    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
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

/// One stable semantic identity embedded by the successful-input manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestSemanticIdentity {
    owner: &'static str,
    schema: &'static str,
    version: u64,
    digest: ManifestDigest,
}

impl ManifestSemanticIdentity {
    pub(super) const fn new(
        owner: &'static str,
        schema: &'static str,
        version: u64,
        digest: ManifestDigest,
    ) -> Self {
        Self {
            owner,
            schema,
            version,
            digest,
        }
    }

    pub const fn owner(self) -> &'static str {
        self.owner
    }

    pub const fn schema(self) -> &'static str {
        self.schema
    }

    pub const fn version(self) -> u64 {
        self.version
    }

    pub const fn digest(self) -> ManifestDigest {
        self.digest
    }
}

/// An admitted namespace-snapshot negative proof.
///
/// The exact target identity comes from the enclosing unavailable outcome. The proof retains the
/// registered cause class/code that its digest binds, which can be narrower than the manifest's
/// outer relation classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestNamespaceSnapshotProof {
    scheme: &'static str,
    version: u64,
    validity_rule: &'static str,
    resolver_identity: ManifestSemanticIdentity,
    namespace_snapshot_digest: ManifestDigest,
    cause: ManifestRegisteredUnavailableCause,
    proof_digest: ManifestDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct ManifestRegisteredUnavailableCause {
    class: &'static str,
    code: &'static str,
}

impl ManifestRegisteredUnavailableCause {
    pub(super) const fn new(class: &'static str, code: &'static str) -> Self {
        Self { class, code }
    }
}

impl ManifestNamespaceSnapshotProof {
    pub(super) const fn new(
        scheme: &'static str,
        version: u64,
        validity_rule: &'static str,
        resolver_identity: ManifestSemanticIdentity,
        namespace_snapshot_digest: ManifestDigest,
        cause: ManifestRegisteredUnavailableCause,
        proof_digest: ManifestDigest,
    ) -> Self {
        Self {
            scheme,
            version,
            validity_rule,
            resolver_identity,
            namespace_snapshot_digest,
            cause,
            proof_digest,
        }
    }

    pub const fn scheme(self) -> &'static str {
        self.scheme
    }

    pub const fn version(self) -> u64 {
        self.version
    }

    pub const fn validity_rule(self) -> &'static str {
        self.validity_rule
    }

    pub const fn resolver_identity(self) -> ManifestSemanticIdentity {
        self.resolver_identity
    }

    pub const fn namespace_snapshot_digest(self) -> ManifestDigest {
        self.namespace_snapshot_digest
    }

    pub const fn cause_class(self) -> &'static str {
        self.cause.class
    }

    pub const fn cause_code(self) -> &'static str {
        self.cause.code
    }

    pub const fn proof_digest(self) -> ManifestDigest {
        self.proof_digest
    }
}

/// An admitted per-reference negative proof.
///
/// V1 reserves this closed union member even though the initial registry has no producer for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestPerReferenceProof {
    scheme: &'static str,
    version: u64,
    validity_rule: &'static str,
    resolver_identity: ManifestSemanticIdentity,
    assertion_digest: ManifestDigest,
    cause: ManifestRegisteredUnavailableCause,
    proof_digest: ManifestDigest,
}

impl ManifestPerReferenceProof {
    pub const fn scheme(self) -> &'static str {
        self.scheme
    }

    pub const fn version(self) -> u64 {
        self.version
    }

    pub const fn validity_rule(self) -> &'static str {
        self.validity_rule
    }

    pub const fn resolver_identity(self) -> ManifestSemanticIdentity {
        self.resolver_identity
    }

    pub const fn assertion_digest(self) -> ManifestDigest {
        self.assertion_digest
    }

    pub const fn cause_class(self) -> &'static str {
        self.cause.class
    }

    pub const fn cause_code(self) -> &'static str {
        self.cause.code
    }

    pub const fn proof_digest(self) -> ManifestDigest {
        self.proof_digest
    }
}

/// Why an unavailable relation cannot be replayed from immutable proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ManifestUnavailableReplay {
    NamespaceSnapshotProof(ManifestNamespaceSnapshotProof),
    PerReferenceProof(ManifestPerReferenceProof),
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
// The manifest arena accounts the complete inline relation size. Boxing the
// bounded proof union would introduce a fallible allocation outside that
// reservation and weaken the manifest's ownership model.
#[allow(clippy::large_enum_variant)]
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

    pub(super) fn with_multiplicity(mut self, multiplicity: u64) -> Self {
        self.multiplicity = multiplicity;
        self
    }
}

/// One semantic node class in a successful input manifest.
#[derive(Debug, PartialEq, Eq)]
pub struct ManifestNodeClass {
    digest: ManifestDigest,
    identity: ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: AccountedVec<ManifestRelation>,
}

impl ManifestNodeClass {
    pub(super) fn new(
        digest: ManifestDigest,
        identity: ManifestResourceIdentity,
        byte_length: u64,
        content_digest: ManifestDigest,
        outgoing_relations: AccountedVec<ManifestRelation>,
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

    fn release_live_reservation(&mut self) {
        self.outgoing_relations.release_capacity_reservation();
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

/// Complete bounded reason set for an observation-only manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestObservedOnlyReasons {
    values: [ManifestObservedOnlyReason; 3],
    len: u8,
}

impl ManifestObservedOnlyReasons {
    pub(super) const fn new() -> Self {
        Self {
            values: [ManifestObservedOnlyReason::UnstableResourceIdentity; 3],
            len: 0,
        }
    }

    pub(super) fn insert(&mut self, reason: ManifestObservedOnlyReason) {
        if self.as_slice().contains(&reason) {
            return;
        }
        let index = usize::from(self.len);
        assert!(
            index < self.values.len(),
            "the closed manifest replay reason set has three members"
        );
        self.values[index] = reason;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[ManifestObservedOnlyReason] {
        &self.values[..usize::from(self.len)]
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Cross-session replay classification for the complete successful manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputReplayStatus {
    Replayable,
    ObservedOnly(ManifestObservedOnlyReasons),
}

impl InputReplayStatus {
    pub fn observed_only_reasons(&self) -> &[ManifestObservedOnlyReason] {
        match self {
            Self::Replayable => &[],
            Self::ObservedOnly(reasons) => reasons.as_slice(),
        }
    }
}

/// Complete successful semantic input manifest.
#[must_use = "the manifest is the replay and dependency evidence for a successful import"]
#[derive(Debug, PartialEq, Eq)]
pub struct SuccessfulInputManifest {
    schema_version: u64,
    digest: ManifestDigest,
    primary: ManifestDigest,
    node_classes: AccountedVec<ManifestNodeClass>,
    counts: ResourceManifestCounts,
    replay_status: InputReplayStatus,
}

impl SuccessfulInputManifest {
    pub(super) fn new(
        digest: ManifestDigest,
        primary: ManifestDigest,
        node_classes: AccountedVec<ManifestNodeClass>,
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

    pub(super) fn release_live_reservations(&mut self) {
        for node_class in self.node_classes.iter_mut() {
            node_class.release_live_reservation();
        }
        self.node_classes.release_capacity_reservation();
    }
}
