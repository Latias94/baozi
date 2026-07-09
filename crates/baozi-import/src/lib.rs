#![forbid(unsafe_code)]

//! Import registry and format importer contracts.

pub mod context;
pub mod format;
pub mod registry;

pub use context::{
    DetectionOptions, DiagnosticOptions, ExternalReferencePolicy, ImportContext, ImportOptions,
    ImportReport, ImportStage, ImportStats, IoOptions, ReadHint, ResourceLedger,
};
pub use format::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ReadConfidence,
};
pub use registry::{ImporterRegistry, SelectedImporter};
