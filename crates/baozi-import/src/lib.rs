#![forbid(unsafe_code)]

//! Import registry, request, session, and format capability contracts.
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

pub use baozi_operation::{CapacityPlan, CapacityPlanError, LedgerCause};

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
mod scope;
mod session;
mod source_scope;
mod store;

/// Opaque descriptor returned after a parser submits one complete canonical Scene batch.
///
/// The descriptor cannot be cloned or constructed by importer code. Its matching retained owner
/// remains inside the import session and is consumed when parsing finishes.
///
/// ```compile_fail,E0599
/// fn clone_draft(draft: baozi_import::SceneDraft) -> baozi_import::SceneDraft {
///     draft.clone()
/// }
/// ```
pub use baozi_core::scene::integration::SceneDraft;
pub use baozi_core::{
    SceneGenerationId, SceneSnapshot, SceneSnapshotError, SceneSnapshotLimits, ValidatedScene,
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
    DependencySnapshot, OptionalDependencyEvidence,
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
pub use resource_graph::{
    InputReplayStatus, ManifestDigest, ManifestNamespaceSnapshotProof, ManifestNodeClass,
    ManifestObservedOnlyReason, ManifestObservedOnlyReasons, ManifestPerReferenceProof,
    ManifestReference, ManifestRelation, ManifestRelationOutcome, ManifestResourceIdentity,
    ManifestResourceIdentityStability, ManifestSemanticIdentity, ManifestSessionDependentReason,
    ManifestSessionOpaqueReason, ManifestUnavailableClass, ManifestUnavailableCode,
    ManifestUnavailableReplay, ResourceManifestCounts, SuccessfulInputManifest,
};
pub use scope::{
    ImportStorageClass, ImportWorkClass, ImportWorkScope, ParseFailure, ParseResult, ParseScope,
};
pub use session::{ReadHint, import_asset, import_asset_with_resolver, import_bytes};
pub use source_scope::{
    FinalizedSourceScopeRecord, SourceScopeCounts, SourceScopeDetail, SourceScopeDetailState,
    SourceScopeDetails, SourceScopeDigest, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeRecord, SourceScopeRecordBuilder,
};
