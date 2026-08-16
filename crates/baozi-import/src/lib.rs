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
mod request_wire;
mod scope;
mod session;
mod source_scope;
#[allow(
    dead_code,
    reason = "the private retained-store host bootstrap precedes the atomic ImportSession cutover"
)]
mod store;

pub use config::{
    ConformancePolicy, DetectionOptions, DetectionOptionsError, FormatSelection, ImportConfig,
    ImportConfigOverlay, ImportLossKind, ImportPreset, ImportRequest, ImportRequestBuildError,
    ImportRequestLayer, LossPolicy, MAX_DETECTOR_WORK_BETWEEN_POLLS,
    MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
};
pub use dependency::{
    AuthorizedDependency, DenyDependencies, DependencyCacheDisposition, DependencyIdentity,
    DependencyPolicyError, DependencyReference, DependencyRequirement, DependencyResolver,
    DependencyRoleId, DependencyRoleIdError, DependencySnapshot, OptionalDependencyEvidence,
    OptionalDependencyOutcome, SameAuthorityResolver,
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
    ImportProgressPhase, ImportProgressUnit,
};
pub use limits::{ResourceLimits, ResourceLimitsOverlay};
pub use loss::{ImportLossDisposition, ImportLossEvidence, ImportLossSubject};
pub use metered_storage::{
    MeteredDecodedText, MeteredMetadataMap, MeteredOpaqueAllowance, MeteredOrderedMap,
    MeteredSceneVec, MeteredString, MeteredVec, PreparedSceneValue, SceneValueParts,
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
    DetectionEvidence, ImportFailure, ImportInitializationError, ImportPhase, ImportPhaseEvidence,
    ImportReport, ImportResult, ImportRunError,
};
pub use registry::{
    FormatUnavailability, RegistryBuildError, RegistryBuilder, RegistryRegistrationError,
    RegistrySnapshot,
};
pub use request::{PreparedImportRequest, RequestError};
#[doc(hidden)]
pub use request_wire::{
    RequestWireError, RequestWireLimits, RequestWireScope, SerializedImportRequest,
};
pub use scope::{
    CameraBuildConfig, ImportStorageClass, ImportWorkClass, ImportWorkScope, MaterialBuildConfig,
    MeshBuildConfig, NodeBuildConfig, ParseFailure, ParseResult, ParseScope, ScopedSceneBuilder,
    SkinBuildConfig, TextureBuildConfig, TextureSlotBuildConfig,
};
pub use session::{ReadHint, import_asset, import_asset_with_resolver, import_bytes};
pub use source_scope::{
    FinalizedSourceScopeRecord, SourceScopeCounts, SourceScopeDetail, SourceScopeDetailState,
    SourceScopeDetails, SourceScopeDigest, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeRecord, SourceScopeRecordBuilder,
};
