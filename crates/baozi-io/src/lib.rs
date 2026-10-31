#![forbid(unsafe_code)]

//! Runtime-neutral, capability-bound asset IO contracts.

mod error;
mod immutable_memory;
mod memory;
mod path;
mod resource;
mod semantic_identity;

pub use error::{
    AssetObjectKind, AssetOpenError, AssetOpenErrorKind, AssetOpenStage,
    AssetReadContractViolation, AssetReadError, AssetReadErrorKind, ImmutableMemoryAssetBuildError,
    MemoryAssetInsertError, PathError, PathErrorCode,
};
pub use immutable_memory::ImmutableMemoryAssetIo;
pub use memory::MemoryAssetIo;
pub use path::AssetPath;
pub use resource::{AssetIo, AssetLength, AssetReader, AuthorityDomain, OpenedAsset};
/// Stable asset-IO semantic registry used by import's deep resource graph module.
///
/// This is intentionally hidden from the ordinary root API. Importers may provide a custom
/// resolver, but only the import session can decide whether a registered profile is admitted into
/// a successful-input manifest.
#[doc(hidden)]
pub mod __semantic_identity {
    pub use crate::semantic_identity::{
        CanonicalizationProfileId, NamespaceSemanticsId, NegativeProofDigestAlgorithm,
        NegativeProofResourceCaps, NegativeProofSchemeId, NegativeProofSchemeSemanticSubject,
        NegativeProofSubjectKind, NegativeProofValidityRuleId, NormalizedReference,
        ReferenceNormalizerContractId, ReferenceNormalizerSemanticSubject,
        RegisteredAssetResolution, RegisteredNegativeProofError, RegisteredNegativeProofPlan,
        RegisteredNegativeProofScheme, RegisteredReferenceNormalizer, RegisteredResolutionError,
        RegisteredResolverProfile, RegisteredUnavailableCause, ResolverContractId,
        ResolverSemanticSubject, SemanticParameterSchema, StableResourceIdentity,
        StableResourceIdentitySchema, VerifiedNegativeInputProof,
        VerifiedNegativeInputProofBinding, immutable_memory_namespace_absence_v1,
        portable_asset_path_normalizer_v1, same_authority_resolver_v1,
    };
}
