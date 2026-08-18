#![forbid(unsafe_code)]

//! Import registry, request, session, and format capability contracts.

pub use baozi_operation::{CapacityPlan, CapacityPlanError, LedgerCause};

mod config;
mod dependency;
mod descriptor;
mod detection;
mod diagnostic;
mod error;
mod format;
mod ledger;
mod limits;
mod loss;
mod metered_storage;
mod options;
mod outcome;
mod registry;
mod request;
mod scope;
mod session;
mod source_scope;
mod store;

pub use baozi_core::{
    SceneGenerationId, SceneSnapshot, SceneSnapshotError, SceneSnapshotLimits, ValidatedScene,
};

#[doc(hidden)]
pub use baozi_core::__canonical_import::{
    AlphaMode, AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring, AppearanceAuthoringBatch, AppearanceBuildError, AttributeDomain,
    AttributeSemantic, AuthoredTransform, AuthoringRange, Axis, CameraAuthoring, CameraLocalId,
    CameraProjection, Color, ColorInterpretation, ContentDigest, CustomAttributeSemantic,
    DeformationAuthoringBatch, DigestAlgorithm, EntityName, EvidenceDigest, ExtensionMediaType,
    ExtensionValueRef, FormatId, FrontFaceWinding, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, Handedness, ImageAuthoring,
    ImageLocalId, InfluenceAuthoring, InfluenceRowsAuthoring, LegacyMaterialFactors,
    LightAuthoring, LightAuthoringError, LightKind, LightLocalId, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, MAX_ENTITY_NAME_BYTES, Mat4, MaterialAuthoring,
    MaterialBindingAuthoring, MaterialFactors, MaterialLocalId, MeshInstanceDeformationAuthoring,
    MetadataAuthoringBatch, MetadataAuthoringEntry, MipmapFilter, MorphDeltaAuthoring,
    MorphDeltaPayloadAuthoring, MorphDeltaSemantic, MorphTargetDefinitionAuthoring,
    MorphTargetIdentity, NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId,
    NodeParent, ObservedResourceAuthoring, OperationEvidenceSummary, OperationKind, OptionsDigest,
    OwnerProvenanceAuthoring, PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors, Presence,
    PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, Quat,
    ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring, SamplerAuthoring,
    SamplerLocalId, SamplerState, SceneAuthoringBatch, SceneDraft, SceneSpace, SessionOpaqueReason,
    ShadingModel, SkinAuthoring, SkinLocalId, SkinMatrixSpaces, SourceDisplayName,
    SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator,
    SpatialAuthoringBatch, SupplementalPbrFactors, TangentBasisConvention,
    TangentHandednessEncoding, TexelFilter, TextureAddressMode, TextureAuthoring,
    TextureBlendOperation, TextureChannel, TextureChannelMapping, TextureChannelSwizzle,
    TextureCoordinateSource, TextureLocalId, TextureRole, TextureSourceKey, TextureStrength,
    TextureTransform, TransformConvention, UvOrigin, Vec2, Vec3,
};

pub use config::{
    ConformancePolicy, DetectionOptions, DetectionOptionsError, FormatSelection, ImportConfig,
    ImportConfigOverlay, ImportLossKind, ImportPreset, ImportRequest, ImportRequestBuildError,
    ImportRequestLayer, LossPolicy, MAX_DETECTOR_WORK_BETWEEN_POLLS,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
};
pub use dependency::{
    AuthorizedDependency, DenyDependencies, DependencyCacheDisposition, DependencyIdentity,
    DependencyOrigin, DependencyOutcome, DependencyPolicyError, DependencyReference,
    DependencyRequirement, DependencyResolver, DependencyRoleId, DependencyRoleIdError,
    DependencySnapshot, OptionalDependencyEvidence, SameAuthorityResolver,
};
pub use descriptor::{
    CapabilityStatus, DeclarationEvidenceAuthority, DescriptorError, FormatCapability,
    FormatDescriptor, FormatDescriptorBuilder, FormatEncoding, TextBomPolicy, TextControlPolicy,
    TextDecodingMode, TextHardCapDimension, TextHardCaps, TextLexicalMode, TextNewline,
    TextProfile, TextProfileBuilder, TextProfileError, TextProfileField, TextSourceEncoding,
};
pub use detection::{DetectionAbort, DetectionResult, DetectionScope};
pub use diagnostic::{
    DiagnosticByteRange, DiagnosticCause, DiagnosticCode, DiagnosticDetail, DiagnosticDetailSink,
    DiagnosticDetailValue, DiagnosticLineColumn, DiagnosticSeverity, DiagnosticSource,
    DiagnosticSummary, ImportDiagnostic,
};
pub use error::{
    CauseOwner, DetectionFailure, FormatFailureDetail, FormatFailureDetailCause,
    FormatFailureDetailSink, FormatFailureDetailValue, FormatFailureEnvelope,
    FormatFailureEvidence, FormatFailureEvidenceProducer, ImportCause, ScopeAbort, ScopeAbortCause,
    TrustedExtensionBoundary,
};
pub use format::{
    BoundedFaultDetail, DetectionFault, DetectionFaultCode, DetectionInconclusive,
    DetectionOutcome, FormatImporter, Probe, ReadConfidence,
};
pub use ledger::{
    ImportDimension, ImportLedgerPhase, ImportLedgerSnapshot, ImportOperationControl,
    ImportProgressPhase, ImportProgressUnit, ImportTerminalCause, ImportTerminalFailure,
    ImportTerminalFailureClass,
};
pub use limits::{ResourceLimits, ResourceLimitsOverlay};
pub use loss::{ImportLossDisposition, ImportLossEvidence, ImportLossSubject};
pub use metered_storage::{
    MeteredDecodedText, MeteredOpaqueAllowance, MeteredOrderedMap, MeteredString, MeteredVec,
};
pub use options::{
    CompiledFormatOptions, FormatOptionCodec, FormatOptionEnvelope, FormatOptionEnvelopeError,
    FormatOptionKey, FormatOptionLayers, FormatOptionPayload, FormatOptionReader,
    FormatOptionSchema, FormatOptionSchemaId, FormatOptionSchemaIdError, FormatOptionSchemaKey,
    NoFormatOptions, OptionBudget, OptionCompileError, OptionCompileLimits, OptionCompileScope,
    OptionErrorCode, OptionLiveReservation, OptionRuntimeTypeKey, OptionScopeLimits,
    RegistryBuildLimits, RegistryBuildScope,
};
pub use outcome::{
    DetectionEvidence, ImportEvidence, ImportFailure, ImportInitializationError, ImportPhase,
    ImportPhaseEvidence, ImportReport, ImportResult, ImportRunError,
};
pub use registry::{
    FormatUnavailability, RegistryBuildError, RegistryBuilder, RegistryRegistrationError,
    RegistrySnapshot,
};
pub use request::{PreparedImportRequest, RequestError};
pub use scope::{
    ImportStorageClass, ImportWorkClass, ImportWorkScope, ParseFailure, ParseResult, ParseScope,
};
pub use session::{ReadHint, import_asset, import_asset_with_resolver, import_bytes};
pub use source_scope::{
    FinalizedSourceScopeRecord, SourceScopeCounts, SourceScopeDetail, SourceScopeDetailState,
    SourceScopeDetails, SourceScopeDigest, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeRecord, SourceScopeRecordBuilder,
};
