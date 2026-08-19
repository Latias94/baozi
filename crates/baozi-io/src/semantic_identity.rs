use crate::{AssetIo, AssetPath, AuthorityDomain, PathError};
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

/// One registered negative-proof scheme.
///
/// The initial same-authority resolver registers no proof scheme. This opaque type reserves the
/// typed registry boundary without allowing callers to construct or self-register schemes.
pub struct RegisteredNegativeProofScheme {
    _private: (),
}

impl fmt::Debug for RegisteredNegativeProofScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RegisteredNegativeProofScheme")
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

static NO_NEGATIVE_PROOF_SCHEMES: &[RegisteredNegativeProofScheme] = &[];

static SAME_AUTHORITY_RESOLVER_ENTRY: ResolverEntry = ResolverEntry {
    subject: ResolverSemanticSubject {
        contract: ResolverContractId("baozi.same-authority-resolver"),
        contract_version: V1,
        namespace_semantics: PORTABLE_LOGICAL_NAMESPACE,
        normalizer: &PORTABLE_ASSET_PATH_NORMALIZER,
        parameters: SemanticParameterSchema::SameAuthorityResolverV1,
        proof_schemes: NO_NEGATIVE_PROOF_SCHEMES,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryAssetIo;

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
        let resolver = same_authority_resolver_v1();
        assert!(std::ptr::eq(resolver, same_authority_resolver_v1()));
        let subject = resolver.semantic_subject();
        assert!(std::ptr::eq(
            subject.normalizer(),
            portable_asset_path_normalizer_v1()
        ));
        assert!(subject.proof_schemes().is_empty());

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
    }
}
