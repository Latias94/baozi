use crate::ImmutableMemoryAssetIo;
use crate::{AssetIo, AssetPath, AuthorityDomain, PathError};
use sha2::{Digest, Sha256};
use std::any::Any;
use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;

/// A registered resolver contract identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolverContractId(&'static str);

impl ResolverContractId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for ResolverContractId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ResolverContractId")
            .field(&self.0)
            .finish()
    }
}

/// A registered reference-normalizer contract identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceNormalizerContractId(&'static str);

impl ReferenceNormalizerContractId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for ReferenceNormalizerContractId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ReferenceNormalizerContractId")
            .field(&self.0)
            .finish()
    }
}

/// A registered resolver namespace-semantics identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamespaceSemanticsId(&'static str);

impl NamespaceSemanticsId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// A registered negative-proof scheme identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NegativeProofSchemeId(&'static str);

impl NegativeProofSchemeId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for NegativeProofSchemeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NegativeProofSchemeId")
            .field(&self.0)
            .finish()
    }
}

/// A registered managed validity-rule identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NegativeProofValidityRuleId(&'static str);

impl NegativeProofValidityRuleId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for NegativeProofValidityRuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NegativeProofValidityRuleId")
            .field(&self.0)
            .finish()
    }
}

/// The subject bound by one admitted negative proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NegativeProofSubjectKind {
    /// The proof is valid against one complete immutable namespace snapshot.
    NamespaceSnapshot,
    /// The proof is valid for one independently immutable reference assertion.
    PerReference,
}

impl NegativeProofSubjectKind {
    /// Returns the canonical registry identifier.
    pub const fn canonical_id(self) -> &'static str {
        match self {
            Self::NamespaceSnapshot => "namespace-snapshot",
            Self::PerReference => "per-reference",
        }
    }
}

/// The digest algorithm fixed by one proof payload schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NegativeProofDigestAlgorithm {
    Sha256,
}

impl NegativeProofDigestAlgorithm {
    /// Returns the canonical registry identifier.
    pub const fn canonical_id(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }
}

/// Hard per-proof resource caps owned by the registered scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NegativeProofResourceCaps {
    canonical_bytes: u64,
    temporary_bytes: u64,
    construction_work: u64,
    verification_work: u64,
}

impl NegativeProofResourceCaps {
    pub const fn canonical_bytes(self) -> u64 {
        self.canonical_bytes
    }

    pub const fn temporary_bytes(self) -> u64 {
        self.temporary_bytes
    }

    pub const fn construction_work(self) -> u64 {
        self.construction_work
    }

    pub const fn verification_work(self) -> u64 {
        self.verification_work
    }
}

/// The complete typed semantic subject for one registered negative-proof scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NegativeProofSchemeSemanticSubject {
    scheme: NegativeProofSchemeId,
    version: NonZeroU32,
    validity_rule: NegativeProofValidityRuleId,
    subject_kind: NegativeProofSubjectKind,
    payload_schema_id: &'static str,
    payload_schema_version: NonZeroU32,
    digest_algorithm: NegativeProofDigestAlgorithm,
    caps: NegativeProofResourceCaps,
}

impl NegativeProofSchemeSemanticSubject {
    pub const fn scheme(self) -> NegativeProofSchemeId {
        self.scheme
    }

    pub const fn version(self) -> NonZeroU32 {
        self.version
    }

    pub const fn validity_rule(self) -> NegativeProofValidityRuleId {
        self.validity_rule
    }

    pub const fn subject_kind(self) -> NegativeProofSubjectKind {
        self.subject_kind
    }

    pub const fn payload_schema_id(self) -> &'static str {
        self.payload_schema_id
    }

    pub const fn payload_schema_version(self) -> NonZeroU32 {
        self.payload_schema_version
    }

    pub const fn digest_algorithm(self) -> NegativeProofDigestAlgorithm {
        self.digest_algorithm
    }

    pub const fn caps(self) -> NegativeProofResourceCaps {
        self.caps
    }
}

impl fmt::Debug for NamespaceSemanticsId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NamespaceSemanticsId")
            .field(&self.0)
            .finish()
    }
}

/// A registered reference canonicalization-profile identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalizationProfileId(&'static str);

impl CanonicalizationProfileId {
    /// Returns the canonical registry identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for CanonicalizationProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CanonicalizationProfileId")
            .field(&self.0)
            .finish()
    }
}

/// A closed semantic-parameter schema admitted by the asset-IO registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SemanticParameterSchema {
    /// The v1 portable asset-path normalizer has no configurable parameters.
    PortableAssetPathNormalizerV1,
    /// The v1 same-authority resolver has no configurable parameters.
    SameAuthorityResolverV1,
}

impl SemanticParameterSchema {
    /// Returns the canonical schema identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::PortableAssetPathNormalizerV1 => "baozi.asset-path.normalizer-parameters.none",
            Self::SameAuthorityResolverV1 => "baozi.same-authority-resolver.parameters.none",
        }
    }

    /// Returns the schema version.
    pub const fn version(self) -> NonZeroU32 {
        NonZeroU32::MIN
    }

    /// Returns the exact zero-field parameter-record tag.
    pub const fn record_tag(self) -> &'static str {
        match self {
            Self::PortableAssetPathNormalizerV1 => "asset-path-normalizer-parameters-v1",
            Self::SameAuthorityResolverV1 => "same-authority-resolver-parameters-v1",
        }
    }
}

#[derive(Debug)]
struct NegativeProofSchemeEntry {
    subject: NegativeProofSchemeSemanticSubject,
}

/// One registered negative-proof scheme.
///
/// Construction remains private so custom resolvers cannot self-admit a proof scheme.
pub struct RegisteredNegativeProofScheme {
    entry: &'static NegativeProofSchemeEntry,
}

impl RegisteredNegativeProofScheme {
    /// Returns the complete registered semantic subject.
    pub const fn semantic_subject(&self) -> NegativeProofSchemeSemanticSubject {
        self.entry.subject
    }
}

impl fmt::Debug for RegisteredNegativeProofScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredNegativeProofScheme")
            .field("subject", &self.entry.subject)
            .finish()
    }
}

/// The typed semantic subject for one registered reference normalizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceNormalizerSemanticSubject {
    contract: ReferenceNormalizerContractId,
    contract_version: NonZeroU32,
    namespace_semantics: NamespaceSemanticsId,
    canonicalization_profile: CanonicalizationProfileId,
    parameters: SemanticParameterSchema,
}

impl ReferenceNormalizerSemanticSubject {
    /// Returns the registered normalizer contract.
    pub const fn contract(self) -> ReferenceNormalizerContractId {
        self.contract
    }

    /// Returns the registered normalizer contract version.
    pub const fn contract_version(self) -> NonZeroU32 {
        self.contract_version
    }

    /// Returns the registered namespace semantics.
    pub const fn namespace_semantics(self) -> NamespaceSemanticsId {
        self.namespace_semantics
    }

    /// Returns the registered canonicalization profile.
    pub const fn canonicalization_profile(self) -> CanonicalizationProfileId {
        self.canonicalization_profile
    }

    /// Returns the closed semantic-parameter schema.
    pub const fn parameters(self) -> SemanticParameterSchema {
        self.parameters
    }
}

/// The typed semantic subject for one registered resolver.
#[derive(Debug, Clone, Copy)]
pub struct ResolverSemanticSubject {
    contract: ResolverContractId,
    contract_version: NonZeroU32,
    namespace_semantics: NamespaceSemanticsId,
    normalizer: &'static RegisteredReferenceNormalizer,
    parameters: SemanticParameterSchema,
    proof_schemes: &'static [RegisteredNegativeProofScheme],
}

impl ResolverSemanticSubject {
    /// Returns the registered resolver contract.
    pub const fn contract(self) -> ResolverContractId {
        self.contract
    }

    /// Returns the registered resolver contract version.
    pub const fn contract_version(self) -> NonZeroU32 {
        self.contract_version
    }

    /// Returns the registered namespace semantics.
    pub const fn namespace_semantics(self) -> NamespaceSemanticsId {
        self.namespace_semantics
    }

    /// Returns the exact registered normalizer used by this resolver.
    pub const fn normalizer(self) -> &'static RegisteredReferenceNormalizer {
        self.normalizer
    }

    /// Returns the closed semantic-parameter schema.
    pub const fn parameters(self) -> SemanticParameterSchema {
        self.parameters
    }

    /// Returns the sorted admitted negative-proof schemes.
    pub const fn proof_schemes(self) -> &'static [RegisteredNegativeProofScheme] {
        self.proof_schemes
    }
}

#[derive(Debug)]
struct ReferenceNormalizerEntry {
    subject: ReferenceNormalizerSemanticSubject,
}

/// One sealed reference normalizer from the static asset-IO registry.
pub struct RegisteredReferenceNormalizer {
    entry: &'static ReferenceNormalizerEntry,
}

impl RegisteredReferenceNormalizer {
    /// Returns the complete registered semantic subject.
    pub const fn semantic_subject(&self) -> ReferenceNormalizerSemanticSubject {
        self.entry.subject
    }

    /// Resolves a hostile relative reference against the declaring file identity.
    pub fn normalize_from_file(
        &'static self,
        origin: &AssetPath,
        reference: &str,
    ) -> Result<NormalizedReference, PathError> {
        let path = origin.resolve_from_file(reference)?;
        Ok(NormalizedReference {
            path,
            normalizer: self,
        })
    }
}

impl fmt::Debug for RegisteredReferenceNormalizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredReferenceNormalizer")
            .field("subject", &self.entry.subject)
            .finish()
    }
}

/// One normalized reference issued by a registered normalizer.
pub struct NormalizedReference {
    path: AssetPath,
    normalizer: &'static RegisteredReferenceNormalizer,
}

impl NormalizedReference {
    /// Returns the exact normalized logical path.
    pub const fn path(&self) -> &AssetPath {
        &self.path
    }

    /// Returns the registry entry that issued this value.
    pub const fn normalizer(&self) -> &'static RegisteredReferenceNormalizer {
        self.normalizer
    }

    /// Consumes the evidence and returns its normalized path.
    pub fn into_path(self) -> AssetPath {
        self.path
    }
}

impl fmt::Debug for NormalizedReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedReference")
            .field("path", &self.path)
            .field("normalizer", &self.normalizer)
            .finish()
    }
}

#[derive(Debug)]
struct StableResourceIdentitySchemaEntry {
    owner: &'static str,
    schema_id: &'static str,
    schema_version: NonZeroU32,
}

/// One sealed stable-resource-identity schema from the ADR 0036 registry.
pub struct StableResourceIdentitySchema {
    entry: &'static StableResourceIdentitySchemaEntry,
}

impl StableResourceIdentitySchema {
    /// Returns the schema owner.
    pub const fn owner(&self) -> &'static str {
        self.entry.owner
    }

    /// Returns the schema identifier.
    pub const fn schema_id(&self) -> &'static str {
        self.entry.schema_id
    }

    /// Returns the schema version.
    pub const fn schema_version(&self) -> NonZeroU32 {
        self.entry.schema_version
    }
}

impl fmt::Debug for StableResourceIdentitySchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableResourceIdentitySchema")
            .field("owner", &self.entry.owner)
            .field("schema_id", &self.entry.schema_id)
            .field("schema_version", &self.entry.schema_version)
            .finish()
    }
}

/// A stable public resource identity issued by one registered resolver.
pub struct StableResourceIdentity {
    value: AssetPath,
    schema: &'static StableResourceIdentitySchema,
}

impl StableResourceIdentity {
    /// Returns the exact public identity value.
    pub const fn value(&self) -> &AssetPath {
        &self.value
    }

    /// Returns the sealed stability schema.
    pub const fn schema(&self) -> &'static StableResourceIdentitySchema {
        self.schema
    }

    /// Consumes the evidence and returns its exact public identity value.
    pub fn into_value(self) -> AssetPath {
        self.value
    }
}

impl fmt::Debug for StableResourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableResourceIdentity")
            .field("value", &self.value)
            .field("schema", &self.schema)
            .finish()
    }
}

#[derive(Debug)]
struct ResolverEntry {
    subject: ResolverSemanticSubject,
    identity_schema: &'static StableResourceIdentitySchema,
}

/// One sealed resolver profile from the static asset-IO registry.
pub struct RegisteredResolverProfile {
    entry: &'static ResolverEntry,
}

impl RegisteredResolverProfile {
    /// Returns the complete registered semantic subject.
    pub const fn semantic_subject(&self) -> ResolverSemanticSubject {
        self.entry.subject
    }

    /// Returns the stable public resource-identity schema issued by this resolver.
    pub const fn identity_schema(&self) -> &'static StableResourceIdentitySchema {
        self.entry.identity_schema
    }

    /// Executes the v1 same-authority resolution after registered normalization.
    pub fn resolve_same_authority<'io>(
        &'static self,
        io: &'io dyn AssetIo,
        authority: AuthorityDomain,
        reference: NormalizedReference,
    ) -> Result<RegisteredAssetResolution<'io>, RegisteredResolutionError> {
        if !std::ptr::eq(reference.normalizer, self.entry.subject.normalizer) {
            return Err(RegisteredResolutionError::NormalizerMismatch);
        }
        Ok(RegisteredAssetResolution {
            io,
            authority,
            identity: StableResourceIdentity {
                value: reference.path,
                schema: self.entry.identity_schema,
            },
            resolver: self,
        })
    }

    /// Plans a registered immutable negative proof without executing proof work.
    ///
    /// The exact concrete backend check is owned here. A custom `AssetIo` implementation cannot
    /// opt itself into this path by forwarding a trait method or returning a registry identifier.
    pub fn plan_negative_proof<'target>(
        &'static self,
        io: &'target dyn AssetIo,
        target: &'target AssetPath,
        cause: RegisteredUnavailableCause,
    ) -> Option<RegisteredNegativeProofPlan<'target>> {
        if !std::ptr::eq(self, &SAME_AUTHORITY_RESOLVER)
            || cause != RegisteredUnavailableCause::Missing
        {
            return None;
        }
        let backend = (io as &dyn Any).downcast_ref::<ImmutableMemoryAssetIo>()?;
        Some(RegisteredNegativeProofPlan {
            backend,
            target,
            resolver: self,
            scheme: immutable_memory_namespace_absence_v1(),
            cause,
        })
    }
}

impl fmt::Debug for RegisteredResolverProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredResolverProfile")
            .field("subject", &self.entry.subject)
            .field("identity_schema", &self.entry.identity_schema)
            .finish()
    }
}

/// One registered resolver result bound to the operational authority used for open.
pub struct RegisteredAssetResolution<'io> {
    io: &'io dyn AssetIo,
    authority: AuthorityDomain,
    identity: StableResourceIdentity,
    resolver: &'static RegisteredResolverProfile,
}

impl<'io> RegisteredAssetResolution<'io> {
    /// Returns the backend that must open the resolved identity.
    pub fn io(&self) -> &'io dyn AssetIo {
        self.io
    }

    /// Returns the private authority-domain identity used only for cache isolation.
    pub const fn authority(&self) -> &AuthorityDomain {
        &self.authority
    }

    /// Returns the stable public identity evidence.
    pub const fn identity(&self) -> &StableResourceIdentity {
        &self.identity
    }

    /// Returns the registry entry that issued this resolution.
    pub const fn resolver(&self) -> &'static RegisteredResolverProfile {
        self.resolver
    }

    /// Consumes the result into its operational and public-identity parts.
    pub fn into_parts(self) -> (&'io dyn AssetIo, AuthorityDomain, StableResourceIdentity) {
        (self.io, self.authority, self.identity)
    }
}

impl fmt::Debug for RegisteredAssetResolution<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredAssetResolution")
            .field("authority", &self.authority)
            .field("identity", &self.identity)
            .field("resolver", &self.resolver)
            .finish_non_exhaustive()
    }
}

/// Exact unavailable cause bound by a registered negative proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RegisteredUnavailableCause {
    Missing,
    OpenFailed,
    ReadFailed,
}

impl RegisteredUnavailableCause {
    pub const fn class_id(self) -> &'static str {
        "baozi.io"
    }

    pub const fn code_id(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::OpenFailed => "open-failed",
            Self::ReadFailed => "read-failed",
        }
    }
}

/// One allocation-free registered proof plan bound to an exact backend and target.
pub struct RegisteredNegativeProofPlan<'target> {
    backend: &'target ImmutableMemoryAssetIo,
    target: &'target AssetPath,
    resolver: &'static RegisteredResolverProfile,
    scheme: &'static RegisteredNegativeProofScheme,
    cause: RegisteredUnavailableCause,
}

impl<'target> RegisteredNegativeProofPlan<'target> {
    pub const fn scheme(&self) -> &'static RegisteredNegativeProofScheme {
        self.scheme
    }

    pub const fn resolver(&self) -> &'static RegisteredResolverProfile {
        self.resolver
    }

    pub const fn target(&self) -> &AssetPath {
        self.target
    }

    pub const fn cause(&self) -> RegisteredUnavailableCause {
        self.cause
    }

    /// Verifies the exact target and cause after the caller has admitted the scheme's work caps.
    pub fn verify(
        self,
        target: &AssetPath,
        cause: RegisteredUnavailableCause,
    ) -> Result<VerifiedNegativeInputProofBinding<'target>, RegisteredNegativeProofError> {
        if target != self.target {
            return Err(RegisteredNegativeProofError::TargetMismatch);
        }
        if cause != self.cause {
            return Err(RegisteredNegativeProofError::CauseMismatch);
        }
        if self.backend.contains(target) {
            return Err(RegisteredNegativeProofError::TargetAvailable);
        }
        let namespace_snapshot_digest = self.backend.namespace_snapshot_digest();
        Ok(VerifiedNegativeInputProofBinding {
            backend: self.backend,
            target: self.target,
            proof: VerifiedNegativeInputProof {
                resolver: self.resolver,
                scheme: self.scheme,
                namespace_snapshot_digest,
                cause,
                proof_digest: immutable_memory_absence_proof_digest(
                    self.resolver,
                    self.scheme,
                    namespace_snapshot_digest,
                    target,
                    cause,
                ),
            },
        })
    }
}

impl fmt::Debug for RegisteredNegativeProofPlan<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredNegativeProofPlan")
            .field("target", &self.target)
            .field("resolver", &self.resolver)
            .field("scheme", &self.scheme)
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

/// One verified inert negative proof issued by the sealed asset-IO registry.
#[derive(Debug, Clone, Copy)]
pub struct VerifiedNegativeInputProof {
    resolver: &'static RegisteredResolverProfile,
    scheme: &'static RegisteredNegativeProofScheme,
    namespace_snapshot_digest: [u8; 32],
    cause: RegisteredUnavailableCause,
    proof_digest: [u8; 32],
}

impl PartialEq for VerifiedNegativeInputProof {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.resolver, other.resolver)
            && std::ptr::eq(self.scheme, other.scheme)
            && self.namespace_snapshot_digest == other.namespace_snapshot_digest
            && self.cause == other.cause
            && self.proof_digest == other.proof_digest
    }
}

impl Eq for VerifiedNegativeInputProof {}

/// A verified proof still bound to the exact target object used by its proof plan.
///
/// This linear wrapper crosses the import work callback, then is consumed at the cache transition.
/// The retained proof cannot be extracted until the cache supplies the same authority, target
/// object, cause, and registered identity schema. That makes publication an allocation-free O(1)
/// check instead of an unmetered second proof hash.
pub struct VerifiedNegativeInputProofBinding<'target> {
    backend: &'target ImmutableMemoryAssetIo,
    target: &'target AssetPath,
    proof: VerifiedNegativeInputProof,
}

impl VerifiedNegativeInputProofBinding<'_> {
    /// Consumes the binding only when the cache presents the exact planned subject.
    pub fn seal_subject(
        self,
        identity_schema: &'static StableResourceIdentitySchema,
        authority: &AuthorityDomain,
        target: &AssetPath,
        cause: RegisteredUnavailableCause,
    ) -> Result<VerifiedNegativeInputProof, RegisteredNegativeProofError> {
        if self.backend.authority() != authority {
            return Err(RegisteredNegativeProofError::AuthorityMismatch);
        }
        if !std::ptr::eq(self.target, target) {
            return Err(RegisteredNegativeProofError::TargetMismatch);
        }
        if !std::ptr::eq(identity_schema, self.proof.resolver.identity_schema()) {
            return Err(RegisteredNegativeProofError::IdentitySchemaMismatch);
        }
        if cause != self.proof.cause {
            return Err(RegisteredNegativeProofError::CauseMismatch);
        }
        Ok(self.proof)
    }
}

impl fmt::Debug for VerifiedNegativeInputProofBinding<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedNegativeInputProofBinding")
            .field("authority", self.backend.authority())
            .field("target", &self.target)
            .field("proof", &self.proof)
            .finish_non_exhaustive()
    }
}

impl VerifiedNegativeInputProof {
    pub const fn resolver(self) -> &'static RegisteredResolverProfile {
        self.resolver
    }

    pub const fn scheme(self) -> &'static RegisteredNegativeProofScheme {
        self.scheme
    }

    pub const fn namespace_snapshot_digest(self) -> [u8; 32] {
        self.namespace_snapshot_digest
    }

    pub const fn cause(self) -> RegisteredUnavailableCause {
        self.cause
    }

    pub const fn proof_digest(self) -> [u8; 32] {
        self.proof_digest
    }
}

/// A registered proof plan did not validate the exact cached failure subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegisteredNegativeProofError {
    AuthorityMismatch,
    TargetMismatch,
    CauseMismatch,
    IdentitySchemaMismatch,
    TargetAvailable,
}

impl fmt::Display for RegisteredNegativeProofError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorityMismatch => {
                formatter.write_str("negative proof authority does not match the issuing namespace")
            }
            Self::TargetMismatch => {
                formatter.write_str("negative proof target does not match the planned target")
            }
            Self::CauseMismatch => {
                formatter.write_str("negative proof cause does not match the planned cause")
            }
            Self::IdentitySchemaMismatch => formatter
                .write_str("negative proof identity schema does not match the issuing resolver"),
            Self::TargetAvailable => formatter
                .write_str("negative proof namespace contains the allegedly missing target"),
        }
    }
}

impl Error for RegisteredNegativeProofError {}

/// A registered resolver rejected evidence from another normalizer profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegisteredResolutionError {
    /// The normalized reference was not issued by the resolver's registered normalizer.
    NormalizerMismatch,
}

impl fmt::Display for RegisteredResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NormalizerMismatch => {
                formatter.write_str("registered resolver received a foreign normalizer result")
            }
        }
    }
}

impl Error for RegisteredResolutionError {}

const V1: NonZeroU32 = NonZeroU32::MIN;
const PORTABLE_LOGICAL_NAMESPACE: NamespaceSemanticsId =
    NamespaceSemanticsId("baozi.asset-path.portable-logical");
const IMMUTABLE_MEMORY_ABSENCE_PROOF_DOMAIN: &[u8] =
    b"baozi.immutable-memory-namespace-absence-proof.v1";

static IMMUTABLE_MEMORY_ABSENCE_SCHEME_ENTRY: NegativeProofSchemeEntry = NegativeProofSchemeEntry {
    subject: NegativeProofSchemeSemanticSubject {
        scheme: NegativeProofSchemeId("baozi.immutable-memory-namespace-absence"),
        version: V1,
        validity_rule: NegativeProofValidityRuleId(
            "baozi.immutable-memory-namespace-membership-v1",
        ),
        subject_kind: NegativeProofSubjectKind::NamespaceSnapshot,
        payload_schema_id: "baozi.immutable-memory-namespace-absence-proof",
        payload_schema_version: V1,
        digest_algorithm: NegativeProofDigestAlgorithm::Sha256,
        caps: NegativeProofResourceCaps {
            canonical_bytes: 16 * 1024,
            temporary_bytes: 0,
            construction_work: 1,
            verification_work: 1,
        },
    },
};

// Own the wrappers in named static storage so const and runtime access share pointer identity.
static SAME_AUTHORITY_NEGATIVE_PROOF_SCHEMES: [RegisteredNegativeProofScheme; 1] =
    [RegisteredNegativeProofScheme {
        entry: &IMMUTABLE_MEMORY_ABSENCE_SCHEME_ENTRY,
    }];

static PORTABLE_ASSET_PATH_NORMALIZER_ENTRY: ReferenceNormalizerEntry = ReferenceNormalizerEntry {
    subject: ReferenceNormalizerSemanticSubject {
        contract: ReferenceNormalizerContractId("baozi.asset-path.resolve-from-file"),
        contract_version: V1,
        namespace_semantics: PORTABLE_LOGICAL_NAMESPACE,
        canonicalization_profile: CanonicalizationProfileId("baozi.asset-path.portable-v1"),
        parameters: SemanticParameterSchema::PortableAssetPathNormalizerV1,
    },
};

static PORTABLE_ASSET_PATH_NORMALIZER: RegisteredReferenceNormalizer =
    RegisteredReferenceNormalizer {
        entry: &PORTABLE_ASSET_PATH_NORMALIZER_ENTRY,
    };

static SAME_AUTHORITY_RESOURCE_IDENTITY_SCHEMA_ENTRY: StableResourceIdentitySchemaEntry =
    StableResourceIdentitySchemaEntry {
        owner: "baozi.asset-io",
        schema_id: "same-authority-asset-path",
        schema_version: V1,
    };

static SAME_AUTHORITY_RESOURCE_IDENTITY_SCHEMA: StableResourceIdentitySchema =
    StableResourceIdentitySchema {
        entry: &SAME_AUTHORITY_RESOURCE_IDENTITY_SCHEMA_ENTRY,
    };

static SAME_AUTHORITY_RESOLVER_ENTRY: ResolverEntry = ResolverEntry {
    subject: ResolverSemanticSubject {
        contract: ResolverContractId("baozi.same-authority-resolver"),
        contract_version: V1,
        namespace_semantics: PORTABLE_LOGICAL_NAMESPACE,
        normalizer: &PORTABLE_ASSET_PATH_NORMALIZER,
        parameters: SemanticParameterSchema::SameAuthorityResolverV1,
        proof_schemes: &SAME_AUTHORITY_NEGATIVE_PROOF_SCHEMES,
    },
    identity_schema: &SAME_AUTHORITY_RESOURCE_IDENTITY_SCHEMA,
};

static SAME_AUTHORITY_RESOLVER: RegisteredResolverProfile = RegisteredResolverProfile {
    entry: &SAME_AUTHORITY_RESOLVER_ENTRY,
};

/// Returns the v1 portable asset-path reference normalizer.
pub const fn portable_asset_path_normalizer_v1() -> &'static RegisteredReferenceNormalizer {
    &PORTABLE_ASSET_PATH_NORMALIZER
}

/// Returns the v1 same-authority resolver profile.
pub const fn same_authority_resolver_v1() -> &'static RegisteredResolverProfile {
    &SAME_AUTHORITY_RESOLVER
}

/// Returns the v1 immutable-memory namespace absence proof scheme.
pub const fn immutable_memory_namespace_absence_v1() -> &'static RegisteredNegativeProofScheme {
    &SAME_AUTHORITY_NEGATIVE_PROOF_SCHEMES[0]
}

fn immutable_memory_absence_proof_digest(
    resolver: &RegisteredResolverProfile,
    scheme: &RegisteredNegativeProofScheme,
    namespace_snapshot_digest: [u8; 32],
    target: &AssetPath,
    cause: RegisteredUnavailableCause,
) -> [u8; 32] {
    let identity_schema = resolver.identity_schema();
    let resolver = resolver.semantic_subject();
    let scheme = scheme.semantic_subject();
    let mut digest = Sha256::new();
    update_proof_frame(&mut digest, IMMUTABLE_MEMORY_ABSENCE_PROOF_DOMAIN);
    update_proof_frame(&mut digest, resolver.contract().as_str().as_bytes());
    digest.update(u64::from(resolver.contract_version().get()).to_be_bytes());
    update_proof_frame(&mut digest, scheme.scheme().as_str().as_bytes());
    digest.update(u64::from(scheme.version().get()).to_be_bytes());
    update_proof_frame(&mut digest, scheme.validity_rule().as_str().as_bytes());
    update_proof_frame(&mut digest, identity_schema.owner().as_bytes());
    update_proof_frame(&mut digest, identity_schema.schema_id().as_bytes());
    digest.update(u64::from(identity_schema.schema_version().get()).to_be_bytes());
    digest.update(namespace_snapshot_digest);
    update_proof_frame(&mut digest, target.as_str().as_bytes());
    update_proof_frame(&mut digest, cause.class_id().as_bytes());
    update_proof_frame(&mut digest, cause.code_id().as_bytes());
    digest.finalize().into()
}

fn update_proof_frame(digest: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).expect("usize fits in u64 on supported Rust targets");
    digest.update(length.to_be_bytes());
    digest.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImmutableMemoryAssetIo, MemoryAssetIo};
    use std::sync::Arc;

    #[test]
    fn portable_normalizer_preserves_exact_unicode_and_registered_semantics() {
        let normalizer = portable_asset_path_normalizer_v1();
        assert!(std::ptr::eq(
            normalizer,
            portable_asset_path_normalizer_v1()
        ));
        let origin = AssetPath::new("models/scene.obj").unwrap();
        let nfc = normalizer
            .normalize_from_file(&origin, "textures/caf\u{e9}.png")
            .unwrap();
        let nfd = normalizer
            .normalize_from_file(&origin, "textures/cafe\u{301}.png")
            .unwrap();
        assert_eq!(nfc.path().as_str(), "models/textures/caf\u{e9}.png");
        assert_eq!(nfd.path().as_str(), "models/textures/cafe\u{301}.png");
        assert_ne!(nfc.path(), nfd.path());

        let subject = normalizer.semantic_subject();
        assert_eq!(
            subject.contract().as_str(),
            "baozi.asset-path.resolve-from-file"
        );
        assert_eq!(subject.contract_version(), V1);
        assert_eq!(
            subject.namespace_semantics().as_str(),
            "baozi.asset-path.portable-logical"
        );
        assert_eq!(
            subject.canonicalization_profile().as_str(),
            "baozi.asset-path.portable-v1"
        );
        assert_eq!(
            subject.parameters(),
            SemanticParameterSchema::PortableAssetPathNormalizerV1
        );
    }

    #[test]
    fn same_authority_profile_issues_the_registered_stable_identity() {
        const CONST_SCHEME: &RegisteredNegativeProofScheme =
            immutable_memory_namespace_absence_v1();
        const CONST_PROFILE_SCHEME: &RegisteredNegativeProofScheme = &same_authority_resolver_v1()
            .semantic_subject()
            .proof_schemes()[0];
        static STATIC_SCHEME: &RegisteredNegativeProofScheme =
            immutable_memory_namespace_absence_v1();

        let resolver = same_authority_resolver_v1();
        assert!(std::ptr::eq(resolver, same_authority_resolver_v1()));
        let subject = resolver.semantic_subject();
        assert!(std::ptr::eq(
            subject.normalizer(),
            portable_asset_path_normalizer_v1()
        ));
        assert_eq!(subject.proof_schemes().len(), 1);
        for scheme in [
            CONST_SCHEME,
            CONST_PROFILE_SCHEME,
            STATIC_SCHEME,
            immutable_memory_namespace_absence_v1(),
        ] {
            assert!(std::ptr::eq(&subject.proof_schemes()[0], scheme));
        }

        let io = MemoryAssetIo::new();
        let authority = io.authority_domain();
        let origin = AssetPath::new("models/scene.obj").unwrap();
        let reference = subject
            .normalizer()
            .normalize_from_file(&origin, "textures/albedo.png")
            .unwrap();
        let resolution = resolver
            .resolve_same_authority(&io, authority.clone(), reference)
            .unwrap();

        assert!(std::ptr::eq(resolution.io(), &io));
        assert_eq!(resolution.authority(), &authority);
        assert_eq!(
            resolution.identity().value().as_str(),
            "models/textures/albedo.png"
        );
        assert_eq!(resolution.identity().schema().owner(), "baozi.asset-io");
        assert_eq!(
            resolution.identity().schema().schema_id(),
            "same-authority-asset-path"
        );
        assert_eq!(resolution.identity().schema().schema_version(), V1);
        assert!(
            resolver
                .plan_negative_proof(
                    &io,
                    &AssetPath::new("models/textures/missing.png").unwrap(),
                    RegisteredUnavailableCause::Missing,
                )
                .is_none(),
            "the mutable memory namespace cannot issue immutable absence proof"
        );
    }

    #[test]
    fn immutable_memory_absence_proof_is_exact_and_namespace_bound() {
        assert_eq!(RegisteredUnavailableCause::Missing.class_id(), "baozi.io");
        assert_eq!(RegisteredUnavailableCause::Missing.code_id(), "missing");
        let first = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("models/scene.obj").unwrap(),
            Arc::from(&b"primary"[..]),
        )])
        .unwrap();
        let second = ImmutableMemoryAssetIo::try_from_entries(vec![
            (
                AssetPath::new("models/scene.obj").unwrap(),
                Arc::from(&b"primary"[..]),
            ),
            (
                AssetPath::new("models/other.bin").unwrap(),
                Arc::from(&b"other"[..]),
            ),
        ])
        .unwrap();
        let target = AssetPath::new("models/missing.bin").unwrap();
        let wrong = AssetPath::new("models/other-missing.bin").unwrap();
        let resolver = same_authority_resolver_v1();

        let wrong_target = resolver
            .plan_negative_proof(&first, &target, RegisteredUnavailableCause::Missing)
            .expect("immutable namespace supports the registered scheme")
            .verify(&wrong, RegisteredUnavailableCause::Missing);
        assert!(matches!(
            wrong_target,
            Err(RegisteredNegativeProofError::TargetMismatch)
        ));

        let wrong_cause = resolver
            .plan_negative_proof(&first, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::OpenFailed);
        assert!(matches!(
            wrong_cause,
            Err(RegisteredNegativeProofError::CauseMismatch)
        ));

        let first_proof = resolver
            .plan_negative_proof(&first, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        let repeated = resolver
            .plan_negative_proof(&first, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        let other_namespace = resolver
            .plan_negative_proof(&second, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();

        assert_eq!(first_proof.proof, repeated.proof);
        assert_eq!(
            first_proof.proof.namespace_snapshot_digest(),
            repeated.proof.namespace_snapshot_digest()
        );
        assert_ne!(
            first_proof.proof.namespace_snapshot_digest(),
            other_namespace.proof.namespace_snapshot_digest()
        );
        assert_ne!(first_proof.proof, other_namespace.proof);

        let present = AssetPath::new("models/scene.obj").unwrap();
        let present_result = resolver
            .plan_negative_proof(&first, &present, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&present, RegisteredUnavailableCause::Missing);
        assert!(matches!(
            present_result,
            Err(RegisteredNegativeProofError::TargetAvailable)
        ));
    }

    #[test]
    fn negative_proof_binding_rejects_stale_subjects_and_unadmitted_inputs() {
        static FOREIGN_SCHEMA_ENTRY: StableResourceIdentitySchemaEntry =
            StableResourceIdentitySchemaEntry {
                owner: "dev.baozi.test",
                schema_id: "foreign-resource-identity",
                schema_version: NonZeroU32::MIN,
            };
        static FOREIGN_SCHEMA: StableResourceIdentitySchema = StableResourceIdentitySchema {
            entry: &FOREIGN_SCHEMA_ENTRY,
        };

        let namespace = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("models/scene.obj").unwrap(),
            Arc::from(&b"primary"[..]),
        )])
        .unwrap();
        let foreign_namespace = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("models/other.bin").unwrap(),
            Arc::from(&b"other"[..]),
        )])
        .unwrap();
        let mutable = MemoryAssetIo::new();
        let resolver = same_authority_resolver_v1();
        let target = AssetPath::new("models/missing.bin").unwrap();
        let equal_target = AssetPath::new("models/missing.bin").unwrap();

        assert!(
            resolver
                .plan_negative_proof(&mutable, &target, RegisteredUnavailableCause::Missing)
                .is_none(),
            "mutable or custom backends cannot self-admit the immutable proof scheme"
        );
        for cause in [
            RegisteredUnavailableCause::OpenFailed,
            RegisteredUnavailableCause::ReadFailed,
        ] {
            assert!(
                resolver
                    .plan_negative_proof(&namespace, &target, cause)
                    .is_none(),
                "only missing inputs have a registered immutable absence proof"
            );
        }

        let wrong_target_binding = resolver
            .plan_negative_proof(&namespace, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            wrong_target_binding.seal_subject(
                resolver.identity_schema(),
                namespace.authority(),
                &equal_target,
                RegisteredUnavailableCause::Missing,
            ),
            Err(RegisteredNegativeProofError::TargetMismatch)
        ));

        let wrong_authority_binding = resolver
            .plan_negative_proof(&namespace, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            wrong_authority_binding.seal_subject(
                resolver.identity_schema(),
                foreign_namespace.authority(),
                &target,
                RegisteredUnavailableCause::Missing,
            ),
            Err(RegisteredNegativeProofError::AuthorityMismatch)
        ));

        let wrong_schema_binding = resolver
            .plan_negative_proof(&namespace, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            wrong_schema_binding.seal_subject(
                &FOREIGN_SCHEMA,
                namespace.authority(),
                &target,
                RegisteredUnavailableCause::Missing,
            ),
            Err(RegisteredNegativeProofError::IdentitySchemaMismatch)
        ));

        let wrong_cause_binding = resolver
            .plan_negative_proof(&namespace, &target, RegisteredUnavailableCause::Missing)
            .unwrap()
            .verify(&target, RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            wrong_cause_binding.seal_subject(
                resolver.identity_schema(),
                namespace.authority(),
                &target,
                RegisteredUnavailableCause::OpenFailed,
            ),
            Err(RegisteredNegativeProofError::CauseMismatch)
        ));
    }
}
