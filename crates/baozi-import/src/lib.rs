#![forbid(unsafe_code)]

//! Import registry and format importer contracts.

pub mod context;
pub mod format;
pub mod outcome;
pub mod registry;

pub use context::{
    DetectionOptions, DiagnosticOptions, ExternalReferencePolicy, ImportContext, ImportOptions,
    ImportReport, ImportStage, ImportStats, IoOptions, ReadHint, ResourceLedger,
};
pub use format::{
    CapabilityStatus, DetectionFault, DetectionInconclusive, DetectionOutcome, FormatCapability,
    FormatEncoding, FormatImporter, FormatInfo, FormatMaturity, FormatSidecarPolicy, Probe,
    ReadConfidence,
};
pub use outcome::{ImportFailure, ImportResult};
pub use registry::{
    DetectionEvidence, DetectionFailure, DetectionResult, ImporterRegistry, PrimaryProbeOperation,
    SelectedImporter,
};
