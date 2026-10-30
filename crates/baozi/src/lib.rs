#![forbid(unsafe_code)]

//! Public Baozi facade.
//!
//! Native filesystem helpers are candidate-only under `native-fs-candidate`; no release
//! `native-fs` capability is currently registered.
//!
//! Custom [`AssetIo`] backends and [`DependencyResolver`] policies use the types re-exported
//! here. Implementing format parsers additionally requires `baozi_import` for parser scopes,
//! option codecs, descriptor construction, and Scene authoring contracts.

mod error;
mod importer;

pub use error::{Error, Result};
pub use importer::{Importer, ImporterBuilder};

/// Explicit, controlled geometry operations on immutable validated Scenes.
#[cfg(feature = "postprocess")]
pub use baozi_postprocess as postprocess;

pub use baozi_core::{
    Aabb, Color, FormatId, FormatIdError, Mat4, Quat, SceneGenerationId, Vec2, Vec3, Vec4,
};
pub use baozi_core::{
    AlphaCutoff, AlphaMode, ColorInterpretation, ColorInterpretationKind, ExtensionSchemaId,
    ExtensionSchemaIdError, LegacyMaterialFactors, MaterialFactorError, MaterialFactorField,
    MaterialFactors, NamespacedName, NamespacedNameError, PbrMetallicRoughnessFactors,
    PbrSpecularGlossinessFactors, ShadingModel, ShadingModelKind, SupplementalPbrFactors,
    TextureBlendOperation, TextureChannel, TextureChannelMapping, TextureChannelSwizzle,
    TextureCoordinateSource, TextureRole, TextureRoleKind, TextureStrength, TextureTransform,
};
pub use baozi_core::{
    AnimationChannelRef, AnimationClipId, AnimationClipRef, AnimationExtrapolation,
    AnimationInterpolation, AnimationTarget, AnimationTimeDomain,
};
pub use baozi_core::{
    AppearanceBehaviorKind, AppearanceBuildResource, AppearanceLimitDimension, EntityNameError,
    MaterialFactorGroup, SceneAnimationRateKind, SceneAnimationSamplePart, SceneAnimationTarget,
    SceneAppearanceRule, SceneAuthoredProfileOwner, SceneInputCollection, SceneLightEmissionRule,
    SceneProfileOwner, SceneValidationDomain, SceneValidationError, SceneValidationLimit,
    TextureStrengthRequirement,
};
pub use baozi_core::{
    AttributeData, AttributeDomain, AttributeRef, AttributeSemantic, AttributeSemanticKind,
    AttributeSemanticRef, AuthoredTransform, AuthoredTransformError, AuthoredTransformKind, Axis,
    CameraId, CameraProjection, CameraRef, CustomAttributeSemantic, FrontFaceWinding, Handedness,
    ImageId, ImageRef, LightId, LightKind, LightRef, LogicalMeshId, LogicalMeshRef,
    MaterialBindingRef, MaterialId, MaterialRef, MeshInstanceId, MeshInstanceRef, MipmapFilter,
    NodeChildren, NodeId, NodeRef, PrimitiveId, PrimitiveRef, SamplerId, SamplerRef, SamplerState,
    SceneEntityKind, SceneLookupError, SceneSpaceRef, TangentBasisRef, TangentEncodingRef,
    TexelFilter, TextureAddressMode, TextureId, TextureRef, TopologyKind, TransformConvention,
    TrsTransform, UvOrigin,
};
pub use baozi_core::{CameraProjectionError, CameraProjectionKind, LightError, LightKindCategory};
pub use baozi_core::{
    InfluenceRef, InfluenceRowRef, InfluenceRows, MorphDeltaData, MorphDeltaRef,
    MorphDeltaSemantic, MorphTargetDefinitionRef, MorphTargetId, MorphTargetRef, SkinId,
    SkinMatrixCorrection, SkinMatrixCorrectionError, SkinMatrixSpace, SkinMatrixSpaces, SkinRef,
};
pub use baozi_core::{
    SceneSpace, SceneSpaceField, SceneSpaceValidationError, TangentBasisConvention,
    TangentHandednessEncoding,
};
pub use baozi_import::{
    AuthorizedDependency, CapabilityStatus, CauseOwner, ConformancePolicy,
    DeclarationEvidenceAuthority, DependencyFailureContext, DependencyIdentity,
    DependencyPolicyError, DependencyReference, DependencyRequirement, DependencyResolver,
    DependencyRoleId, DependencyRoleIdError, DetectionEvidence, DetectionFailure, DetectionFault,
    DetectionFaultCode, DetectionInconclusive, DetectionOptions, DetectionOptionsError,
    DetectionOutcome, DiagnosticCause, DiagnosticCode, DiagnosticSeverity, DiagnosticSummary,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatFailureDetail,
    FormatFailureDetailValue, FormatFailureEnvelope, FormatFailureEvidence, FormatImporter,
    FormatOptionEnvelope, FormatOptionEnvelopeError, FormatOptionSchemaId,
    FormatOptionSchemaIdError, FormatOptionSchemaKey, FormatSelection, FormatUnavailability,
    ImportCause, ImportConfig, ImportConfigOverlay, ImportDiagnostic, ImportDimension,
    ImportFailure, ImportInitializationError, ImportLedgerPhase, ImportLedgerSnapshot,
    ImportLossKind, ImportOperationControl, ImportPhase, ImportPhaseEvidence, ImportPreset,
    ImportProgressPhase, ImportProgressUnit, ImportReport, ImportRequest, ImportRequestBuildError,
    ImportRequestLayer, ImportRunError, ImportTerminalCause, ImportTerminalFailure,
    ImportTerminalFailureClass, InputReplayStatus, LossPolicy, ManifestDigest,
    ManifestNamespaceSnapshotProof, ManifestNodeClass, ManifestObservedOnlyReason,
    ManifestObservedOnlyReasons, ManifestPerReferenceProof, ManifestReference, ManifestRelation,
    ManifestRelationOutcome, ManifestResourceIdentity, ManifestResourceIdentityStability,
    ManifestSemanticIdentity, ManifestSessionDependentReason, ManifestSessionOpaqueReason,
    ManifestUnavailableClass, ManifestUnavailableCode, ManifestUnavailableReplay,
    OptionCompileLimits, OptionScopeLimits, PreparedImportRequest, ReadConfidence,
    RegistryBuildError, RegistryBuildLimits, RegistryRegistrationError, RequestError,
    ResourceLimits, ResourceLimitsOverlay, ResourceManifestCounts, SceneSnapshot,
    SceneSnapshotError, SceneSnapshotLimits, ScopeAbort, ScopeAbortCause, SuccessfulInputManifest,
    TrustedExtensionBoundary, ValidatedScene,
};
pub use baozi_io::{
    AssetIo, AssetLength, AssetObjectKind, AssetOpenError, AssetOpenErrorKind, AssetOpenStage,
    AssetPath, AssetReadContractViolation, AssetReadError, AssetReadErrorKind, AssetReader,
    AuthorityDomain, ImmutableMemoryAssetBuildError, ImmutableMemoryAssetIo,
    MemoryAssetInsertError, MemoryAssetIo, NATIVE_FS_CANDIDATE_TARGETS,
    NATIVE_FS_REGISTERED_TARGETS, NATIVE_FS_SUPPORT_STATUS, NativeFsSupportStatus, OpenedAsset,
    PathError, PathErrorCode,
};
#[cfg(feature = "native-fs-candidate")]
pub use baozi_io::{FileSystemAssetIo, NativeDirectoryCapability};
pub use baozi_operation::{
    CancellationOrigin, CancellationSignal, CancellationToken, ControlCause, LedgerLimitsError,
    ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ProgressObserver,
    ProgressStorageError, ProgressViolation,
};
