use crate::dependency::{DependencyRequirement, DependencyRoleId};
use baozi_io::AssetPath;
use sha2::{Digest, Sha256};
use std::fmt;

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

    pub(super) fn canonical_bytes(&self) -> Result<Vec<u8>, ManifestEncodingError> {
        encode_manifest(
            self.primary,
            &self.node_classes,
            self.counts,
            &self.replay_status,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManifestEncodingError {
    LengthOverflow,
    InvalidCanonicalId,
}

pub(super) fn encode_node_class(
    identity: &ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: &[ManifestRelation],
) -> Result<Vec<u8>, ManifestEncodingError> {
    let relations = outgoing_relations
        .iter()
        .map(encode_relation)
        .collect::<Result<Vec<_>, _>>()?;
    record_value(
        "node-class",
        vec![
            field("resource-identity", encode_resource_identity(identity)?)?,
            field("byte-length", u64_value(byte_length)?)?,
            field("content-digest", digest_value(content_digest)?)?,
            field("outgoing-relations", sequence_value(relations)?)?,
        ],
    )
}

pub(super) fn encode_relation_key(
    role: &DependencyRoleId,
    requirement: DependencyRequirement,
    reference: &ManifestReference,
    outcome: &ManifestRelationOutcome,
) -> Result<Vec<u8>, ManifestEncodingError> {
    record_value(
        "relation-key",
        vec![
            field("role", enum_value(role.as_str())?)?,
            field("requirement", enum_value(requirement_id(requirement))?)?,
            field("reference", encode_reference(reference)?)?,
            field("outcome", encode_outcome(outcome)?)?,
        ],
    )
}

fn encode_manifest(
    primary: ManifestDigest,
    node_classes: &[ManifestNodeClass],
    counts: ResourceManifestCounts,
    replay_status: &InputReplayStatus,
) -> Result<Vec<u8>, ManifestEncodingError> {
    let node_entries = node_classes
        .iter()
        .map(|class| {
            record_value(
                "node-entry",
                vec![
                    field("node-class-digest", digest_value(class.digest)?)?,
                    field(
                        "node-class",
                        encode_node_class(
                            &class.identity,
                            class.byte_length,
                            class.content_digest,
                            &class.outgoing_relations,
                        )?,
                    )?,
                ],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    record_value(
        "input-manifest",
        vec![
            field("domain", utf8_value(MANIFEST_DOMAIN.as_bytes())?)?,
            field("schema-version", u64_value(MANIFEST_SCHEMA_VERSION)?)?,
            field("digest-algorithm", enum_value("sha256")?)?,
            field(
                "primary",
                record_value(
                    "primary",
                    vec![field("node-class-digest", digest_value(primary)?)?],
                )?,
            )?,
            field("node-classes", sequence_value(node_entries)?)?,
            field("counts", encode_counts(counts)?)?,
            field("input-replay-status", encode_replay_status(replay_status)?)?,
        ],
    )
}

fn encode_relation(relation: &ManifestRelation) -> Result<Vec<u8>, ManifestEncodingError> {
    record_value(
        "relation",
        vec![
            field(
                "key",
                encode_relation_key(
                    &relation.role,
                    relation.requirement,
                    &relation.reference,
                    &relation.outcome,
                )?,
            )?,
            field("multiplicity", u64_value(relation.multiplicity)?)?,
        ],
    )
}

fn encode_resource_identity(
    identity: &ManifestResourceIdentity,
) -> Result<Vec<u8>, ManifestEncodingError> {
    let stability = match &identity.stability {
        ManifestResourceIdentityStability::Stable {
            owner,
            schema,
            version,
        } => union_value(
            "stable-resource-identity",
            vec![
                field("owner", enum_value(owner)?)?,
                field("schema-id", enum_value(schema)?)?,
                field("schema-version", u64_value(*version)?)?,
            ],
        )?,
        ManifestResourceIdentityStability::SessionOpaque(reason) => union_value(
            "session-opaque-resource-identity",
            vec![field(
                "reason",
                enum_value(session_opaque_reason_id(*reason))?,
            )?],
        )?,
    };
    record_value(
        "resource-identity-v1",
        vec![
            field("value", utf8_value(identity.value.as_str().as_bytes())?)?,
            field("stability", stability)?,
        ],
    )
}

fn encode_reference(reference: &ManifestReference) -> Result<Vec<u8>, ManifestEncodingError> {
    match reference {
        ManifestReference::Normalized(identity) => union_value(
            "normalized-reference",
            vec![field(
                "resource-identity",
                encode_resource_identity(identity)?,
            )?],
        ),
        ManifestReference::Rejected {
            class,
            code,
            reference_digest,
        } => union_value(
            "rejected-reference",
            vec![
                field("class", enum_value(class.canonical_id())?)?,
                field("code", enum_value(code.as_str())?)?,
                field("reference-digest", digest_value(*reference_digest)?)?,
            ],
        ),
    }
}

fn encode_outcome(outcome: &ManifestRelationOutcome) -> Result<Vec<u8>, ManifestEncodingError> {
    match outcome {
        ManifestRelationOutcome::Available {
            target_node_class_digest,
        } => union_value(
            "available",
            vec![field(
                "target-node-class-digest",
                digest_value(*target_node_class_digest)?,
            )?],
        ),
        ManifestRelationOutcome::Unavailable {
            class,
            code,
            identity_evidence,
            replay,
        } => union_value(
            "unavailable",
            vec![
                field("class", enum_value(class.canonical_id())?)?,
                field("code", enum_value(code.as_str())?)?,
                field(
                    "identity-evidence",
                    optional_value(
                        identity_evidence
                            .as_ref()
                            .map(encode_resource_identity)
                            .transpose()?,
                    )?,
                )?,
                field("replay-classification", encode_unavailable_replay(*replay)?)?,
            ],
        ),
    }
}

fn encode_unavailable_replay(
    replay: ManifestUnavailableReplay,
) -> Result<Vec<u8>, ManifestEncodingError> {
    match replay {
        ManifestUnavailableReplay::UnprovenNegativeInput => union_value(
            "unproven-negative-input",
            vec![field("reason", enum_value("missing-negative-proof")?)?],
        ),
        ManifestUnavailableReplay::SessionDependent(reason) => union_value(
            "session-dependent-unavailable",
            vec![field(
                "reason",
                enum_value(session_dependent_reason_id(reason))?,
            )?],
        ),
    }
}

fn encode_counts(counts: ResourceManifestCounts) -> Result<Vec<u8>, ManifestEncodingError> {
    record_value(
        "counts",
        vec![
            field("node-class-count", u64_value(counts.node_class_count)?)?,
            field(
                "quotient-relation-record-count",
                u64_value(counts.quotient_relation_record_count)?,
            )?,
            field(
                "relation-occurrence-count",
                u64_value(counts.relation_occurrence_count)?,
            )?,
            field(
                "available-relation-record-count",
                u64_value(counts.available_relation_record_count)?,
            )?,
            field(
                "available-relation-occurrence-count",
                u64_value(counts.available_relation_occurrence_count)?,
            )?,
            field(
                "unavailable-relation-record-count",
                u64_value(counts.unavailable_relation_record_count)?,
            )?,
            field(
                "unavailable-relation-occurrence-count",
                u64_value(counts.unavailable_relation_occurrence_count)?,
            )?,
            field("maximum-depth", u64_value(counts.maximum_depth)?)?,
        ],
    )
}

fn encode_replay_status(
    replay_status: &InputReplayStatus,
) -> Result<Vec<u8>, ManifestEncodingError> {
    let (status, reasons) = match replay_status {
        InputReplayStatus::Replayable => ("replayable", Vec::new()),
        InputReplayStatus::ObservedOnly(reasons) => {
            let reasons = reasons
                .iter()
                .map(|reason| enum_value(reason.canonical_id()))
                .collect::<Result<Vec<_>, _>>()?;
            ("observed-only", reasons)
        }
    };
    record_value(
        "input-replay-status",
        vec![
            field("status", enum_value(status)?)?,
            field("reasons", set_value(reasons)?)?,
        ],
    )
}

fn requirement_id(requirement: DependencyRequirement) -> &'static str {
    match requirement {
        DependencyRequirement::Required => "required",
        DependencyRequirement::Optional => "optional",
    }
}

fn session_opaque_reason_id(reason: ManifestSessionOpaqueReason) -> &'static str {
    match reason {
        ManifestSessionOpaqueReason::GeneratedReportIdentity => "generated-report-identity",
        ManifestSessionOpaqueReason::UnregisteredCallerIdentity => "unregistered-caller-identity",
        ManifestSessionOpaqueReason::UnregisteredResolverIdentity => {
            "unregistered-resolver-identity"
        }
    }
}

fn session_dependent_reason_id(reason: ManifestSessionDependentReason) -> &'static str {
    match reason {
        ManifestSessionDependentReason::AcquisitionBudget => "session-dependent-acquisition-budget",
        ManifestSessionDependentReason::MutablePolicy => "session-dependent-mutable-policy",
        ManifestSessionDependentReason::HostCallback => "session-dependent-host-callback",
        ManifestSessionDependentReason::ProcessState => "session-dependent-process-state",
        ManifestSessionDependentReason::UnregisteredSemanticBasis => "unregistered-semantic-basis",
    }
}

fn validate_canonical_id(value: &str) -> Result<(), ManifestEncodingError> {
    if value.is_empty()
        || value.len() > 96
        || value.split(['.', '_', ':', '-']).any(|part| {
            part.is_empty()
                || part
                    .bytes()
                    .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit()))
        })
    {
        return Err(ManifestEncodingError::InvalidCanonicalId);
    }
    Ok(())
}

fn checked_len(value: usize) -> Result<u64, ManifestEncodingError> {
    u64::try_from(value).map_err(|_| ManifestEncodingError::LengthOverflow)
}

fn frame(label: &[u8], payload: &[u8]) -> Result<Vec<u8>, ManifestEncodingError> {
    let capacity = 16_usize
        .checked_add(label.len())
        .and_then(|value| value.checked_add(payload.len()))
        .ok_or(ManifestEncodingError::LengthOverflow)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(capacity)
        .map_err(|_| ManifestEncodingError::LengthOverflow)?;
    encoded.extend_from_slice(&checked_len(label.len())?.to_be_bytes());
    encoded.extend_from_slice(label);
    encoded.extend_from_slice(&checked_len(payload.len())?.to_be_bytes());
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

fn record(tag: &str, fields: Vec<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    let tag = frame(b"record-tag", tag.as_bytes())?;
    let capacity = tag
        .len()
        .checked_add(8)
        .and_then(|base| {
            fields
                .iter()
                .try_fold(base, |sum, field| sum.checked_add(field.len()))
        })
        .ok_or(ManifestEncodingError::LengthOverflow)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(capacity)
        .map_err(|_| ManifestEncodingError::LengthOverflow)?;
    encoded.extend(tag);
    encoded.extend_from_slice(&checked_len(fields.len())?.to_be_bytes());
    for field in fields {
        encoded.extend(field);
    }
    Ok(encoded)
}

fn value(kind: &str, payload: Vec<u8>) -> Result<Vec<u8>, ManifestEncodingError> {
    record(
        "identity-value",
        vec![
            frame(b"kind", kind.as_bytes())?,
            frame(b"payload", &payload)?,
        ],
    )
}

fn field(label: &str, value: Vec<u8>) -> Result<Vec<u8>, ManifestEncodingError> {
    frame(label.as_bytes(), &value)
}

fn record_value(tag: &str, fields: Vec<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    value("record", record(tag, fields)?)
}

fn u64_value(value_: u64) -> Result<Vec<u8>, ManifestEncodingError> {
    value("u64", value_.to_be_bytes().to_vec())
}

fn bytes_value(bytes: Vec<u8>) -> Result<Vec<u8>, ManifestEncodingError> {
    value("bytes", bytes)
}

fn utf8_value(bytes: &[u8]) -> Result<Vec<u8>, ManifestEncodingError> {
    value("utf8", bytes.to_vec())
}

fn enum_value(value_: &str) -> Result<Vec<u8>, ManifestEncodingError> {
    validate_canonical_id(value_)?;
    value("enum", value_.as_bytes().to_vec())
}

fn digest_value(digest: ManifestDigest) -> Result<Vec<u8>, ManifestEncodingError> {
    record_value(
        "digest",
        vec![
            field("algorithm", enum_value("sha256")?)?,
            field("value", bytes_value(digest.0.to_vec())?)?,
        ],
    )
}

fn optional_value(value_: Option<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    match value_ {
        None => value("optional", vec![0]),
        Some(value_) => {
            let mut payload = vec![1];
            payload.extend(frame(b"some", &value_)?);
            value("optional", payload)
        }
    }
}

fn sequence_value(values: Vec<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    let mut payload = checked_len(values.len())?.to_be_bytes().to_vec();
    for value_ in values {
        payload.extend(frame(b"item", &value_)?);
    }
    value("sequence", payload)
}

fn set_value(mut values: Vec<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ManifestEncodingError::InvalidCanonicalId);
    }
    let mut payload = checked_len(values.len())?.to_be_bytes().to_vec();
    for value_ in values {
        payload.extend(frame(b"item", &value_)?);
    }
    value("set", payload)
}

fn union_value(tag: &str, fields: Vec<Vec<u8>>) -> Result<Vec<u8>, ManifestEncodingError> {
    value("union", record(tag, fields)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_deny_parameter_record_matches_the_adr_vector() {
        let encoded = record_value("external-policy-deny-v1", Vec::new()).unwrap();
        assert_eq!(encoded.len(), 154);
        assert_eq!(
            ManifestDigest::of(&encoded).as_bytes(),
            [
                0x32, 0x5d, 0x32, 0xa5, 0x71, 0xf7, 0x30, 0x88, 0x38, 0xf5, 0x88, 0x79, 0xc1, 0x7d,
                0xc5, 0xdb, 0xdb, 0x78, 0x4e, 0xb9, 0x1f, 0x23, 0x03, 0x11, 0x16, 0x02, 0xbe, 0x8b,
                0x18, 0xf5, 0x59, 0xc9,
            ]
        );
    }

    #[test]
    fn canonical_ids_reject_non_registry_grammar() {
        assert_eq!(
            enum_value("Debug Value"),
            Err(ManifestEncodingError::InvalidCanonicalId)
        );
    }
}
