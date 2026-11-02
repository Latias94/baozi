#![forbid(unsafe_code)]

//! Typed format registration, import plans, and exclusive parse-context contracts.
//!
//! The retired pre-release request serialization protocol is not part of the package interface:
//!
//! ```compile_fail,E0432
//! use baozi_import::SerializedImportRequest;
//! ```
//!
//! The default same-authority resolver is session-owned rather than a caller-constructible
//! semantic authority:
//!
//! ```compile_fail,E0432
//! use baozi_import::SameAuthorityResolver;
//! ```

pub use baozi_operation::{
    CapacityPlan, CapacityPlanError, LedgerCause, MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS,
};

mod asset_open;
pub mod authoring;
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
mod resource_graph;
mod resource_hash;
mod scope;
mod session;
mod source_scope;
mod store;
mod text_input;

pub use baozi_core::{
    AnimationInterpolation, AnimationTarget, AppearanceBuildResource, AppearanceLimitDimension,
    EntityNameError, MaterialFactorGroup, SceneAnimationRateKind, SceneAnimationSamplePart,
    SceneAnimationTarget, SceneAppearanceRule, SceneAuthoredProfileOwner, SceneGenerationId,
    SceneInputCollection, SceneLightEmissionRule, SceneProfileOwner, SceneSnapshot,
    SceneSnapshotError, SceneSnapshotLimits, SceneValidationDomain, SceneValidationError,
    SceneValidationLimit, TextureStrengthRequirement, ValidatedScene,
};
pub use baozi_core::{
    DigestAlgorithm, EntityMetadataOwner, MetadataArrayRef, MetadataEntryRef, MetadataOwnerRef,
    MetadataValueRef, ProfileResourceRef, ProfileResourceStabilityRef, ProfileSessionOpaqueReason,
    SceneMetadataOwner, SourceIdentityRef, SourceObjectKind, SourceObjectLocatorRef,
};

pub use config::{
    ConformancePolicy, DetectionOptions, DetectionOptionsError, FormatSelection, ImportConfig,
    ImportConfigOverlay, ImportLossKind, ImportPreset, ImportRequest, ImportRequestBuildError,
    ImportRequestLayer, LossPolicy, MAX_DETECTOR_WORK_BETWEEN_POLLS,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
};
pub use dependency::{
    AuthorizedDependency, DenyDependencies, DependencyCacheDisposition, DependencyFailureContext,
    DependencyIdentity, DependencyOrigin, DependencyOutcome, DependencyPolicyError,
    DependencyReference, DependencyRequirement, DependencyResolver, DependencyRoleId,
    DependencyRoleIdError, DependencySnapshot, OptionalDependencyEvidence,
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
    DiagnosticSummary, ImportDiagnostic, TextDiagnosticLocation,
};
pub use error::{
    CauseOwner, DetectionCandidates, DetectionFailure, FormatFailureDetail,
    FormatFailureDetailCause, FormatFailureDetailSink, FormatFailureDetailValue,
    FormatFailureEnvelope, FormatFailureEvidence, FormatFailureEvidenceProducer, ImportCause,
    ScopeAbort, ScopeAbortCause, TrustedExtensionBoundary,
};
pub use format::{
    BoundedFaultDetail, DetectionFault, DetectionFaultCode, DetectionInconclusive,
    DetectionOutcome, Format, Probe, ReadConfidence,
};
pub use ledger::{
    ImportDimension, ImportDimensionSnapshot, ImportLedgerPhase, ImportLedgerSnapshot,
    ImportOperationControl, ImportProgressPhase, ImportProgressUnit, ImportTerminalCause,
    ImportTerminalFailure, ImportTerminalFailureClass,
};
pub use limits::{ResourceLimits, ResourceLimitsOverlay};
pub use loss::{ImportLossDisposition, ImportLossEvidence, ImportLossSubject};
pub use metered_storage::{
    MeteredDecodedText, MeteredOpaqueAllowance, MeteredOrderedMap, MeteredString, MeteredVec,
};
pub use options::{
    CompiledOptions, FormatOptionEnvelope, FormatOptionEnvelopeError, FormatOptionLayers,
    FormatOptionPayload, FormatOptionReader, FormatOptionSchema, FormatOptionSchemaId,
    FormatOptionSchemaIdError, FormatOptionSchemaKey, NoFormatOptions, OptionBudget,
    OptionCompileError, OptionCompileLimits, OptionCompileScope, OptionErrorCode,
    OptionLiveReservation, OptionScopeLimits, RegistryBuildLimits, RegistryBuildScope,
};
pub use outcome::{
    DetectionEvidence, ImportEvidence, ImportFailure, ImportInitializationError, ImportPhase,
    ImportPhaseEvidence, ImportReport, ImportResult, ImportRunError,
};
pub use registry::{
    FormatUnavailability, RegistryBuildError, RegistryBuilder, RegistryRegistrationError,
    RegistrySnapshot,
};
pub use request::{ImportPlan, RequestError};
pub use resource_graph::{
    InputReplayStatus, ManifestDigest, ManifestNamespaceSnapshotProof, ManifestNodeClass,
    ManifestObservedOnlyReason, ManifestObservedOnlyReasons, ManifestPerReferenceProof,
    ManifestReference, ManifestRelation, ManifestRelationOutcome, ManifestResourceIdentity,
    ManifestResourceIdentityStability, ManifestSemanticIdentity, ManifestSessionDependentReason,
    ManifestSessionOpaqueReason, ManifestUnavailableClass, ManifestUnavailableCode,
    ManifestUnavailableReplay, ResourceManifestCounts, SuccessfulInputManifest,
};
pub use scope::{
    ImportStorageClass, ImportWorkClass, ImportWorkGroup, ImportWorkScope, ParseContext,
    ParseExecution, ParseFailure, ParseResult,
};
pub use session::ReadHint;
pub use source_scope::{
    FinalizedSourceScopeRecord, SourceScopeCounts, SourceScopeDetail, SourceScopeDetailState,
    SourceScopeDetails, SourceScopeDigest, SourceScopeDisposition, SourceScopeDocumentIdentity,
    SourceScopeEntityKind, SourceScopeRecord, SourceScopeRecordBuilder,
    SourceScopeSelectionOutcome, SourceScopeSelectionRequest, SourceScopeSourceIdentity,
};
pub use text_input::{
    ConformedTextInput, TextConformanceResult, TextConformanceViolation,
    TextConformanceViolationKind, TextInputBom, TextInputLine, TextInputOptions,
    TextInputRecordCursor, TextInputRecoveryTracker, TextInputReplacement,
};
