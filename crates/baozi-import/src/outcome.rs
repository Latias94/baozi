use crate::dependency::OptionalDependencyEvidence;
use crate::descriptor::FormatDescriptor;
use crate::diagnostic::{DiagnosticSummary, ImportDiagnostic};
use crate::error::ImportCause;
use crate::format::DetectionOutcome;
use crate::ledger::{ImportDimension, ImportLedgerSnapshot};
use crate::loss::ImportLossEvidence;
use crate::source_scope::SourceScopeRecord;
use baozi_core::{FormatId, ValidatedScene};
use baozi_io::AssetPath;
use baozi_operation::{LedgerCause, LedgerLimitsError};
use std::fmt;
use std::sync::Arc;

pub type ImportResult<T> = Result<T, ImportRunError>;

/// A typed failure before an import session exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportInitializationError {
    /// The owner-defined ledger limits could not be frozen.
    LedgerLimits {
        source: LedgerLimitsError<ImportDimension>,
    },

    /// The coordinator could not retain the complete accounting-phase catalog.
    Coordinator {
        source: LedgerCause<ImportDimension>,
    },
}

impl ImportInitializationError {
    pub(crate) const fn ledger_limits(source: LedgerLimitsError<ImportDimension>) -> Self {
        Self::LedgerLimits { source }
    }

    pub(crate) const fn coordinator(source: LedgerCause<ImportDimension>) -> Self {
        Self::Coordinator { source }
    }

    /// Returns the limits error when policy freezing failed.
    pub const fn ledger_limits_error(&self) -> Option<&LedgerLimitsError<ImportDimension>> {
        match self {
            Self::LedgerLimits { source } => Some(source),
            Self::Coordinator { .. } => None,
        }
    }

    /// Returns the coordinator cause when phase-catalog initialization failed.
    pub const fn coordinator_cause(&self) -> Option<&LedgerCause<ImportDimension>> {
        match self {
            Self::LedgerLimits { .. } => None,
            Self::Coordinator { source } => Some(source),
        }
    }
}

impl fmt::Display for ImportInitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LedgerLimits { source } => source.fmt(formatter),
            Self::Coordinator { source } => source.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportInitializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LedgerLimits { source } => Some(source),
            Self::Coordinator { source } => Some(source),
        }
    }
}

/// A failure before or during one import invocation.
///
/// Initialization failures occur before an import session or operation ledger
/// exists and therefore cannot carry session evidence. Once initialization
/// succeeds, every failure is retained as a complete [`ImportFailure`].
#[derive(Debug)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "operation failures retain complete bounded evidence without failure-path allocation"
)]
pub enum ImportRunError {
    /// The import operation could not be initialized.
    Initialization { source: ImportInitializationError },

    /// The initialized import operation failed and retained its terminal evidence.
    Operation { failure: ImportFailure },
}

impl ImportRunError {
    pub(crate) const fn initialization(source: LedgerLimitsError<ImportDimension>) -> Self {
        Self::Initialization {
            source: ImportInitializationError::ledger_limits(source),
        }
    }

    pub(crate) const fn coordinator_initialization(source: LedgerCause<ImportDimension>) -> Self {
        Self::Initialization {
            source: ImportInitializationError::coordinator(source),
        }
    }

    pub(crate) const fn operation(failure: ImportFailure) -> Self {
        Self::Operation { failure }
    }

    /// Returns the complete operation failure when initialization succeeded.
    pub const fn operation_failure(&self) -> Option<&ImportFailure> {
        match self {
            Self::Initialization { .. } => None,
            Self::Operation { failure } => Some(failure),
        }
    }

    /// Extracts the complete operation failure without discarding an
    /// initialization error on mismatch.
    pub fn into_operation_failure(self) -> Result<ImportFailure, ImportInitializationError> {
        match self {
            Self::Initialization { source } => Err(source),
            Self::Operation { failure } => Ok(failure),
        }
    }
}

impl fmt::Display for ImportRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initialization { source } => {
                write!(
                    formatter,
                    "import operation initialization failed: {source}"
                )
            }
            Self::Operation { failure } => write!(formatter, "import operation failed: {failure}"),
        }
    }
}

impl std::error::Error for ImportRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Initialization { source } => Some(source),
            Self::Operation { failure } => Some(failure),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportPhase {
    SessionCreated,
    PrimaryBound,
    SelectionCompleted,
    DraftBuilt,
    Validated,
}

/// Sealed lifecycle evidence. Callers may inspect but cannot construct it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImportPhaseEvidence {
    phase: ImportPhase,
}

impl ImportPhaseEvidence {
    pub(crate) const fn new(phase: ImportPhase) -> Self {
        Self { phase }
    }

    pub const fn phase(self) -> ImportPhase {
        self.phase
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionEvidence {
    format: FormatId,
    outcome: DetectionOutcome,
}

impl DetectionEvidence {
    pub(crate) const fn new(format: FormatId, outcome: DetectionOutcome) -> Self {
        Self { format, outcome }
    }

    pub const fn format(&self) -> &FormatId {
        &self.format
    }

    pub const fn outcome(&self) -> &DetectionOutcome {
        &self.outcome
    }
}

pub struct ImportReport {
    scene: ValidatedScene,
    source: AssetPath,
    format: Arc<FormatDescriptor>,
    detection: Vec<DetectionEvidence>,
    diagnostics: Vec<ImportDiagnostic>,
    diagnostic_summary: DiagnosticSummary,
    optional_dependency_evidence: Vec<OptionalDependencyEvidence>,
    loss_evidence: Vec<ImportLossEvidence>,
    source_scope_record: Option<SourceScopeRecord>,
    ledger: ImportLedgerSnapshot,
    last_completed: ImportPhaseEvidence,
}

pub(crate) struct ImportReportParts {
    pub scene: ValidatedScene,
    pub source: AssetPath,
    pub format: Arc<FormatDescriptor>,
    pub detection: Vec<DetectionEvidence>,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub diagnostic_summary: DiagnosticSummary,
    pub optional_dependency_evidence: Vec<OptionalDependencyEvidence>,
    pub loss_evidence: Vec<ImportLossEvidence>,
    pub source_scope_record: Option<SourceScopeRecord>,
    pub ledger: ImportLedgerSnapshot,
    pub last_completed: ImportPhaseEvidence,
}

impl ImportReport {
    pub(crate) fn from_parts(parts: ImportReportParts) -> Self {
        Self {
            scene: parts.scene,
            source: parts.source,
            format: parts.format,
            detection: parts.detection,
            diagnostics: parts.diagnostics,
            diagnostic_summary: parts.diagnostic_summary,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            source_scope_record: parts.source_scope_record,
            ledger: parts.ledger,
            last_completed: parts.last_completed,
        }
    }

    pub const fn scene(&self) -> &ValidatedScene {
        &self.scene
    }

    pub const fn source(&self) -> &AssetPath {
        &self.source
    }

    pub fn format(&self) -> &FormatDescriptor {
        self.format.as_ref()
    }

    pub fn detection_evidence(&self) -> &[DetectionEvidence] {
        &self.detection
    }

    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    pub const fn diagnostic_summary(&self) -> DiagnosticSummary {
        self.diagnostic_summary
    }

    pub fn optional_dependency_evidence(&self) -> &[OptionalDependencyEvidence] {
        &self.optional_dependency_evidence
    }

    pub fn loss_evidence(&self) -> &[ImportLossEvidence] {
        &self.loss_evidence
    }

    pub const fn source_scope_record(&self) -> Option<&SourceScopeRecord> {
        self.source_scope_record.as_ref()
    }

    pub const fn ledger(&self) -> &ImportLedgerSnapshot {
        &self.ledger
    }

    pub const fn last_completed(&self) -> ImportPhaseEvidence {
        self.last_completed
    }
}

impl fmt::Debug for ImportReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportReport")
            .field("source", &self.source)
            .field("format", &self.format.id())
            .field("detection", &self.detection)
            .field("diagnostics", &self.diagnostics)
            .field("diagnostic_summary", &self.diagnostic_summary)
            .field(
                "optional_dependency_evidence",
                &self.optional_dependency_evidence,
            )
            .field("loss_evidence", &self.loss_evidence)
            .field("source_scope_record", &self.source_scope_record)
            .field("ledger", &self.ledger)
            .field("last_completed", &self.last_completed)
            .finish_non_exhaustive()
    }
}

pub struct ImportFailure {
    cause: ImportCause,
    source: AssetPath,
    selected_format: Option<FormatId>,
    detection: Vec<DetectionEvidence>,
    diagnostics: Vec<ImportDiagnostic>,
    diagnostic_summary: DiagnosticSummary,
    optional_dependency_evidence: Vec<OptionalDependencyEvidence>,
    loss_evidence: Vec<ImportLossEvidence>,
    source_scope_record: Option<SourceScopeRecord>,
    ledger: ImportLedgerSnapshot,
    finalization_cause: Option<LedgerCause<ImportDimension>>,
    last_completed: ImportPhaseEvidence,
}

pub(crate) struct ImportFailureParts {
    pub cause: ImportCause,
    pub source: AssetPath,
    pub selected_format: Option<FormatId>,
    pub detection: Vec<DetectionEvidence>,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub diagnostic_summary: DiagnosticSummary,
    pub optional_dependency_evidence: Vec<OptionalDependencyEvidence>,
    pub loss_evidence: Vec<ImportLossEvidence>,
    pub source_scope_record: Option<SourceScopeRecord>,
    pub ledger: ImportLedgerSnapshot,
    pub finalization_cause: Option<LedgerCause<ImportDimension>>,
    pub last_completed: ImportPhaseEvidence,
}

impl ImportFailure {
    pub(crate) fn from_parts(parts: ImportFailureParts) -> Self {
        Self {
            cause: parts.cause,
            source: parts.source,
            selected_format: parts.selected_format,
            detection: parts.detection,
            diagnostics: parts.diagnostics,
            diagnostic_summary: parts.diagnostic_summary,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            source_scope_record: parts.source_scope_record,
            ledger: parts.ledger,
            finalization_cause: parts.finalization_cause,
            last_completed: parts.last_completed,
        }
    }

    pub const fn cause(&self) -> &ImportCause {
        &self.cause
    }

    pub const fn source_identity(&self) -> &AssetPath {
        &self.source
    }

    pub const fn selected_format(&self) -> Option<&FormatId> {
        self.selected_format.as_ref()
    }

    pub fn detection_evidence(&self) -> &[DetectionEvidence] {
        &self.detection
    }

    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    pub const fn diagnostic_summary(&self) -> DiagnosticSummary {
        self.diagnostic_summary
    }

    pub fn optional_dependency_evidence(&self) -> &[OptionalDependencyEvidence] {
        &self.optional_dependency_evidence
    }

    pub fn loss_evidence(&self) -> &[ImportLossEvidence] {
        &self.loss_evidence
    }

    pub const fn source_scope_record(&self) -> Option<&SourceScopeRecord> {
        self.source_scope_record.as_ref()
    }

    pub const fn ledger(&self) -> &ImportLedgerSnapshot {
        &self.ledger
    }

    /// Returns a ledger-finalization invariant without replacing the owning failure cause.
    pub const fn finalization_cause(&self) -> Option<&LedgerCause<ImportDimension>> {
        self.finalization_cause.as_ref()
    }

    pub const fn last_completed(&self) -> ImportPhaseEvidence {
        self.last_completed
    }
}

impl fmt::Debug for ImportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportFailure")
            .field("cause", &self.cause)
            .field("source", &self.source)
            .field("selected_format", &self.selected_format)
            .field("detection", &self.detection)
            .field("diagnostics", &self.diagnostics)
            .field("diagnostic_summary", &self.diagnostic_summary)
            .field(
                "optional_dependency_evidence",
                &self.optional_dependency_evidence,
            )
            .field("loss_evidence", &self.loss_evidence)
            .field("source_scope_record", &self.source_scope_record)
            .field("ledger", &self.ledger)
            .field("finalization_cause", &self.finalization_cause)
            .field("last_completed", &self.last_completed)
            .finish()
    }
}

impl fmt::Display for ImportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for ImportFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_initialization_retains_its_typed_source_on_mismatch() {
        let cause: LedgerCause<ImportDimension> = LedgerCause::DuplicatePhase;
        let run_error = ImportRunError::coordinator_initialization(cause.clone());

        let initialization_source = std::error::Error::source(&run_error)
            .and_then(|source| source.downcast_ref::<ImportInitializationError>())
            .expect("run error must retain the typed initialization source");
        assert_eq!(initialization_source.coordinator_cause(), Some(&cause));
        let coordinator_source = std::error::Error::source(initialization_source)
            .and_then(|source| source.downcast_ref::<LedgerCause<ImportDimension>>())
            .expect("initialization error must retain the typed coordinator cause");
        assert_eq!(coordinator_source, &cause);

        let initialization = run_error
            .into_operation_failure()
            .expect_err("coordinator initialization cannot be an operation failure");
        assert_eq!(initialization.coordinator_cause(), Some(&cause));
    }
}
