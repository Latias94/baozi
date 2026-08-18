use crate::dependency::OptionalDependencyEvidence;
use crate::descriptor::FormatDescriptor;
use crate::diagnostic::{DiagnosticSummary, ImportDiagnostic};
use crate::error::ImportCause;
use crate::format::DetectionOutcome;
use crate::ledger::{ImportDimension, ImportLedgerSnapshot};
use crate::loss::ImportLossEvidence;
use crate::source_scope::SourceScopeRecord;
use baozi_core::__canonical_import::{
    SceneMinimalFallback, SceneRichFailure, SceneSuccessReport, ValidatedScene,
};
use baozi_core::{FormatId, SceneGenerationId};
use baozi_io::AssetPath;
use baozi_operation::LedgerLimitsError;
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

    /// The retained-store host could not be created before any import IO began.
    RetainedStore { code: &'static str },
}

impl ImportInitializationError {
    pub(crate) const fn ledger_limits(source: LedgerLimitsError<ImportDimension>) -> Self {
        Self::LedgerLimits { source }
    }

    pub(crate) const fn retained_store(code: &'static str) -> Self {
        Self::RetainedStore { code }
    }

    /// Returns the limits error when policy freezing failed.
    pub const fn ledger_limits_error(&self) -> Option<&LedgerLimitsError<ImportDimension>> {
        match self {
            Self::LedgerLimits { source } => Some(source),
            Self::RetainedStore { .. } => None,
        }
    }
}

impl fmt::Display for ImportInitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LedgerLimits { source } => source.fmt(formatter),
            Self::RetainedStore { code } => {
                write!(
                    formatter,
                    "retained import store initialization failed: {code}"
                )
            }
        }
    }
}

impl std::error::Error for ImportInitializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LedgerLimits { source } => Some(source),
            Self::RetainedStore { .. } => None,
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

    /// Fail-closed terminal preparation could not produce auditable import evidence.
    Finalization { code: &'static str },
}

impl ImportRunError {
    pub(crate) const fn initialization(source: LedgerLimitsError<ImportDimension>) -> Self {
        Self::Initialization {
            source: ImportInitializationError::ledger_limits(source),
        }
    }

    pub(crate) const fn retained_store_initialization(code: &'static str) -> Self {
        Self::Initialization {
            source: ImportInitializationError::retained_store(code),
        }
    }

    pub(crate) const fn operation(failure: ImportFailure) -> Self {
        Self::Operation { failure }
    }

    pub(crate) const fn finalization(code: &'static str) -> Self {
        Self::Finalization { code }
    }

    /// Returns the complete operation failure when initialization succeeded.
    pub const fn operation_failure(&self) -> Option<&ImportFailure> {
        match self {
            Self::Initialization { .. } | Self::Finalization { .. } => None,
            Self::Operation { failure } => Some(failure),
        }
    }

    /// Extracts the complete operation failure without discarding an
    /// initialization error on mismatch.
    #[allow(
        clippy::result_large_err,
        reason = "the error retains its linear terminal owner without post-terminal allocation"
    )]
    pub fn into_operation_failure(self) -> Result<ImportFailure, Self> {
        match self {
            error @ (Self::Initialization { .. } | Self::Finalization { .. }) => Err(error),
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
            Self::Finalization { code } => {
                write!(formatter, "import terminal finalization failed: {code}")
            }
        }
    }
}

impl std::error::Error for ImportRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Initialization { source } => Some(source),
            Self::Operation { failure } => Some(failure),
            Self::Finalization { .. } => None,
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
    evidence: ImportEvidence,
}

/// Import evidence retained independently from the published Scene owner.
///
/// The value keeps the success publication, source identity, diagnostics, losses,
/// dependency evidence, and terminal ledger coupled after a caller moves the Scene
/// into a later owning pipeline.
#[must_use = "import evidence retains the successful publication and terminal ledger"]
pub struct ImportEvidence {
    scene_generation: SceneGenerationId,
    _scene_report: SceneSuccessReport,
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
    pub scene_report: SceneSuccessReport,
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
        let scene_generation = parts.scene.generation();
        Self {
            scene: parts.scene,
            evidence: ImportEvidence {
                scene_generation,
                _scene_report: parts.scene_report,
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
            },
        }
    }

    pub const fn scene(&self) -> &ValidatedScene {
        &self.scene
    }

    /// Borrows the evidence jointly retained with the successful import.
    pub const fn evidence(&self) -> &ImportEvidence {
        &self.evidence
    }

    /// Consumes the report into the unique Scene owner and its independent evidence.
    pub fn into_parts(self) -> (ValidatedScene, ImportEvidence) {
        (self.scene, self.evidence)
    }

    pub const fn source(&self) -> &AssetPath {
        self.evidence.source()
    }

    pub fn format(&self) -> &FormatDescriptor {
        self.evidence.format()
    }

    pub fn detection_evidence(&self) -> &[DetectionEvidence] {
        self.evidence.detection_evidence()
    }

    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        self.evidence.diagnostics()
    }

    pub const fn diagnostic_summary(&self) -> DiagnosticSummary {
        self.evidence.diagnostic_summary()
    }

    pub fn optional_dependency_evidence(&self) -> &[OptionalDependencyEvidence] {
        self.evidence.optional_dependency_evidence()
    }

    pub fn loss_evidence(&self) -> &[ImportLossEvidence] {
        self.evidence.loss_evidence()
    }

    pub const fn source_scope_record(&self) -> Option<&SourceScopeRecord> {
        self.evidence.source_scope_record()
    }

    pub const fn ledger(&self) -> &ImportLedgerSnapshot {
        self.evidence.ledger()
    }

    pub const fn last_completed(&self) -> ImportPhaseEvidence {
        self.evidence.last_completed()
    }
}

impl ImportEvidence {
    /// Returns the canonical generation jointly published with this evidence.
    pub const fn scene_generation(&self) -> SceneGenerationId {
        self.scene_generation
    }

    /// Returns whether this evidence belongs to the provided canonical Scene.
    pub fn matches_scene(&self, scene: &ValidatedScene) -> bool {
        self.scene_generation == scene.generation()
    }

    /// Returns the primary source identity retained by the import session.
    pub const fn source(&self) -> &AssetPath {
        &self.source
    }

    /// Returns the selected format descriptor.
    pub fn format(&self) -> &FormatDescriptor {
        self.format.as_ref()
    }

    /// Returns ordered detection evidence.
    pub fn detection_evidence(&self) -> &[DetectionEvidence] {
        &self.detection
    }

    /// Returns retained diagnostics.
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    /// Returns the precomputed diagnostic summary.
    pub const fn diagnostic_summary(&self) -> DiagnosticSummary {
        self.diagnostic_summary
    }

    /// Returns optional dependency evidence.
    pub fn optional_dependency_evidence(&self) -> &[OptionalDependencyEvidence] {
        &self.optional_dependency_evidence
    }

    /// Returns structured loss evidence.
    pub fn loss_evidence(&self) -> &[ImportLossEvidence] {
        &self.loss_evidence
    }

    /// Returns the bounded source-scope record, when one was published.
    pub const fn source_scope_record(&self) -> Option<&SourceScopeRecord> {
        self.source_scope_record.as_ref()
    }

    /// Returns immutable terminal ledger evidence.
    pub const fn ledger(&self) -> &ImportLedgerSnapshot {
        &self.ledger
    }

    /// Returns the final completed import lifecycle phase.
    pub const fn last_completed(&self) -> ImportPhaseEvidence {
        self.last_completed
    }
}

impl fmt::Debug for ImportReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportReport")
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ImportEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportEvidence")
            .field("scene_generation", &self.scene_generation)
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
    _store_owner: ImportFailureStoreOwner,
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
    pub store_owner: ImportFailureStoreOwner,
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
            _store_owner: parts.store_owner,
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
            .field("last_completed", &self.last_completed)
            .finish()
    }
}

pub(crate) enum ImportFailureStoreOwner {
    Rich { _owner: SceneRichFailure },
    Minimal { _owner: SceneMinimalFallback },
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
    fn retained_store_initialization_preserves_its_typed_error_on_mismatch() {
        const CODE: &str = "retained-store-start-rejected";
        let run_error = ImportRunError::retained_store_initialization(CODE);

        let initialization_source = std::error::Error::source(&run_error)
            .and_then(|source| source.downcast_ref::<ImportInitializationError>())
            .expect("run error must retain the typed initialization source");
        assert!(matches!(
            initialization_source,
            ImportInitializationError::RetainedStore { code } if *code == CODE
        ));
        assert!(std::error::Error::source(initialization_source).is_none());

        let mismatched_error = run_error
            .into_operation_failure()
            .expect_err("retained-store initialization cannot be an operation failure");
        assert!(matches!(
            mismatched_error,
            ImportRunError::Initialization {
                source: ImportInitializationError::RetainedStore { code }
            } if code == CODE
        ));
    }
}
