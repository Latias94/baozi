use std::cell::{RefCell, RefMut};
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};

use baozi_core::__canonical_import::{
    PendingSceneDraftOwner, PendingSceneFailureOwner, PreparedSceneConstruction,
    SceneActivationError, SceneActivationRequest, SceneActivationRoles, SceneConstructionTerminal,
    SceneConstructionTerminalParts, SceneDraft, SceneDraftBuild, SceneMinimalFallback,
    SceneRichFailure, SceneSuccessReport, SceneValidationLimits, SceneWriterRoles,
    ValidatedScene as CanonicalValidatedScene, maximum_scene_canonical_blocks,
    maximum_scene_structural_work, prepare_unactivated_scene, scene_failure_evidence_field_work,
    scene_failure_evidence_member_count, scene_failure_terminal_work_requirement,
};
use baozi_operation::__canonical_activation::{
    CanonicalValidationProgress, RetainedStoreDimensionState, RetainedStoreObservedWork,
};
use baozi_operation::{
    ControlCause, DimensionPolicy, FinalProgress, LedgerCause, LedgerLimits, LedgerLimitsError,
    PhysicalStorageAccount, RetainedStoreBatchLimits, RetainedStoreFailureTerminalWorkPlan,
    RetainedStoreOperation, RetainedStoreReservation, RetainedStoreSharePlan,
    RetainedStoreStartError, RetainedStoreStartPlan, RetainedStoreWriterPlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreDimensionSnapshot, StoreFallbackEnvelope, StoreKeyDomain,
    StoreKeyDomainCreateError, StoreLedgerEvidence, StoreLedgerPlan, StoreLedgerPlanError,
    StorePhaseKey, StorePhaseSemanticId, StorePhaseSnapshot, StorePlanError,
    StorePreparationFailureClass, StoreRuntimeError, StoreSemanticCatalogId, StoreTerminalCause,
};

use crate::error::ScopeAbortCause;
use crate::ledger::{
    ImportDimension, ImportLedgerPhase, ImportOperationControl, ImportOperationOwner,
    ImportProgressPhase, ImportProgressUnit, import_ledger_limits,
};
use crate::limits::ResourceLimits;

const IMPORT_STORE_CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-import-led", NonZeroU32::MIN);
const IMPORT_DIMENSION_COUNT: usize = ImportDimension::DecompressedBytes as usize + 1;
const IMPORT_STORE_FALLBACK_CODE: u32 = 0x494d_0001;
const IMPORT_SCENE_NOT_SUBMITTED_CODE: u32 = 0x494d_0002;
const MIN_CANONICAL_RETAINED_MEMBER_LIMIT: u64 = 1;
const STARTUP_OUTCOME_EVIDENCE_AMOUNT: u64 = 1;
const STARTUP_OUTCOME_EVIDENCE_TOTAL: u64 = STARTUP_OUTCOME_EVIDENCE_AMOUNT * 2;
const IMPORT_STORE_TERMINAL_DIMENSION_COUNT: usize = 4;
const IMPORT_STORE_EVIDENCE_DIMENSION_COUNT: usize =
    IMPORT_DIMENSION_COUNT + IMPORT_STORE_TERMINAL_DIMENSION_COUNT;

type ImportStoreOperation =
    RetainedStoreOperation<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type ImportSceneDraftBuild =
    SceneDraftBuild<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type ImportPendingSceneDraftOwner =
    PendingSceneDraftOwner<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type ImportPendingSceneFailureOwner =
    PendingSceneFailureOwner<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

/// Import-owned terminal classification copied from the unique ledger header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreTerminalCause {
    Success,
    Failure {
        class: ImportStoreTerminalFailureClass,
        code: u32,
    },
}

impl ImportStoreTerminalCause {
    pub(crate) const fn class(self) -> ImportStoreTerminalClass {
        match self {
            Self::Success => ImportStoreTerminalClass::Success,
            Self::Failure { class, .. } => match class {
                ImportStoreTerminalFailureClass::Requested => ImportStoreTerminalClass::Requested,
                ImportStoreTerminalFailureClass::Control => ImportStoreTerminalClass::Control,
                ImportStoreTerminalFailureClass::ProductGraph => {
                    ImportStoreTerminalClass::ProductGraph
                }
                ImportStoreTerminalFailureClass::Ledger => ImportStoreTerminalClass::Ledger,
            },
        }
    }

    #[cfg(test)]
    pub(crate) const fn code(self) -> Option<u32> {
        match self {
            Self::Success => None,
            Self::Failure { code, .. } => Some(code),
        }
    }
}

/// Import-owned flattened terminal class for reporting and outcome projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreTerminalClass {
    Success,
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

/// Import-owned failure class copied from the ledger's first terminal cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreTerminalFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

/// Import-owned identity for the terminal-only retained-store dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreTerminalDimension {
    StartupOutcomeEvidence,
    FailureTerminalWork,
    CanonicalRetainedMembers,
    CanonicalStructuralWork,
}

impl ImportStoreTerminalDimension {
    const ALL: [Self; IMPORT_STORE_TERMINAL_DIMENSION_COUNT] = [
        Self::StartupOutcomeEvidence,
        Self::FailureTerminalWork,
        Self::CanonicalRetainedMembers,
        Self::CanonicalStructuralWork,
    ];

    const fn semantic_id(self) -> StoreDimensionSemanticId {
        match self {
            Self::StartupOutcomeEvidence => STARTUP_OUTCOME_EVIDENCE_SEMANTIC,
            Self::FailureTerminalWork => FAILURE_TERMINAL_WORK_SEMANTIC,
            Self::CanonicalRetainedMembers => CANONICAL_RETAINED_MEMBERS_SEMANTIC,
            Self::CanonicalStructuralWork => CANONICAL_STRUCTURAL_WORK_SEMANTIC,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::StartupOutcomeEvidence => 0,
            Self::FailureTerminalWork => 1,
            Self::CanonicalRetainedMembers => 2,
            Self::CanonicalStructuralWork => 3,
        }
    }
}

/// Import-owned copied accounting for one terminal phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStorePhaseEvidence {
    phase: ImportLedgerPhase,
    current_live_bytes: u64,
    peak_live_bytes: u64,
}

impl ImportStorePhaseEvidence {
    const fn empty(phase: ImportLedgerPhase) -> Self {
        Self {
            phase,
            current_live_bytes: 0,
            peak_live_bytes: 0,
        }
    }

    fn from_store(phase: ImportLedgerPhase, snapshot: StorePhaseSnapshot) -> Self {
        Self {
            phase,
            current_live_bytes: snapshot.current_live_bytes(),
            peak_live_bytes: snapshot.peak_live_bytes(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn phase(self) -> ImportLedgerPhase {
        self.phase
    }

    #[cfg(test)]
    pub(crate) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }
}

/// Import-owned copied accounting for one producer-visible import dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStoreDimensionEvidence {
    dimension: ImportDimension,
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

impl ImportStoreDimensionEvidence {
    const fn empty(dimension: ImportDimension) -> Self {
        Self {
            dimension,
            attempted: 0,
            consumed: 0,
            current_live_bytes: 0,
            peak_live_bytes: 0,
            pending_publication: 0,
            committed: 0,
            committed_canonical_output: 0,
            published_outcome_evidence: 0,
        }
    }

    fn from_store(dimension: ImportDimension, snapshot: StoreDimensionSnapshot) -> Self {
        Self {
            dimension,
            attempted: snapshot.attempted(),
            consumed: snapshot.consumed(),
            current_live_bytes: snapshot.current_live_bytes(),
            peak_live_bytes: snapshot.peak_live_bytes(),
            pending_publication: snapshot.pending_publication(),
            committed: snapshot.committed(),
            committed_canonical_output: snapshot.committed_canonical_output(),
            published_outcome_evidence: snapshot.published_outcome_evidence(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn dimension(self) -> ImportDimension {
        self.dimension
    }

    pub(crate) const fn attempted(self) -> u64 {
        self.attempted
    }

    pub(crate) const fn consumed(self) -> u64 {
        self.consumed
    }

    pub(crate) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    pub(crate) const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    #[cfg(test)]
    pub(crate) const fn pending_publication(self) -> u64 {
        self.pending_publication
    }

    pub(crate) const fn committed(self) -> u64 {
        self.committed
    }

    pub(crate) const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

/// Import-owned copied accounting for one terminal-only dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStoreTerminalDimensionEvidence {
    dimension: ImportStoreTerminalDimension,
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

impl ImportStoreTerminalDimensionEvidence {
    const fn empty(dimension: ImportStoreTerminalDimension) -> Self {
        Self {
            dimension,
            attempted: 0,
            consumed: 0,
            current_live_bytes: 0,
            peak_live_bytes: 0,
            pending_publication: 0,
            committed: 0,
            committed_canonical_output: 0,
            published_outcome_evidence: 0,
        }
    }

    fn from_store(
        dimension: ImportStoreTerminalDimension,
        snapshot: StoreDimensionSnapshot,
    ) -> Self {
        Self {
            dimension,
            attempted: snapshot.attempted(),
            consumed: snapshot.consumed(),
            current_live_bytes: snapshot.current_live_bytes(),
            peak_live_bytes: snapshot.peak_live_bytes(),
            pending_publication: snapshot.pending_publication(),
            committed: snapshot.committed(),
            committed_canonical_output: snapshot.committed_canonical_output(),
            published_outcome_evidence: snapshot.published_outcome_evidence(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn dimension(self) -> ImportStoreTerminalDimension {
        self.dimension
    }

    #[cfg(test)]
    pub(crate) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    #[cfg(test)]
    pub(crate) const fn pending_publication(self) -> u64 {
        self.pending_publication
    }

    pub(crate) const fn committed_canonical_output(self) -> u64 {
        self.committed_canonical_output
    }

    #[cfg(test)]
    pub(crate) const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

/// Import-owned immutable projection of one real canonical Scene terminal.
///
/// No retained-store key, domain, snapshot, or ledger owner escapes this value. Every cause is
/// copied exclusively from the frozen ledger header and every accounting row is resolved through
/// the complete import semantic catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportStoreTerminalEvidence {
    schema_revision: u32,
    cause: ImportStoreTerminalCause,
    last_completed_phase: Option<ImportLedgerPhase>,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
    phases: [ImportStorePhaseEvidence; ImportLedgerPhase::ALL.len()],
    dimensions: [ImportStoreDimensionEvidence; IMPORT_DIMENSION_COUNT],
    terminal_dimensions:
        [ImportStoreTerminalDimensionEvidence; IMPORT_STORE_TERMINAL_DIMENSION_COUNT],
}

impl ImportStoreTerminalEvidence {
    pub(crate) fn try_from_terminal(
        terminal: &SceneConstructionTerminal,
    ) -> Result<Self, ImportStoreTerminalProjectionError> {
        Self::try_from_ledger(terminal.ledger_evidence())
    }

    fn try_from_ledger(
        evidence: &StoreLedgerEvidence,
    ) -> Result<Self, ImportStoreTerminalProjectionError> {
        let header = evidence.header();
        if header.phase_count() != ImportLedgerPhase::ALL.len() {
            return Err(ImportStoreTerminalProjectionError::UnexpectedPhaseCount {
                actual: header.phase_count(),
                expected: ImportLedgerPhase::ALL.len(),
            });
        }
        if header.dimension_count() != IMPORT_STORE_EVIDENCE_DIMENSION_COUNT {
            return Err(
                ImportStoreTerminalProjectionError::UnexpectedDimensionCount {
                    actual: header.dimension_count(),
                    expected: IMPORT_STORE_EVIDENCE_DIMENSION_COUNT,
                },
            );
        }
        if header.current_live_bytes() != 0 || header.active_reservations() != 0 {
            return Err(
                ImportStoreTerminalProjectionError::UnreleasedTerminalState {
                    current_live_bytes: header.current_live_bytes(),
                    active_reservations: header.active_reservations(),
                },
            );
        }

        let cause = import_terminal_cause(header.terminal_cause())?;
        let last_completed_phase = header
            .last_completed_phase()
            .map(import_phase_from_semantic)
            .transpose()?;
        let phases = project_import_phases(|semantic| evidence.phase_by_semantic(semantic))?;
        let dimensions =
            project_import_dimensions(|semantic| evidence.dimension_by_semantic(semantic))?;
        let terminal_dimensions = project_import_terminal_dimensions(|semantic| {
            evidence.dimension_by_semantic(semantic)
        })?;

        Ok(Self {
            schema_revision: header.schema_revision(),
            cause,
            last_completed_phase,
            current_live_bytes: header.current_live_bytes(),
            peak_live_bytes: header.peak_live_bytes(),
            active_reservations: header.active_reservations(),
            current_evidence_bytes: header.current_evidence_bytes(),
            peak_evidence_bytes: header.peak_evidence_bytes(),
            phases,
            dimensions,
            terminal_dimensions,
        })
    }

    pub(crate) const fn cause(&self) -> ImportStoreTerminalCause {
        self.cause
    }

    pub(crate) const fn last_completed_phase(&self) -> Option<ImportLedgerPhase> {
        self.last_completed_phase
    }

    pub(crate) const fn current_live_bytes(&self) -> u64 {
        self.current_live_bytes
    }

    pub(crate) const fn peak_live_bytes(&self) -> u64 {
        self.peak_live_bytes
    }

    pub(crate) const fn active_reservations(&self) -> u64 {
        self.active_reservations
    }

    pub(crate) const fn current_evidence_bytes(&self) -> u64 {
        self.current_evidence_bytes
    }

    pub(crate) const fn peak_evidence_bytes(&self) -> u64 {
        self.peak_evidence_bytes
    }

    #[cfg(test)]
    pub(crate) const fn phases(&self) -> &[ImportStorePhaseEvidence] {
        &self.phases
    }

    #[cfg(test)]
    pub(crate) const fn dimensions(&self) -> &[ImportStoreDimensionEvidence] {
        &self.dimensions
    }

    #[cfg(test)]
    pub(crate) const fn terminal_dimensions(&self) -> &[ImportStoreTerminalDimensionEvidence] {
        &self.terminal_dimensions
    }

    #[cfg(test)]
    pub(crate) fn phase(&self, phase: ImportLedgerPhase) -> Option<ImportStorePhaseEvidence> {
        self.phases.get(phase_index(phase)).copied()
    }

    pub(crate) fn dimension(
        &self,
        dimension: ImportDimension,
    ) -> Option<ImportStoreDimensionEvidence> {
        self.dimensions.get(dimension as usize).copied()
    }

    #[cfg(test)]
    pub(crate) fn terminal_dimension(
        &self,
        dimension: ImportStoreTerminalDimension,
    ) -> Option<ImportStoreTerminalDimensionEvidence> {
        self.terminal_dimensions.get(dimension.index()).copied()
    }

    pub(crate) const fn committed_canonical_members(&self) -> u64 {
        self.terminal_dimensions[ImportStoreTerminalDimension::CanonicalRetainedMembers.index()]
            .committed_canonical_output()
    }
}

/// Exact retained owners and import-semantic evidence produced by one terminal handoff.
#[must_use = "the terminal outcome owns the published Scene or failure evidence"]
#[allow(
    clippy::large_enum_variant,
    reason = "published owners remain inline so terminal handoff is a pure ownership move"
)]
pub(crate) enum ImportStoreTerminalOutcome {
    Success {
        scene: CanonicalValidatedScene,
        report: SceneSuccessReport,
        evidence: ImportStoreTerminalEvidence,
    },
    RichFailure {
        failure: SceneRichFailure,
        evidence: ImportStoreTerminalEvidence,
        control_cause: Option<ControlCause>,
        scene_failure_submitted: bool,
    },
    MinimalFallback {
        fallback: SceneMinimalFallback,
        evidence: ImportStoreTerminalEvidence,
        control_cause: Option<ControlCause>,
        scene_failure_submitted: bool,
    },
}

impl ImportStoreTerminalOutcome {
    fn try_from_terminal(
        terminal: SceneConstructionTerminal,
        scene_submitted: bool,
        scene_failure_submitted: bool,
    ) -> Result<Self, ImportStoreTerminalProjectionError> {
        let control_cause = terminal.control_cause().cloned();
        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)?;
        if control_cause.is_some() && evidence.cause().class() != ImportStoreTerminalClass::Control
        {
            return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
        }
        match terminal.into_parts() {
            SceneConstructionTerminalParts::Success { scene, report } => {
                if !scene_submitted || evidence.cause() != ImportStoreTerminalCause::Success {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(Self::Success {
                    scene,
                    report,
                    evidence,
                })
            }
            SceneConstructionTerminalParts::RichFailure(failure) => {
                if evidence.cause() == ImportStoreTerminalCause::Success {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(Self::RichFailure {
                    failure,
                    evidence,
                    control_cause,
                    scene_failure_submitted,
                })
            }
            SceneConstructionTerminalParts::MinimalFallback(fallback) => {
                if evidence.cause() == ImportStoreTerminalCause::Success {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(Self::MinimalFallback {
                    fallback,
                    evidence,
                    control_cause,
                    scene_failure_submitted,
                })
            }
        }
    }
}

fn import_terminal_cause(
    cause: StoreTerminalCause,
) -> Result<ImportStoreTerminalCause, ImportStoreTerminalProjectionError> {
    match cause {
        StoreTerminalCause::Success => Ok(ImportStoreTerminalCause::Success),
        StoreTerminalCause::Failure(cause) => {
            let class = import_terminal_failure_class(cause.class())?;
            Ok(ImportStoreTerminalCause::Failure {
                class,
                code: cause.code(),
            })
        }
        _ => Err(ImportStoreTerminalProjectionError::UnsupportedTerminalCause),
    }
}

fn import_terminal_failure_class(
    class: StorePreparationFailureClass,
) -> Result<ImportStoreTerminalFailureClass, ImportStoreTerminalProjectionError> {
    match class {
        StorePreparationFailureClass::Requested => Ok(ImportStoreTerminalFailureClass::Requested),
        StorePreparationFailureClass::Control => Ok(ImportStoreTerminalFailureClass::Control),
        StorePreparationFailureClass::ProductGraph => {
            Ok(ImportStoreTerminalFailureClass::ProductGraph)
        }
        StorePreparationFailureClass::Ledger => Ok(ImportStoreTerminalFailureClass::Ledger),
        _ => Err(ImportStoreTerminalProjectionError::UnsupportedTerminalFailureClass),
    }
}

fn import_phase_from_semantic(
    semantic: StorePhaseSemanticId,
) -> Result<ImportLedgerPhase, ImportStoreTerminalProjectionError> {
    ImportLedgerPhase::ALL
        .into_iter()
        .find(|phase| import_phase_semantic_id(*phase) == semantic)
        .ok_or(ImportStoreTerminalProjectionError::UnknownLastCompletedPhase)
}

fn project_import_phases(
    mut lookup: impl FnMut(StorePhaseSemanticId) -> Option<StorePhaseSnapshot>,
) -> Result<
    [ImportStorePhaseEvidence; ImportLedgerPhase::ALL.len()],
    ImportStoreTerminalProjectionError,
> {
    let mut projected = ImportLedgerPhase::ALL.map(ImportStorePhaseEvidence::empty);
    for (index, phase) in ImportLedgerPhase::ALL.into_iter().enumerate() {
        let snapshot = lookup(import_phase_semantic_id(phase))
            .ok_or(ImportStoreTerminalProjectionError::MissingPhase { phase })?;
        projected[index] = ImportStorePhaseEvidence::from_store(phase, snapshot);
    }
    Ok(projected)
}

fn project_import_dimensions(
    mut lookup: impl FnMut(StoreDimensionSemanticId) -> Option<StoreDimensionSnapshot>,
) -> Result<
    [ImportStoreDimensionEvidence; IMPORT_DIMENSION_COUNT],
    ImportStoreTerminalProjectionError,
> {
    let mut projected = ImportDimension::ALL.map(ImportStoreDimensionEvidence::empty);
    for (index, dimension) in ImportDimension::ALL.into_iter().enumerate() {
        let snapshot = lookup(import_dimension_semantic_id(dimension))
            .ok_or(ImportStoreTerminalProjectionError::MissingDimension { dimension })?;
        projected[index] = ImportStoreDimensionEvidence::from_store(dimension, snapshot);
    }
    Ok(projected)
}

fn project_import_terminal_dimensions(
    mut lookup: impl FnMut(StoreDimensionSemanticId) -> Option<StoreDimensionSnapshot>,
) -> Result<
    [ImportStoreTerminalDimensionEvidence; IMPORT_STORE_TERMINAL_DIMENSION_COUNT],
    ImportStoreTerminalProjectionError,
> {
    let mut projected =
        ImportStoreTerminalDimension::ALL.map(ImportStoreTerminalDimensionEvidence::empty);
    for (index, dimension) in ImportStoreTerminalDimension::ALL.into_iter().enumerate() {
        let snapshot = lookup(dimension.semantic_id())
            .ok_or(ImportStoreTerminalProjectionError::MissingTerminalDimension { dimension })?;
        projected[index] = ImportStoreTerminalDimensionEvidence::from_store(dimension, snapshot);
    }
    Ok(projected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreTerminalProjectionError {
    UnexpectedPhaseCount {
        actual: usize,
        expected: usize,
    },
    UnexpectedDimensionCount {
        actual: usize,
        expected: usize,
    },
    UnreleasedTerminalState {
        current_live_bytes: u64,
        active_reservations: u64,
    },
    UnsupportedTerminalCause,
    UnsupportedTerminalFailureClass,
    TerminalOwnerCauseMismatch,
    UnknownLastCompletedPhase,
    MissingPhase {
        phase: ImportLedgerPhase,
    },
    MissingDimension {
        dimension: ImportDimension,
    },
    MissingTerminalDimension {
        dimension: ImportStoreTerminalDimension,
    },
}

impl fmt::Display for ImportStoreTerminalProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedPhaseCount { actual, expected } => write!(
                formatter,
                "terminal store evidence contains {actual} phases; expected {expected}"
            ),
            Self::UnexpectedDimensionCount { actual, expected } => write!(
                formatter,
                "terminal store evidence contains {actual} dimensions; expected {expected}"
            ),
            Self::UnreleasedTerminalState {
                current_live_bytes,
                active_reservations,
            } => write!(
                formatter,
                "terminal store evidence retains {current_live_bytes} live bytes across {active_reservations} active reservations"
            ),
            Self::UnsupportedTerminalCause => {
                formatter.write_str("terminal store evidence has an unsupported terminal cause")
            }
            Self::UnsupportedTerminalFailureClass => formatter
                .write_str("terminal store evidence has an unsupported terminal failure class"),
            Self::TerminalOwnerCauseMismatch => formatter.write_str(
                "terminal store ownership does not match the unique ledger terminal cause",
            ),
            Self::UnknownLastCompletedPhase => {
                formatter.write_str("terminal store evidence has an unknown last-completed phase")
            }
            Self::MissingPhase { phase } => {
                write!(
                    formatter,
                    "terminal store evidence is missing phase {phase:?}"
                )
            }
            Self::MissingDimension { dimension } => write!(
                formatter,
                "terminal store evidence is missing dimension {dimension:?}"
            ),
            Self::MissingTerminalDimension { dimension } => write!(
                formatter,
                "terminal store evidence is missing terminal dimension {dimension:?}"
            ),
        }
    }
}

impl std::error::Error for ImportStoreTerminalProjectionError {}

/// Import-owned view of one retained-store reservation.
///
/// The raw operation token never crosses this adapter. Import code may observe only the exact
/// byte envelope it asked the sole session owner to retain.
pub(crate) struct ImportStoreReservation {
    raw: RetainedStoreReservation<ImportOperationOwner>,
    bytes: u64,
}

impl fmt::Debug for ImportStoreReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportStoreReservation")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl ImportStoreReservation {
    pub(crate) const fn bytes(&self) -> u64 {
        self.bytes
    }
}

const STARTUP_OUTCOME_EVIDENCE_SEMANTIC: StoreDimensionSemanticId = StoreDimensionSemanticId::new(
    IMPORT_STORE_CATALOG,
    NonZeroU64::new(IMPORT_DIMENSION_COUNT as u64 + 1)
        .expect("the startup outcome-evidence semantic id is nonzero"),
);
const FAILURE_TERMINAL_WORK_SEMANTIC: StoreDimensionSemanticId = StoreDimensionSemanticId::new(
    IMPORT_STORE_CATALOG,
    NonZeroU64::new(IMPORT_DIMENSION_COUNT as u64 + 2)
        .expect("the failure-terminal semantic id is nonzero"),
);
const CANONICAL_RETAINED_MEMBERS_SEMANTIC: StoreDimensionSemanticId = StoreDimensionSemanticId::new(
    IMPORT_STORE_CATALOG,
    NonZeroU64::new(IMPORT_DIMENSION_COUNT as u64 + 3)
        .expect("the canonical retained-members semantic id is nonzero"),
);
const CANONICAL_STRUCTURAL_WORK_SEMANTIC: StoreDimensionSemanticId = StoreDimensionSemanticId::new(
    IMPORT_STORE_CATALOG,
    NonZeroU64::new(IMPORT_DIMENSION_COUNT as u64 + 4)
        .expect("the canonical structural-work semantic id is nonzero"),
);

/// Exact retained-start ceilings supplied by the import host.
///
/// Canonical shape and semantic work stay outside this vocabulary adapter. The later core sealed
/// plan owns those facts; bootstrap receives only the operation-native terminal capacities that
/// must exist before any import work begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStoreBootstrapRequirements {
    terminal_writer_limits: RetainedStoreBatchLimits,
    max_failure_evidence_members: usize,
    work: ImportStoreWorkBudget,
    canonical_structural_work_limit: NonZeroU64,
    canonical_retained_member_limit: NonZeroU64,
    published_outcome_evidence_limit: u64,
}

/// Caller-owned total work cap and its protected rich-failure subdivision.
///
/// The attempted and consumed totals include the protected terminal allowance. Bootstrap rejects
/// unless every ordinary import dimension exactly fills the checked remainder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStoreWorkBudget {
    total_attempted_work_limit: u64,
    total_consumed_work_limit: u64,
    failure_construction_work_limit: u64,
    failure_validation_work_limit: u64,
}

impl ImportStoreWorkBudget {
    pub(crate) const fn new(
        total_attempted_work_limit: u64,
        total_consumed_work_limit: u64,
        failure_construction_work_limit: u64,
        failure_validation_work_limit: u64,
    ) -> Self {
        Self {
            total_attempted_work_limit,
            total_consumed_work_limit,
            failure_construction_work_limit,
            failure_validation_work_limit,
        }
    }
}

impl ImportStoreBootstrapRequirements {
    pub(crate) fn new(
        terminal_attribution_slots: usize,
        terminal_commitment_slots: usize,
        max_failure_evidence_members: usize,
        work: ImportStoreWorkBudget,
        canonical_structural_work_limit: u64,
        canonical_retained_member_limit: u64,
        published_outcome_evidence_limit: u64,
    ) -> Result<Self, ImportStoreBootstrapError> {
        let canonical_structural_work_limit =
            NonZeroU64::new(canonical_structural_work_limit).ok_or(StorePlanError::EmptyPayload)?;
        let canonical_retained_member_limit = NonZeroU64::new(canonical_retained_member_limit)
            .ok_or(
                ImportStoreBootstrapError::PublishedCanonicalRetainedMemberLimitTooSmall {
                    limit: canonical_retained_member_limit,
                    required: MIN_CANONICAL_RETAINED_MEMBER_LIMIT,
                },
            )?;
        if published_outcome_evidence_limit < STARTUP_OUTCOME_EVIDENCE_TOTAL {
            return Err(
                ImportStoreBootstrapError::PublishedOutcomeEvidenceLimitTooSmall {
                    limit: published_outcome_evidence_limit,
                    required: STARTUP_OUTCOME_EVIDENCE_TOTAL,
                },
            );
        }
        Ok(Self {
            terminal_writer_limits: RetainedStoreBatchLimits::new(
                terminal_attribution_slots,
                terminal_commitment_slots,
            )?,
            max_failure_evidence_members,
            work,
            canonical_structural_work_limit,
            canonical_retained_member_limit,
            published_outcome_evidence_limit,
        })
    }

    const fn canonical_retained_member_policy(&self) -> DimensionPolicy {
        DimensionPolicy::counter(0, 0, self.canonical_retained_member_limit.get())
    }

    const fn canonical_structural_work_policy(&self) -> DimensionPolicy {
        DimensionPolicy::counter(
            self.canonical_structural_work_limit.get(),
            self.canonical_structural_work_limit.get(),
            0,
        )
    }

    pub(crate) fn for_node_scene(
        limits: &ResourceLimits,
    ) -> Result<Self, ImportStoreBootstrapError> {
        let validation_limits = scene_validation_limits(limits);
        let terminal_attribution_slots = scene_failure_evidence_member_count()
            .checked_add(1)
            .ok_or(StorePlanError::SizeOverflow)?;
        let canonical_retained_member_limit = u64::try_from(
            maximum_scene_canonical_blocks(validation_limits)
                .ok_or(StorePlanError::SizeOverflow)?,
        )
        .map_err(|_| StorePlanError::SizeOverflow)?;
        let canonical_structural_work_limit =
            maximum_scene_structural_work(validation_limits).ok_or(StorePlanError::SizeOverflow)?;
        let import_limits =
            import_ledger_limits(limits).map_err(ImportStoreBootstrapError::ImportLedgerLimits)?;
        let (ordinary_attempted, ordinary_consumed) = ImportDimension::ALL.into_iter().try_fold(
            (0_u64, 0_u64),
            |(attempted, consumed), dimension| {
                let policy = import_limits
                    .dimension(dimension)
                    .ok_or(ImportStoreBootstrapError::MissingImportDimensionPolicy { dimension })?;
                Ok::<_, ImportStoreBootstrapError>((
                    attempted
                        .checked_add(policy.attempted_limit())
                        .ok_or(StorePlanError::SizeOverflow)?,
                    consumed
                        .checked_add(policy.consumed_limit())
                        .ok_or(StorePlanError::SizeOverflow)?,
                ))
            },
        )?;
        let protected = scene_failure_terminal_work_requirement()?;
        let evidence_work = scene_failure_evidence_field_work();
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted
                .checked_add(canonical_structural_work_limit)
                .ok_or(StorePlanError::SizeOverflow)?
                .checked_add(protected.attempted_work_limit())
                .ok_or(StorePlanError::SizeOverflow)?,
            ordinary_consumed
                .checked_add(canonical_structural_work_limit)
                .ok_or(StorePlanError::SizeOverflow)?
                .checked_add(protected.consumed_work_limit())
                .ok_or(StorePlanError::SizeOverflow)?,
            evidence_work,
            evidence_work,
        );
        Self::new(
            terminal_attribution_slots,
            4,
            scene_failure_evidence_member_count(),
            work,
            canonical_structural_work_limit,
            canonical_retained_member_limit,
            STARTUP_OUTCOME_EVIDENCE_TOTAL,
        )
    }
}

/// Complete host-minted mapping from import semantics to one operation retained-store domain.
///
/// The process-local keys are crate-private and never become stable identity. The matching
/// [`StoreKeyDomain`] is consumed into the bootstrap's [`StoreLedgerPlan`], which retains the
/// authority anchor for the operation lifetime.
pub(crate) struct ImportStoreVocabulary {
    phases: [(ImportLedgerPhase, StorePhaseKey); ImportLedgerPhase::ALL.len()],
    dimensions: [(ImportDimension, StoreDimensionKey); IMPORT_DIMENSION_COUNT],
    startup_outcome_evidence: StoreDimensionKey,
    failure_terminal_work: StoreDimensionKey,
    canonical_retained_members: StoreDimensionKey,
    canonical_structural_work: StoreDimensionKey,
}

impl ImportStoreVocabulary {
    fn try_admit(domain: &StoreKeyDomain) -> Result<Self, ImportStoreBootstrapError> {
        let phases = [
            admit_phase(domain, ImportLedgerPhase::PrimarySnapshot)?,
            admit_phase(domain, ImportLedgerPhase::Detection)?,
            admit_phase(domain, ImportLedgerPhase::DependencyAcquisition)?,
            admit_phase(domain, ImportLedgerPhase::BackendDecode)?,
            admit_phase(domain, ImportLedgerPhase::FormatNormalization)?,
            admit_phase(domain, ImportLedgerPhase::SceneConstruction)?,
            admit_phase(domain, ImportLedgerPhase::Validation)?,
            admit_phase(domain, ImportLedgerPhase::OutcomeFinalization)?,
        ];
        let dimensions = [
            admit_dimension(domain, ImportDimension::AssetOpen)?,
            admit_dimension(domain, ImportDimension::DependencyReference)?,
            admit_dimension(domain, ImportDimension::PrimaryBytes)?,
            admit_dimension(domain, ImportDimension::DependencyBytes)?,
            admit_dimension(domain, ImportDimension::TotalAssetBytes)?,
            admit_dimension(domain, ImportDimension::DataUriBytes)?,
            admit_dimension(domain, ImportDimension::DiagnosticCount)?,
            admit_dimension(domain, ImportDimension::DiagnosticBytes)?,
            admit_dimension(domain, ImportDimension::DecodedTextBytes)?,
            admit_dimension(domain, ImportDimension::LineIndexBytes)?,
            admit_dimension(domain, ImportDimension::RecoveryCount)?,
            admit_dimension(domain, ImportDimension::RecoveryRawBytes)?,
            admit_dimension(domain, ImportDimension::LossEvidenceCount)?,
            admit_dimension(domain, ImportDimension::LossEvidenceBytes)?,
            admit_dimension(domain, ImportDimension::SourceScopeRecordCount)?,
            admit_dimension(domain, ImportDimension::SourceScopeItemCount)?,
            admit_dimension(domain, ImportDimension::SourceScopeDetailCount)?,
            admit_dimension(domain, ImportDimension::SourceScopeDetailBytes)?,
            admit_dimension(domain, ImportDimension::SourceScopeDigestBytes)?,
            admit_dimension(domain, ImportDimension::DetectionWork)?,
            admit_dimension(domain, ImportDimension::WorkUnits)?,
            admit_dimension(domain, ImportDimension::Tokens)?,
            admit_dimension(domain, ImportDimension::Records)?,
            admit_dimension(domain, ImportDimension::Meshes)?,
            admit_dimension(domain, ImportDimension::Vertices)?,
            admit_dimension(domain, ImportDimension::Faces)?,
            admit_dimension(domain, ImportDimension::GraphNodes)?,
            admit_dimension(domain, ImportDimension::GraphEdges)?,
            admit_dimension(domain, ImportDimension::GraphDepth)?,
            admit_dimension(domain, ImportDimension::GraphWork)?,
            admit_dimension(domain, ImportDimension::Primitives)?,
            admit_dimension(domain, ImportDimension::AttributeStreams)?,
            admit_dimension(domain, ImportDimension::Solids)?,
            admit_dimension(domain, ImportDimension::DecodeWork)?,
            admit_dimension(domain, ImportDimension::ScratchBytes)?,
            admit_dimension(domain, ImportDimension::CanonicalNodes)?,
            admit_dimension(domain, ImportDimension::CanonicalPrimitives)?,
            admit_dimension(domain, ImportDimension::AttributeCells)?,
            admit_dimension(domain, ImportDimension::TopologyCells)?,
            admit_dimension(domain, ImportDimension::InfluenceCells)?,
            admit_dimension(domain, ImportDimension::MorphCells)?,
            admit_dimension(domain, ImportDimension::AnimationCells)?,
            admit_dimension(domain, ImportDimension::CanonicalStringBytes)?,
            admit_dimension(domain, ImportDimension::CanonicalPayloadBytes)?,
            admit_dimension(domain, ImportDimension::CanonicalOutputBytes)?,
            admit_dimension(domain, ImportDimension::ArchiveBytes)?,
            admit_dimension(domain, ImportDimension::CompressedBytes)?,
            admit_dimension(domain, ImportDimension::DecompressedBytes)?,
        ];
        let startup_outcome_evidence = domain
            .try_admit_dimension(STARTUP_OUTCOME_EVIDENCE_SEMANTIC)
            .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
        let failure_terminal_work = domain
            .try_admit_dimension(FAILURE_TERMINAL_WORK_SEMANTIC)
            .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
        let canonical_retained_members = domain
            .try_admit_dimension(CANONICAL_RETAINED_MEMBERS_SEMANTIC)
            .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
        let canonical_structural_work = domain
            .try_admit_dimension(CANONICAL_STRUCTURAL_WORK_SEMANTIC)
            .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
        Ok(Self {
            phases,
            dimensions,
            startup_outcome_evidence,
            failure_terminal_work,
            canonical_retained_members,
            canonical_structural_work,
        })
    }

    pub(crate) fn phase(&self, phase: ImportLedgerPhase) -> StorePhaseKey {
        self.phases[phase_index(phase)].1
    }

    pub(crate) fn dimension(&self, dimension: ImportDimension) -> StoreDimensionKey {
        let index = dimension as usize;
        debug_assert_eq!(self.dimensions[index].0, dimension);
        self.dimensions[index].1
    }

    fn import_dimension(&self, key: StoreDimensionKey) -> Option<ImportDimension> {
        self.dimensions
            .iter()
            .find_map(|(dimension, candidate)| (*candidate == key).then_some(*dimension))
    }

    #[cfg(test)]
    pub(crate) const fn outcome_evidence_dimension(&self) -> StoreDimensionKey {
        self.startup_outcome_evidence
    }

    pub(crate) const fn canonical_retained_member_dimension(&self) -> StoreDimensionKey {
        self.canonical_retained_members
    }

    const fn canonical_structural_work_dimension(&self) -> StoreDimensionKey {
        self.canonical_structural_work
    }

    fn phase_keys(&self) -> [StorePhaseKey; ImportLedgerPhase::ALL.len()] {
        self.phases.map(|(_, key)| key)
    }

    fn activation_roles(&self) -> SceneActivationRoles {
        SceneActivationRoles::new(
            SceneWriterRoles::new(
                self.phase(ImportLedgerPhase::SceneConstruction),
                self.dimension(ImportDimension::CanonicalOutputBytes),
                self.dimension(ImportDimension::CanonicalOutputBytes),
            ),
            SceneWriterRoles::new(
                self.phase(ImportLedgerPhase::OutcomeFinalization),
                self.startup_outcome_evidence,
                self.startup_outcome_evidence,
            ),
            self.dimension(ImportDimension::WorkUnits),
            self.dimension(ImportDimension::ScratchBytes),
            self.phase(ImportLedgerPhase::Validation),
            self.dimension(ImportDimension::WorkUnits),
            self.dimension(ImportDimension::ScratchBytes),
            self.canonical_retained_members,
            self.startup_outcome_evidence,
        )
        .with_canonical_string_dimension(self.dimension(ImportDimension::CanonicalStringBytes))
    }
}

/// Plan-only import host bootstrap for one future retained-store operation.
///
/// This value creates neither an [`OperationCoordinator`](baozi_operation::OperationCoordinator)
/// nor a [`RetainedStoreOperation`](baozi_operation::RetainedStoreOperation). The session cutover
/// consumes these plans exactly once to start the sole operation ledger.
pub(crate) struct ImportStoreBootstrap {
    vocabulary: ImportStoreVocabulary,
    ledger_plan: StoreLedgerPlan,
    start_plan: RetainedStoreStartPlan,
}

impl ImportStoreBootstrap {
    pub(crate) fn into_parts(
        self,
    ) -> (
        ImportStoreVocabulary,
        StoreLedgerPlan,
        RetainedStoreStartPlan,
    ) {
        (self.vocabulary, self.ledger_plan, self.start_plan)
    }
}

/// Sole retained-store owner for one import session before canonical activation begins.
///
/// Producer-facing code borrows [`ImportStoreHost`] rather than the raw operation, store keys, or
/// physical account. The private state slot moves the operation exactly once into the later
/// scoped canonical outcome transaction while outstanding producer values retain this owner.
pub(crate) struct ImportStoreSessionOwner {
    state: RefCell<ImportStoreSessionState>,
    vocabulary: ImportStoreVocabulary,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the sole operation and pending terminal owners stay inline so fail-closed transitions do not allocate"
)]
enum ImportStoreSessionState {
    Active(ImportStoreOperation),
    Transitioning,
    Pending(ImportPendingScene),
    Contained,
}

fn inactive_store_state_error(state: &ImportStoreSessionState) -> ImportStoreHostError {
    match state {
        ImportStoreSessionState::Contained => ImportStoreHostError::OperationContained,
        ImportStoreSessionState::Active(_)
        | ImportStoreSessionState::Transitioning
        | ImportStoreSessionState::Pending(_) => ImportStoreHostError::OperationConsumed,
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "pending retained owners stay inline so descriptor rejection remains allocation-free"
)]
enum ImportPendingScene {
    Draft(ImportPendingSceneDraftOwner),
    Failure {
        draft_built: bool,
        owner: ImportPendingSceneFailureOwner,
    },
}

struct ImportStorePreparedScene {
    prepared: PreparedSceneConstruction,
    scene_submitted: bool,
    scene_failure_submitted: bool,
}

impl ImportStorePreparedScene {
    fn handoff(self) -> SceneConstructionTerminal {
        self.prepared.handoff()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreSceneBoundary {
    DraftBuilt,
    Validated,
}

#[must_use = "the Scene activation transition must install a pending owner or become contained"]
struct ImportStoreActivationTransition<'session> {
    state: RefMut<'session, ImportStoreSessionState>,
    armed: bool,
}

impl<'session> ImportStoreActivationTransition<'session> {
    fn begin(
        state: &'session RefCell<ImportStoreSessionState>,
    ) -> Result<(ImportStoreOperation, Self), ImportStoreHostError> {
        let mut state = state
            .try_borrow_mut()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        let previous = std::mem::replace(&mut *state, ImportStoreSessionState::Transitioning);
        let operation = match previous {
            ImportStoreSessionState::Active(operation) => operation,
            ImportStoreSessionState::Contained => {
                *state = ImportStoreSessionState::Contained;
                return Err(ImportStoreHostError::OperationContained);
            }
            previous @ (ImportStoreSessionState::Transitioning
            | ImportStoreSessionState::Pending(_)) => {
                *state = previous;
                return Err(ImportStoreHostError::OperationConsumed);
            }
        };
        Ok((operation, Self { state, armed: true }))
    }

    fn install(mut self, pending: ImportPendingScene) {
        debug_assert!(matches!(
            *self.state,
            ImportStoreSessionState::Transitioning
        ));
        *self.state = ImportStoreSessionState::Pending(pending);
        self.armed = false;
    }

    fn contain(mut self) {
        *self.state = ImportStoreSessionState::Contained;
        self.armed = false;
    }
}

impl Drop for ImportStoreActivationTransition<'_> {
    fn drop(&mut self) {
        if self.armed {
            *self.state = ImportStoreSessionState::Contained;
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "startup failures preserve the complete linear retained-store recovery owner"
)]
impl ImportStoreSessionOwner {
    pub(crate) fn try_start(
        control: ImportOperationControl,
        account: &PhysicalStorageAccount,
        bootstrap: ImportStoreBootstrap,
    ) -> Result<Self, RetainedStoreStartError> {
        let (vocabulary, ledger_plan, start_plan) = bootstrap.into_parts();
        let operation = ImportStoreOperation::try_start(control, ledger_plan, account, start_plan)?;
        Ok(Self {
            state: RefCell::new(ImportStoreSessionState::Active(operation)),
            vocabulary,
        })
    }

    pub(crate) fn host(&self) -> ImportStoreHost<'_> {
        ImportStoreHost {
            state: &self.state,
            vocabulary: &self.vocabulary,
        }
    }

    pub(crate) fn poll_parser_boundary(&self) -> Result<(), ImportStoreHostError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        match &mut *state {
            ImportStoreSessionState::Active(operation) => operation
                .producer_host()
                .poll()
                .map_err(|cause| project_store_runtime_error(&self.vocabulary, cause)),
            ImportStoreSessionState::Pending(_) => Ok(()),
            ImportStoreSessionState::Transitioning => Err(ImportStoreHostError::OperationConsumed),
            ImportStoreSessionState::Contained => Err(ImportStoreHostError::OperationContained),
        }
    }

    pub(crate) fn completed_scene_boundary(
        &self,
    ) -> Result<Option<ImportStoreSceneBoundary>, ImportStoreHostError> {
        let state = self
            .state
            .try_borrow()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        Ok(match &*state {
            ImportStoreSessionState::Pending(ImportPendingScene::Draft(_)) => {
                Some(ImportStoreSceneBoundary::Validated)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::Failure {
                draft_built: true,
                ..
            }) => Some(ImportStoreSceneBoundary::DraftBuilt),
            ImportStoreSessionState::Active(_)
            | ImportStoreSessionState::Transitioning
            | ImportStoreSessionState::Pending(ImportPendingScene::Failure {
                draft_built: false,
                ..
            })
            | ImportStoreSessionState::Contained => None,
        })
    }

    pub(crate) fn submit_scene<'input>(
        &self,
        request: SceneActivationRequest<'input>,
    ) -> Result<SceneDraft, ImportStoreActivationError> {
        let (operation, transition) = ImportStoreActivationTransition::begin(&self.state)
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
        let build = match request.activate(
            operation,
            self.vocabulary.activation_roles(),
            CanonicalValidationProgress::new(
                ImportProgressPhase::Validate,
                ImportProgressUnit::WorkUnits,
            ),
            FinalProgress::new(
                ImportProgressPhase::Finalize,
                ImportProgressUnit::CanonicalItems,
                1,
            ),
        ) {
            Ok(build) => build,
            Err(cause) => {
                transition.contain();
                return Err(ImportStoreActivationError::Scene(cause));
            }
        };
        let (pending, result) = match build {
            ImportSceneDraftBuild::Draft { draft, owner } => {
                (ImportPendingScene::Draft(owner), Ok(draft))
            }
            ImportSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built,
                control_cause,
                owner,
            } => {
                let result = if let Some(cause) = control_cause {
                    Err(ImportStoreActivationError::Control(cause))
                } else if authoring_rejected {
                    Err(ImportStoreActivationError::AuthoringRejected)
                } else {
                    Err(ImportStoreActivationError::MaterializationFailed)
                };
                (ImportPendingScene::Failure { draft_built, owner }, result)
            }
        };
        transition.install(pending);
        result
    }

    fn prepare_scene(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
        let (prepared, scene_submitted, scene_failure_submitted) = match self.state.into_inner() {
            ImportStoreSessionState::Pending(ImportPendingScene::Draft(owner)) => (
                owner
                    .prepare(draft)
                    .map_err(ImportStoreActivationError::Scene)?,
                true,
                false,
            ),
            ImportStoreSessionState::Pending(ImportPendingScene::Failure { owner, .. }) => (
                owner.prepare().map_err(ImportStoreActivationError::Scene)?,
                true,
                true,
            ),
            ImportStoreSessionState::Active(operation) => (
                prepare_unactivated_scene(operation, IMPORT_SCENE_NOT_SUBMITTED_CODE)
                    .map_err(ImportStoreActivationError::Scene)?,
                false,
                false,
            ),
            ImportStoreSessionState::Transitioning | ImportStoreSessionState::Contained => {
                return Err(ImportStoreActivationError::StoreUnavailable(
                    ImportStoreHostError::OperationContained,
                ));
            }
        };
        Ok(ImportStorePreparedScene {
            prepared,
            scene_submitted,
            scene_failure_submitted,
        })
    }

    pub(crate) fn finish_scene(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<ImportStoreTerminalOutcome, ImportStoreFinalizationError> {
        let prepared = self
            .prepare_scene(draft)
            .map_err(ImportStoreFinalizationError::Activation)?;
        let scene_submitted = prepared.scene_submitted;
        let scene_failure_submitted = prepared.scene_failure_submitted;
        let terminal = prepared.handoff();
        ImportStoreTerminalOutcome::try_from_terminal(
            terminal,
            scene_submitted,
            scene_failure_submitted,
        )
        .map_err(ImportStoreFinalizationError::Projection)
    }
}

pub(crate) fn scene_validation_limits(limits: &ResourceLimits) -> SceneValidationLimits {
    let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
    // Format graph counters cover source topology only. Canonical validation also counts
    // profile, provenance, appearance, and deformation relations, so graph-specific limits
    // cannot be reused as the aggregate Scene edge/reference envelope. Every retained relation
    // consumes canonical payload bytes, making the payload-byte limit a conservative upper bound.
    let canonical_relations = limits.max_canonical_payload_bytes;
    let payload_cells = [
        limits.max_total_vertex_attribute_cells,
        limits.max_total_topology_cells,
        limits.max_influence_cells,
        limits.max_morph_cells,
        limits.max_animation_cells,
    ]
    .into_iter()
    .fold(0_u64, |total, value| total.saturating_add(count(value)));

    SceneValidationLimits::default()
        .with_max_nodes(count(limits.max_graph_nodes))
        .with_max_edges(canonical_relations)
        .with_max_depth(count(limits.max_graph_depth))
        .with_max_entities(limits.max_canonical_payload_bytes)
        .with_max_references(canonical_relations)
        .with_max_payload_cells(payload_cells)
        .with_max_work(limits.max_work_units)
        .with_max_metadata_entries(count(limits.max_records))
        .with_max_metadata_bytes(count(limits.max_canonical_string_bytes))
        .with_max_metadata_key_bytes(limits.max_string_bytes)
        .with_max_metadata_string_bytes(limits.max_canonical_string_bytes)
        .with_max_canonical_bytes(limits.max_canonical_payload_bytes)
        .with_max_scratch_bytes(limits.max_live_bytes)
}

#[derive(Debug)]
pub(crate) enum ImportStoreActivationError {
    StoreUnavailable(ImportStoreHostError),
    Control(ControlCause),
    AuthoringRejected,
    MaterializationFailed,
    Scene(SceneActivationError),
}

impl fmt::Display for ImportStoreActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreUnavailable(cause) => cause.fmt(formatter),
            Self::Control(cause) => cause.fmt(formatter),
            Self::AuthoringRejected => {
                formatter.write_str("canonical Scene authoring was rejected")
            }
            Self::MaterializationFailed => {
                formatter.write_str("canonical Scene materialization prepared a terminal failure")
            }
            Self::Scene(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportStoreActivationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StoreUnavailable(cause) => Some(cause),
            Self::Control(cause) => Some(cause),
            Self::AuthoringRejected | Self::MaterializationFailed => None,
            Self::Scene(cause) => Some(cause),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ImportStoreFinalizationError {
    Activation(ImportStoreActivationError),
    Projection(ImportStoreTerminalProjectionError),
}

impl fmt::Display for ImportStoreFinalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activation(cause) => cause.fmt(formatter),
            Self::Projection(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportStoreFinalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Activation(cause) => Some(cause),
            Self::Projection(cause) => Some(cause),
        }
    }
}

/// Borrowed import-semantic producer capability backed by the session's sole retained operation.
///
/// This adapter never exposes operation-native keys, publication authority, terminal dimensions,
/// or the physical account. Observed-work permits remain inside one closure so they cannot escape
/// the state borrow that owns the operation.
#[derive(Clone, Copy)]
pub(crate) struct ImportStoreHost<'session> {
    state: &'session RefCell<ImportStoreSessionState>,
    vocabulary: &'session ImportStoreVocabulary,
}

#[allow(
    clippy::result_large_err,
    reason = "producer failures retain exact typed store causes without allocating on error paths"
)]
impl ImportStoreHost<'_> {
    fn runtime_error(&self, cause: StoreRuntimeError) -> ImportStoreHostError {
        project_store_runtime_error(self.vocabulary, cause)
    }

    fn try_operation(&self) -> Result<RefMut<'_, ImportStoreOperation>, ImportStoreHostError> {
        let state = self
            .state
            .try_borrow_mut()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        RefMut::filter_map(state, |state| match state {
            ImportStoreSessionState::Active(operation) => Some(operation),
            ImportStoreSessionState::Transitioning
            | ImportStoreSessionState::Pending(_)
            | ImportStoreSessionState::Contained => None,
        })
        .map_err(|state| inactive_store_state_error(&state))
    }

    pub(crate) fn poll(&self) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .producer_host()
            .poll()
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn report_progress(
        &self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .producer_host()
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn dimension_state(
        &self,
        dimension: ImportDimension,
    ) -> Result<RetainedStoreDimensionState, ImportStoreHostError> {
        let key = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .producer_host()
            .dimension_state(key)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn reserve(
        &self,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ImportStoreHostError> {
        let phase = self.vocabulary.phase(phase);
        let dimension = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .control_checkpoint()
            .reserve(phase, dimension, bytes)
            .map(|raw| ImportStoreReservation { raw, bytes })
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn resize(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .control_checkpoint()
            .resize(&mut reservation.raw, bytes)
            .map_err(|cause| self.runtime_error(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn transition(
        &self,
        reservation: &mut ImportStoreReservation,
        phase: ImportLedgerPhase,
        dimension: ImportDimension,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        let phase = self.vocabulary.phase(phase);
        let dimension = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .control_checkpoint()
            .transition(&mut reservation.raw, phase, dimension, bytes)
            .map_err(|cause| self.runtime_error(cause))?;
        reservation.bytes = bytes;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn validate_live_owner(
        &self,
        reservation: &ImportStoreReservation,
    ) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .control_checkpoint()
            .validate_live_owner(&reservation.raw)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn tighten_dimension_policy(
        &self,
        dimension: ImportDimension,
        upper_bound: DimensionPolicy,
    ) -> Result<(), ImportStoreHostError> {
        let dimension = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .producer_host()
            .tighten_dimension_policy(dimension, upper_bound)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_attempted(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        let dimension = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .producer_host()
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_consumed(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> Result<(), ImportStoreHostError> {
        let dimension = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .producer_host()
            .charge_consumed(dimension, amount)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_observed_batch<const N: usize>(
        &self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        let charges =
            charges.map(|(dimension, amount)| (self.vocabulary.dimension(dimension), amount));
        self.try_operation()?
            .producer_host()
            .charge_observed_batch(charges)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &self,
        charges: &[(ImportDimension, u64); N],
    ) -> Result<(), ImportStoreHostError> {
        let mut charges =
            charges.map(|(dimension, amount)| (self.vocabulary.dimension(dimension), amount));
        charges.sort_unstable_by_key(|(dimension, _)| *dimension);
        self.try_operation()?
            .producer_host()
            .charge_consumed_batch(charges)
            .map_err(|cause| self.runtime_error(cause))
    }

    pub(crate) fn with_observed_work<const N: usize, T, E>(
        &self,
        charges: [(ImportDimension, u64); N],
        action: impl FnOnce(&mut ImportObservedWork<'_, N>) -> Result<T, E>,
    ) -> Result<T, ImportObservedWorkFailure<E>> {
        let charges =
            charges.map(|(dimension, amount)| (self.vocabulary.dimension(dimension), amount));
        let mut operation = self
            .try_operation()
            .map_err(ImportObservedWorkFailure::Store)?;
        let mut producer = operation.producer_host();
        let raw = producer
            .try_begin_observed_work(charges)
            .map_err(|cause| ImportObservedWorkFailure::Store(self.runtime_error(cause)))?;
        let mut work = ImportObservedWork {
            raw,
            vocabulary: self.vocabulary,
        };
        match action(&mut work) {
            Ok(value) => {
                work.finish().map_err(ImportObservedWorkFailure::Store)?;
                Ok(value)
            }
            Err(cause) => {
                work.finish_partial();
                Err(ImportObservedWorkFailure::Action(cause))
            }
        }
    }
}

pub(crate) struct ImportObservedWork<'operation, const N: usize> {
    raw: RetainedStoreObservedWork<
        'operation,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        N,
    >,
    vocabulary: &'operation ImportStoreVocabulary,
}

#[allow(
    clippy::result_large_err,
    reason = "observed-work errors preserve exact ledger and control causes without boxing"
)]
impl<const N: usize> ImportObservedWork<'_, N> {
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }

    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), ImportStoreHostError> {
        self.raw
            .complete(amounts)
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    fn finish(self) -> Result<(), ImportStoreHostError> {
        self.raw
            .finish()
            .map_err(|cause| project_store_runtime_error(self.vocabulary, cause))
    }

    fn finish_partial(self) {
        self.raw.finish_partial();
    }
}

#[derive(Debug)]
pub(crate) enum ImportStoreHostError {
    ReentrantBorrow,
    OperationConsumed,
    OperationContained,
    Control(ControlCause),
    Ledger(LedgerCause<ImportDimension>),
    Contract { code: &'static str },
}

impl ImportStoreHostError {
    pub(crate) fn into_scope_abort_cause(self) -> ScopeAbortCause {
        match self {
            Self::Control(cause) => ScopeAbortCause::Control(cause),
            Self::Ledger(cause) => ScopeAbortCause::Ledger(cause),
            Self::ReentrantBorrow => ScopeAbortCause::DescriptorContract {
                code: "import-store-host-reentered",
            },
            Self::OperationConsumed => ScopeAbortCause::DescriptorContract {
                code: "import-store-operation-consumed",
            },
            Self::OperationContained => ScopeAbortCause::DescriptorContract {
                code: "import-store-operation-contained",
            },
            Self::Contract { code } => ScopeAbortCause::DescriptorContract { code },
        }
    }
}

impl fmt::Display for ImportStoreHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReentrantBorrow => {
                formatter.write_str("the import retained-store host was borrowed reentrantly")
            }
            Self::OperationConsumed => {
                formatter.write_str("the import retained-store operation was already consumed")
            }
            Self::OperationContained => formatter
                .write_str("the import retained-store operation ended in fail-closed containment"),
            Self::Control(cause) => cause.fmt(formatter),
            Self::Ledger(cause) => cause.fmt(formatter),
            Self::Contract { code } => {
                write!(
                    formatter,
                    "the import retained-store contract failed: {code}"
                )
            }
        }
    }
}

impl std::error::Error for ImportStoreHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Control(cause) => Some(cause),
            Self::Ledger(cause) => Some(cause),
            Self::ReentrantBorrow
            | Self::OperationConsumed
            | Self::OperationContained
            | Self::Contract { .. } => None,
        }
    }
}

fn project_store_runtime_error(
    vocabulary: &ImportStoreVocabulary,
    cause: StoreRuntimeError,
) -> ImportStoreHostError {
    match cause {
        StoreRuntimeError::Control(cause) => ImportStoreHostError::Control(cause),
        StoreRuntimeError::Ledger(cause) => match project_store_ledger_cause(vocabulary, cause) {
            Some(cause) => ImportStoreHostError::Ledger(cause),
            None => ImportStoreHostError::Contract {
                code: "import-store-ledger-dimension-projection",
            },
        },
        StoreRuntimeError::ObservedWorkOutstanding => ImportStoreHostError::Contract {
            code: "import-store-observed-work-outstanding",
        },
        StoreRuntimeError::ObservedWorkExceedsAdmitted => ImportStoreHostError::Contract {
            code: "import-store-observed-work-overrun",
        },
        StoreRuntimeError::ObservedWorkIncomplete => ImportStoreHostError::Contract {
            code: "import-store-observed-work-incomplete",
        },
        _ => ImportStoreHostError::Contract {
            code: "import-store-runtime-contract",
        },
    }
}

fn project_store_ledger_cause(
    vocabulary: &ImportStoreVocabulary,
    cause: LedgerCause<StoreDimensionKey>,
) -> Option<LedgerCause<ImportDimension>> {
    let dimension = |key| vocabulary.import_dimension(key);
    Some(match cause {
        LedgerCause::UnknownDimension { dimension: key } => LedgerCause::UnknownDimension {
            dimension: dimension(key)?,
        },
        LedgerCause::UnknownPhase => LedgerCause::UnknownPhase,
        LedgerCause::DuplicatePhase => LedgerCause::DuplicatePhase,
        LedgerCause::DimensionPolicyShapeMismatch { dimension: key } => {
            LedgerCause::DimensionPolicyShapeMismatch {
                dimension: dimension(key)?,
            }
        }
        LedgerCause::DimensionNotReservable { dimension: key } => {
            LedgerCause::DimensionNotReservable {
                dimension: dimension(key)?,
            }
        }
        LedgerCause::ArithmeticOverflow {
            class,
            dimension: key,
        } => LedgerCause::ArithmeticOverflow {
            class,
            dimension: dimension(key)?,
        },
        LedgerCause::LimitExceeded {
            class,
            dimension: key,
            value,
            limit,
        } => LedgerCause::LimitExceeded {
            class,
            dimension: dimension(key)?,
            value,
            limit,
        },
        LedgerCause::ConsumedExceedsAttempted {
            dimension: key,
            consumed,
            attempted,
        } => LedgerCause::ConsumedExceedsAttempted {
            dimension: dimension(key)?,
            consumed,
            attempted,
        },
        LedgerCause::ConsumedBatchOutOfOrder {
            previous,
            dimension: key,
        } => LedgerCause::ConsumedBatchOutOfOrder {
            previous: dimension(previous)?,
            dimension: dimension(key)?,
        },
        LedgerCause::OwnerMismatch => LedgerCause::OwnerMismatch,
        LedgerCause::InactiveReservation => LedgerCause::InactiveReservation,
        LedgerCause::InactiveLinearWorkPermit => LedgerCause::InactiveLinearWorkPermit,
        LedgerCause::SplitExceedsReservation {
            requested,
            available,
        } => LedgerCause::SplitExceedsReservation {
            requested,
            available,
        },
        LedgerCause::PublicationAlreadyPending => LedgerCause::PublicationAlreadyPending,
        LedgerCause::PublicationAlreadyCompleted => LedgerCause::PublicationAlreadyCompleted,
        LedgerCause::InactivePublication => LedgerCause::InactivePublication,
        LedgerCause::PublicationLiveMismatch {
            transaction_bytes,
            ledger_bytes,
        } => LedgerCause::PublicationLiveMismatch {
            transaction_bytes,
            ledger_bytes,
        },
        LedgerCause::LiveReservationsRemain { bytes } => {
            LedgerCause::LiveReservationsRemain { bytes }
        }
        LedgerCause::ActiveReservationsRemain { count } => {
            LedgerCause::ActiveReservationsRemain { count }
        }
        LedgerCause::OutstandingLinearWorkPermits { count } => {
            LedgerCause::OutstandingLinearWorkPermits { count }
        }
        LedgerCause::PendingPublicationsRemain => LedgerCause::PendingPublicationsRemain,
        LedgerCause::Evidence { cause } => LedgerCause::Evidence { cause },
        LedgerCause::FinalSnapshotShapeMismatch => LedgerCause::FinalSnapshotShapeMismatch,
        _ => return None,
    })
}

#[derive(Debug)]
pub(crate) enum ImportObservedWorkFailure<E> {
    Store(ImportStoreHostError),
    Action(E),
}

pub(crate) fn try_import_store_bootstrap(
    limits: &ResourceLimits,
    requirements: ImportStoreBootstrapRequirements,
) -> Result<ImportStoreBootstrap, ImportStoreBootstrapError> {
    let import_limits =
        import_ledger_limits(limits).map_err(ImportStoreBootstrapError::ImportLedgerLimits)?;
    let domain = StoreKeyDomain::try_new(IMPORT_STORE_CATALOG)
        .map_err(ImportStoreBootstrapError::KeyDomain)?;
    let vocabulary = ImportStoreVocabulary::try_admit(&domain)?;
    let canonical_retained_member_policy = requirements.canonical_retained_member_policy();
    let canonical_structural_work_policy = requirements.canonical_structural_work_policy();
    let startup_outcome_evidence_policy =
        DimensionPolicy::reservable_bytes(0, 0, 0, limits.max_live_bytes)
            .with_published_outcome_evidence_limit(requirements.published_outcome_evidence_limit);
    let (ordinary_attempted_work_limit, ordinary_consumed_work_limit) = vocabulary
        .dimensions
        .iter()
        .try_fold((0_u64, 0_u64), |(attempted, consumed), (dimension, _)| {
            let policy = import_limits.dimension(*dimension).ok_or(
                ImportStoreBootstrapError::MissingImportDimensionPolicy {
                    dimension: *dimension,
                },
            )?;
            Ok::<_, ImportStoreBootstrapError>((
                attempted
                    .checked_add(policy.attempted_limit())
                    .ok_or(StorePlanError::SizeOverflow)?,
                consumed
                    .checked_add(policy.consumed_limit())
                    .ok_or(StorePlanError::SizeOverflow)?,
            ))
        })?;
    let ordinary_attempted_work_limit = ordinary_attempted_work_limit
        .checked_add(requirements.canonical_structural_work_limit.get())
        .ok_or(StorePlanError::SizeOverflow)?;
    let ordinary_consumed_work_limit = ordinary_consumed_work_limit
        .checked_add(requirements.canonical_structural_work_limit.get())
        .ok_or(StorePlanError::SizeOverflow)?;
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        vocabulary.failure_terminal_work,
        requirements.max_failure_evidence_members,
        requirements.work.failure_construction_work_limit,
        requirements.work.failure_validation_work_limit,
        requirements.work.total_attempted_work_limit,
        requirements.work.total_consumed_work_limit,
    )?;
    if ordinary_attempted_work_limit != failure_terminal_work.ordinary_attempted_work_limit()
        || ordinary_consumed_work_limit != failure_terminal_work.ordinary_consumed_work_limit()
    {
        return Err(ImportStoreBootstrapError::OrdinaryWorkPartitionMismatch {
            actual_attempted: ordinary_attempted_work_limit,
            expected_attempted: failure_terminal_work.ordinary_attempted_work_limit(),
            actual_consumed: ordinary_consumed_work_limit,
            expected_consumed: failure_terminal_work.ordinary_consumed_work_limit(),
        });
    }
    let failure_terminal_policy = failure_terminal_work.requirement().dimension_policy();
    let store_limits = LedgerLimits::with_evidence_byte_limit(
        import_limits.operation_live_byte_limit(),
        import_limits.evidence_byte_limit(),
        vocabulary
            .dimensions
            .iter()
            .copied()
            .map(|(dimension, key)| {
                let policy = import_limits
                    .dimension(dimension)
                    .expect("the complete import policy was validated before store mapping");
                (key, policy)
            })
            .chain([
                (
                    vocabulary.startup_outcome_evidence,
                    startup_outcome_evidence_policy,
                ),
                (vocabulary.failure_terminal_work, failure_terminal_policy),
                (
                    vocabulary.canonical_retained_member_dimension(),
                    canonical_retained_member_policy,
                ),
                (
                    vocabulary.canonical_structural_work_dimension(),
                    canonical_structural_work_policy,
                ),
            ]),
    )
    .map_err(ImportStoreBootstrapError::StoreLedgerLimits)?;
    let phase_keys = vocabulary.phase_keys();
    let ledger_plan = StoreLedgerPlan::try_new(domain, store_limits, &phase_keys)
        .map_err(ImportStoreBootstrapError::LedgerPlan)?;
    let start_plan = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            vocabulary.phase(ImportLedgerPhase::OutcomeFinalization),
            vocabulary.startup_outcome_evidence,
            vocabulary.startup_outcome_evidence,
            requirements.terminal_writer_limits,
        ),
        RetainedStoreSharePlan::new(
            vocabulary.canonical_structural_work_dimension(),
            vocabulary.canonical_structural_work_dimension(),
        ),
        StoreFallbackEnvelope::new(IMPORT_STORE_FALLBACK_CODE),
        vocabulary.startup_outcome_evidence,
        STARTUP_OUTCOME_EVIDENCE_AMOUNT,
        vocabulary.startup_outcome_evidence,
        STARTUP_OUTCOME_EVIDENCE_AMOUNT,
        failure_terminal_work,
    );
    Ok(ImportStoreBootstrap {
        vocabulary,
        ledger_plan,
        start_plan,
    })
}

fn admit_phase(
    domain: &StoreKeyDomain,
    phase: ImportLedgerPhase,
) -> Result<(ImportLedgerPhase, StorePhaseKey), ImportStoreBootstrapError> {
    let key = domain
        .try_admit_phase(import_phase_semantic_id(phase))
        .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
    Ok((phase, key))
}

fn admit_dimension(
    domain: &StoreKeyDomain,
    dimension: ImportDimension,
) -> Result<(ImportDimension, StoreDimensionKey), ImportStoreBootstrapError> {
    let key = domain
        .try_admit_dimension(import_dimension_semantic_id(dimension))
        .ok_or(ImportStoreBootstrapError::KeyAdmissionInvariant)?;
    Ok((dimension, key))
}

const fn import_phase_semantic_id(phase: ImportLedgerPhase) -> StorePhaseSemanticId {
    let local = match phase {
        ImportLedgerPhase::PrimarySnapshot => 1,
        ImportLedgerPhase::Detection => 2,
        ImportLedgerPhase::DependencyAcquisition => 3,
        ImportLedgerPhase::BackendDecode => 4,
        ImportLedgerPhase::FormatNormalization => 5,
        ImportLedgerPhase::SceneConstruction => 6,
        ImportLedgerPhase::Validation => 7,
        ImportLedgerPhase::OutcomeFinalization => 8,
    };
    StorePhaseSemanticId::new(
        IMPORT_STORE_CATALOG,
        NonZeroU64::new(local).expect("import phase semantic ids are nonzero"),
    )
}

const fn import_dimension_semantic_id(dimension: ImportDimension) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        IMPORT_STORE_CATALOG,
        NonZeroU64::new(dimension as u64 + 1).expect("import dimension semantic ids are nonzero"),
    )
}

const fn phase_index(phase: ImportLedgerPhase) -> usize {
    match phase {
        ImportLedgerPhase::PrimarySnapshot => 0,
        ImportLedgerPhase::Detection => 1,
        ImportLedgerPhase::DependencyAcquisition => 2,
        ImportLedgerPhase::BackendDecode => 3,
        ImportLedgerPhase::FormatNormalization => 4,
        ImportLedgerPhase::SceneConstruction => 5,
        ImportLedgerPhase::Validation => 6,
        ImportLedgerPhase::OutcomeFinalization => 7,
    }
}

#[derive(Debug)]
pub(crate) enum ImportStoreBootstrapError {
    KeyDomain(StoreKeyDomainCreateError),
    KeyAdmissionInvariant,
    ImportLedgerLimits(LedgerLimitsError<ImportDimension>),
    MissingImportDimensionPolicy {
        dimension: ImportDimension,
    },
    OrdinaryWorkPartitionMismatch {
        actual_attempted: u64,
        expected_attempted: u64,
        actual_consumed: u64,
        expected_consumed: u64,
    },
    PublishedOutcomeEvidenceLimitTooSmall {
        limit: u64,
        required: u64,
    },
    PublishedCanonicalRetainedMemberLimitTooSmall {
        limit: u64,
        required: u64,
    },
    StoreLedgerLimits(LedgerLimitsError<StoreDimensionKey>),
    LedgerPlan(StoreLedgerPlanError),
    StorePlan(StorePlanError),
}

impl From<StorePlanError> for ImportStoreBootstrapError {
    fn from(cause: StorePlanError) -> Self {
        Self::StorePlan(cause)
    }
}

impl fmt::Display for ImportStoreBootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyDomain(cause) => {
                write!(formatter, "cannot create import store domain: {cause}")
            }
            Self::KeyAdmissionInvariant => {
                formatter.write_str("cannot admit the normalized import store vocabulary")
            }
            Self::ImportLedgerLimits(cause) => {
                write!(formatter, "cannot freeze import ledger policy: {cause}")
            }
            Self::MissingImportDimensionPolicy { dimension } => write!(
                formatter,
                "import store vocabulary has no policy for {dimension:?}"
            ),
            Self::OrdinaryWorkPartitionMismatch {
                actual_attempted,
                expected_attempted,
                actual_consumed,
                expected_consumed,
            } => write!(
                formatter,
                "import ordinary work policy is {actual_attempted}/{actual_consumed}, expected the caller-owned protected remainder {expected_attempted}/{expected_consumed}"
            ),
            Self::PublishedOutcomeEvidenceLimitTooSmall { limit, required } => write!(
                formatter,
                "import outcome-evidence publication limit {limit} is below the startup requirement {required}"
            ),
            Self::PublishedCanonicalRetainedMemberLimitTooSmall { limit, required } => write!(
                formatter,
                "import canonical retained-member publication limit {limit} is below the success requirement {required}"
            ),
            Self::StoreLedgerLimits(cause) => {
                write!(
                    formatter,
                    "cannot map import policy into store limits: {cause}"
                )
            }
            Self::LedgerPlan(cause) => {
                write!(
                    formatter,
                    "cannot freeze the import store ledger plan: {cause}"
                )
            }
            Self::StorePlan(cause) => {
                write!(
                    formatter,
                    "cannot build the import retained-store plan: {cause}"
                )
            }
        }
    }
}

impl std::error::Error for ImportStoreBootstrapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::KeyDomain(cause) => Some(cause),
            Self::ImportLedgerLimits(cause) => Some(cause),
            Self::StoreLedgerLimits(cause) => Some(cause),
            Self::LedgerPlan(cause) => Some(cause),
            Self::StorePlan(cause) => Some(cause),
            Self::KeyAdmissionInvariant
            | Self::MissingImportDimensionPolicy { .. }
            | Self::OrdinaryWorkPartitionMismatch { .. }
            | Self::PublishedOutcomeEvidenceLimitTooSmall { .. }
            | Self::PublishedCanonicalRetainedMemberLimitTooSmall { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use baozi_core::__canonical_import::{
        AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoredTransform,
        DeformationAuthoringBatch, GeometryAuthoringBatch, LineageAuthoringBatch,
        MetadataAuthoringBatch, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
        OperationKind, OptionsDigest, OwnerProvenanceAuthoring, Presence, ProfileAuthoringBatch,
        ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
        SceneActivationRequest, SceneAuthoringBatch, SceneSpace, SceneValidationLimits,
        SpatialAuthoringBatch,
    };
    use baozi_operation::{
        CancellationOrigin, CancellationSignal, PhysicalStorageAccountFactory,
        PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, RetainedStoreOperation,
        StoreTerminalCause,
    };

    use crate::ledger::{
        ImportOperationControl, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit,
    };

    #[test]
    fn retained_session_owner_has_a_bounded_inline_stack_footprint() {
        const MAX_INLINE_SESSION_OWNER_BYTES: usize = 64 * 1024;

        assert!(size_of::<ImportStoreOperation>() <= MAX_INLINE_SESSION_OWNER_BYTES);
        assert!(size_of::<ImportSceneDraftBuild>() <= MAX_INLINE_SESSION_OWNER_BYTES);
        assert!(size_of::<ImportPendingSceneDraftOwner>() <= MAX_INLINE_SESSION_OWNER_BYTES);
        assert!(size_of::<ImportPendingSceneFailureOwner>() <= MAX_INLINE_SESSION_OWNER_BYTES);
        assert!(size_of::<ImportStoreSessionState>() <= MAX_INLINE_SESSION_OWNER_BYTES);
        assert!(size_of::<ImportStoreSessionOwner>() <= MAX_INLINE_SESSION_OWNER_BYTES);
    }

    fn ordinary_work_limits(limits: &ResourceLimits) -> (u64, u64) {
        let import_limits = import_ledger_limits(limits).expect("test import policy is valid");
        ImportDimension::ALL
            .into_iter()
            .try_fold((0_u64, 0_u64), |(attempted, consumed), dimension| {
                let policy = import_limits
                    .dimension(dimension)
                    .expect("the admitted import dimension has a policy");
                Some((
                    attempted.checked_add(policy.attempted_limit())?,
                    consumed.checked_add(policy.consumed_limit())?,
                ))
            })
            .expect("default import aggregate work fits u64")
    }

    fn requirements_for(limits: &ResourceLimits) -> ImportStoreBootstrapRequirements {
        ImportStoreBootstrapRequirements::for_node_scene(limits)
            .expect("test retained-store requirements are valid")
    }

    fn requirements() -> ImportStoreBootstrapRequirements {
        requirements_for(&ResourceLimits::default())
    }

    #[test]
    fn source_graph_limits_do_not_shrink_canonical_relation_envelopes() {
        let limits = ResourceLimits {
            max_graph_edges: 3,
            max_graph_work: 7,
            max_canonical_payload_bytes: 1_024,
            ..ResourceLimits::default()
        };

        let scene = scene_validation_limits(&limits);

        assert_eq!(scene.max_edges(), 1_024);
        assert_eq!(scene.max_references(), 1_024);
    }

    fn start_store_owner() -> ImportStoreSessionOwner {
        let (owner, account) = start_store_owner_with_account();
        drop(account);
        owner
    }

    fn start_store_owner_with_account() -> (ImportStoreSessionOwner, PhysicalStorageAccount) {
        start_store_owner_with_account_and_control(
            ResourceLimits::default(),
            ImportOperationControl::default(),
        )
    }

    fn start_store_owner_with_account_and_control(
        limits: ResourceLimits,
        control: ImportOperationControl,
    ) -> (ImportStoreSessionOwner, PhysicalStorageAccount) {
        let bootstrap = try_import_store_bootstrap(&limits, requirements_for(&limits))
            .expect("test import store bootstrap must be valid");
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
                .expect("test factory limits are valid"),
        )
        .expect("test account factory must allocate");
        let account = factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX)
                    .expect("test account limits are valid"),
            )
            .expect("test physical account must allocate");
        let owner = ImportStoreSessionOwner::try_start(control, &account, bootstrap)
            .expect("the import store owner must start");
        drop(factory);
        (owner, account)
    }

    #[derive(Clone)]
    struct CancelOnSignalPoll {
        polls: Arc<AtomicUsize>,
        cancel_on: Arc<AtomicUsize>,
    }

    impl CancelOnSignalPoll {
        fn new() -> Self {
            Self {
                polls: Arc::new(AtomicUsize::new(0)),
                cancel_on: Arc::new(AtomicUsize::new(usize::MAX)),
            }
        }

        fn cancel_after_additional_polls(&self, additional: usize) {
            let current = self.polls.load(Ordering::Acquire);
            self.cancel_on.store(
                current
                    .checked_add(additional)
                    .expect("test cancellation poll threshold must fit usize"),
                Ordering::Release,
            );
        }
    }

    impl CancellationSignal for CancelOnSignalPoll {
        fn is_cancelled(&self) -> bool {
            let poll = self.polls.fetch_add(1, Ordering::AcqRel) + 1;
            poll >= self.cancel_on.load(Ordering::Acquire)
        }
    }

    fn with_two_node_request_parent<R>(
        child_parent: NodeLocalId,
        action: impl for<'input> FnOnce(SceneActivationRequest<'input>) -> R,
    ) -> R {
        let provenance = [ProvenanceLocalId::new(0)];
        let operation_kind = OperationKind::Generated;
        let provenance_nodes = [ProvenanceNodeAuthoring::operation(
            ProvenanceOperationAuthoring::new(
                &operation_kind,
                OptionsDigest::sha256([0; 32]),
                &[],
                &[],
            ),
        )];
        let owner_provenance = [
            OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance),
            OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance),
            OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &provenance),
            OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(1)), &provenance),
        ];
        let roots = [NodeLocalId::new(0)];
        let nodes = [
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            ),
            NodeAuthoringNode::new(
                Presence::Present(NodeParent::new(child_parent, 0)),
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let input = SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
                MetadataAuthoringBatch::empty(),
            ),
        );
        let request = SceneActivationRequest::try_new(input, SceneValidationLimits::default())
            .expect("the complete node-only batch must form one roleless activation request");
        action(request)
    }

    fn with_two_node_request<R>(
        action: impl for<'input> FnOnce(SceneActivationRequest<'input>) -> R,
    ) -> R {
        with_two_node_request_parent(NodeLocalId::new(0), action)
    }

    #[allow(
        clippy::result_large_err,
        reason = "the integration helper preserves the exact allocation-free activation error chain"
    )]
    fn submit_two_node(
        owner: &ImportStoreSessionOwner,
    ) -> Result<SceneDraft, ImportStoreActivationError> {
        with_two_node_request(|request| owner.submit_scene(request))
    }

    #[allow(
        clippy::result_large_err,
        reason = "the integration helper preserves the exact allocation-free authoring rejection chain"
    )]
    fn submit_invalid_parent(
        owner: &ImportStoreSessionOwner,
    ) -> Result<SceneDraft, ImportStoreActivationError> {
        with_two_node_request_parent(NodeLocalId::new(2), |request| owner.submit_scene(request))
    }

    #[test]
    fn bootstrap_maps_the_complete_import_vocabulary_into_one_store_domain() {
        let bootstrap = try_import_store_bootstrap(&ResourceLimits::default(), requirements())
            .expect("default import store bootstrap must be valid");
        let (vocabulary, ledger_plan, _start_plan) = bootstrap.into_parts();

        assert_eq!(ledger_plan.phase_count(), ImportLedgerPhase::ALL.len());
        assert_eq!(
            ledger_plan.dimension_count(),
            IMPORT_STORE_EVIDENCE_DIMENSION_COUNT
        );
        for (phase, key) in vocabulary.phases {
            assert_eq!(key.semantic_id(), import_phase_semantic_id(phase));
        }
        for (dimension, key) in vocabulary.dimensions {
            assert_eq!(key.semantic_id(), import_dimension_semantic_id(dimension));
        }
        assert_eq!(
            vocabulary.outcome_evidence_dimension().semantic_id(),
            STARTUP_OUTCOME_EVIDENCE_SEMANTIC
        );
        assert_eq!(
            vocabulary
                .canonical_retained_member_dimension()
                .semantic_id(),
            CANONICAL_RETAINED_MEMBERS_SEMANTIC
        );
        assert_eq!(
            vocabulary
                .canonical_structural_work_dimension()
                .semantic_id(),
            CANONICAL_STRUCTURAL_WORK_SEMANTIC
        );
    }

    #[test]
    fn bootstrap_plans_start_and_finish_one_retained_store_operation() {
        let bootstrap = try_import_store_bootstrap(&ResourceLimits::default(), requirements())
            .expect("default import store bootstrap must be valid");
        let (_vocabulary, ledger_plan, start_plan) = bootstrap.into_parts();
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
                .expect("test factory limits are valid"),
        )
        .expect("test account factory must allocate");
        let account = factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX)
                    .expect("test account limits are valid"),
            )
            .expect("test physical account must allocate");
        let operation = RetainedStoreOperation::<
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >::try_start(
            ImportOperationControl::default(),
            ledger_plan,
            &account,
            start_plan,
        )
        .expect("the bootstrap plans must start one retained store operation");

        let published = operation
            .prepare_minimal_fallback(IMPORT_STORE_FALLBACK_CODE)
            .expect("the pre-admitted fallback must prepare")
            .handoff();
        assert_eq!(
            published.fallback.value().code(),
            IMPORT_STORE_FALLBACK_CODE
        );
        assert!(matches!(
            published.ledger_evidence.header().terminal_cause(),
            baozi_operation::StoreTerminalCause::Failure(cause)
                if cause.code() == IMPORT_STORE_FALLBACK_CODE
        ));
    }

    #[test]
    fn session_owner_drives_node_scene_success_on_the_same_import_ledger() {
        let (owner, account) = start_store_owner_with_account();
        let host = owner.host();
        host.with_observed_work([(ImportDimension::DetectionWork, 1)], |work| {
            work.complete([1])
                .expect("the producer detection unit must complete");
            Ok::<_, ()>(())
        })
        .expect("producer work must remain on the sole import ledger");

        let draft = submit_two_node(&owner)
            .expect("the import owner must activate the exact NodeScene request");
        owner
            .poll_parser_boundary()
            .expect("a submitted Scene already completed the transaction's final control poll");
        assert!(matches!(
            host.poll(),
            Err(ImportStoreHostError::OperationConsumed)
        ));
        let terminal = owner
            .prepare_scene(Some(draft))
            .expect("the centrally validated draft must prepare")
            .handoff();
        let terminal_evidence = terminal.ledger_evidence();
        assert!(
            terminal.is_success(),
            "NodeScene terminal cause: {:?}; work: {:?}; canonical: {:?}; evidence: {:?}",
            terminal_evidence.header().terminal_cause(),
            terminal_evidence
                .dimension_by_semantic(import_dimension_semantic_id(ImportDimension::WorkUnits)),
            terminal_evidence.dimension_by_semantic(CANONICAL_RETAINED_MEMBERS_SEMANTIC),
            terminal_evidence.dimension_by_semantic(STARTUP_OUTCOME_EVIDENCE_SEMANTIC),
        );
        let canonical_block_count = u64::try_from(
            terminal
                .canonical_block_count()
                .expect("success must retain one canonical Scene"),
        )
        .expect("the canonical block count fits u64");
        let evidence = terminal_evidence;
        assert!(matches!(
            evidence.header().terminal_cause(),
            StoreTerminalCause::Success
        ));
        let detection = evidence
            .dimension_by_semantic(import_dimension_semantic_id(ImportDimension::DetectionWork))
            .expect("the terminal ledger retains pre-activation producer work");
        assert_eq!(detection.attempted(), 1);
        assert_eq!(detection.consumed(), 1);
        let work = evidence
            .dimension_by_semantic(import_dimension_semantic_id(ImportDimension::WorkUnits))
            .expect("the terminal ledger retains canonical work");
        assert!(work.attempted() > 0);
        assert_eq!(work.attempted(), work.consumed());
        let validation = evidence
            .phase_by_semantic(import_phase_semantic_id(ImportLedgerPhase::Validation))
            .expect("central validation uses the import validation phase");
        assert!(validation.peak_live_bytes() > 0);
        let canonical = evidence
            .dimension_by_semantic(CANONICAL_RETAINED_MEMBERS_SEMANTIC)
            .expect("the terminal ledger retains the private canonical destination");
        assert_eq!(
            canonical.committed_canonical_output(),
            canonical_block_count
        );
        let outcome_evidence = evidence
            .dimension_by_semantic(STARTUP_OUTCOME_EVIDENCE_SEMANTIC)
            .expect("the terminal ledger retains the private evidence destination");
        assert_eq!(outcome_evidence.published_outcome_evidence(), 2);

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn consuming_finalization_preserves_scene_report_and_unique_terminal_evidence() {
        let (owner, account) = start_store_owner_with_account();
        let draft = submit_two_node(&owner)
            .expect("the import owner must activate the exact NodeScene request");
        let outcome = owner
            .finish_scene(Some(draft))
            .expect("the exact descriptor must finish through one consuming terminal seam");

        let ImportStoreTerminalOutcome::Success {
            scene,
            report,
            evidence,
        } = &outcome
        else {
            panic!("the exact descriptor must preserve the successful Scene owners")
        };
        assert_eq!(evidence.cause(), ImportStoreTerminalCause::Success);
        assert_eq!(
            u64::try_from(scene.canonical_block_count()).expect("the Scene block count fits u64"),
            evidence
                .terminal_dimension(ImportStoreTerminalDimension::CanonicalRetainedMembers)
                .expect("success must project the canonical destination")
                .committed_canonical_output()
        );
        assert_eq!(
            u64::try_from(report.outcome_evidence_block_count())
                .expect("the outcome-evidence block count fits u64"),
            evidence
                .terminal_dimension(ImportStoreTerminalDimension::StartupOutcomeEvidence)
                .expect("success must project the outcome-evidence destination")
                .published_outcome_evidence()
                .checked_sub(STARTUP_OUTCOME_EVIDENCE_AMOUNT)
                .expect("terminal evidence includes the startup fallback baseline")
        );
        assert!(matches!(
            report.ledger_evidence().header().terminal_cause(),
            StoreTerminalCause::Success
        ));

        drop(outcome);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn session_owner_without_scene_submission_prepares_requested_terminal_on_the_same_ledger() {
        let (owner, account) = start_store_owner_with_account();
        owner
            .host()
            .with_observed_work([(ImportDimension::DetectionWork, 1)], |work| {
                work.complete([1])
                    .expect("the producer detection unit must complete");
                Ok::<_, ()>(())
            })
            .expect("producer work must remain on the sole import ledger");

        let terminal = owner
            .prepare_scene(None)
            .expect("an import without a Scene must prepare the pre-admitted fallback")
            .handoff();
        assert!(!terminal.is_success());
        assert_eq!(terminal.canonical_block_count(), None);
        assert!(matches!(
            terminal.ledger_evidence().header().terminal_cause(),
            StoreTerminalCause::Failure(cause)
                if cause.code() == IMPORT_SCENE_NOT_SUBMITTED_CODE
        ));

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the unactivated terminal must project into the complete import vocabulary");
        assert_eq!(
            evidence.cause().class(),
            ImportStoreTerminalClass::Requested
        );
        assert_eq!(
            evidence.cause().code(),
            Some(IMPORT_SCENE_NOT_SUBMITTED_CODE)
        );
        let detection = evidence
            .dimension(ImportDimension::DetectionWork)
            .expect("the terminal must retain pre-activation producer work");
        assert_eq!(detection.attempted(), 1);
        assert_eq!(detection.consumed(), 1);
        let canonical = evidence
            .terminal_dimension(ImportStoreTerminalDimension::CanonicalRetainedMembers)
            .expect("the terminal must project the private canonical destination");
        assert_eq!(canonical.committed_canonical_output(), 0);

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn terminal_projection_maps_the_complete_import_vocabulary() {
        let owner = start_store_owner();
        let draft = submit_two_node(&owner)
            .expect("the import owner must activate the exact NodeScene request");
        let terminal = owner
            .prepare_scene(Some(draft))
            .expect("the centrally validated draft must prepare")
            .handoff();

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the exact terminal evidence must project into import vocabulary");
        assert_eq!(evidence.cause(), ImportStoreTerminalCause::Success);
        assert_eq!(evidence.cause().class(), ImportStoreTerminalClass::Success);
        assert_eq!(evidence.cause().code(), None);
        assert_eq!(evidence.current_live_bytes(), 0);
        assert_eq!(evidence.active_reservations(), 0);
        assert_eq!(evidence.phases().len(), ImportLedgerPhase::ALL.len());
        assert_eq!(evidence.dimensions().len(), ImportDimension::ALL.len());
        assert_eq!(
            evidence.terminal_dimensions().len(),
            ImportStoreTerminalDimension::ALL.len()
        );
        for expected_phase in ImportLedgerPhase::ALL {
            let phase = evidence
                .phase(expected_phase)
                .expect("every import phase must be projected");
            assert_eq!(phase.phase(), expected_phase);
            assert_eq!(phase.current_live_bytes(), 0);
        }
        for dimension in ImportDimension::ALL {
            let projected = evidence
                .dimension(dimension)
                .expect("every import dimension must be projected");
            assert_eq!(projected.dimension(), dimension);
            assert_eq!(projected.current_live_bytes(), 0);
            assert_eq!(projected.pending_publication(), 0);
        }
        for dimension in ImportStoreTerminalDimension::ALL {
            let projected = evidence
                .terminal_dimension(dimension)
                .expect("every terminal-only dimension must be projected");
            assert_eq!(projected.dimension(), dimension);
            assert_eq!(projected.current_live_bytes(), 0);
            assert_eq!(projected.pending_publication(), 0);
        }
    }

    #[test]
    fn terminal_projection_fails_closed_when_a_semantic_row_is_missing() {
        assert_eq!(
            project_import_phases(|_| None).expect_err("a missing phase must fail closed"),
            ImportStoreTerminalProjectionError::MissingPhase {
                phase: ImportLedgerPhase::PrimarySnapshot,
            }
        );
        assert_eq!(
            project_import_dimensions(|_| None)
                .expect_err("a missing import dimension must fail closed"),
            ImportStoreTerminalProjectionError::MissingDimension {
                dimension: ImportDimension::AssetOpen,
            }
        );
        assert_eq!(
            project_import_terminal_dimensions(|_| None)
                .expect_err("a missing terminal dimension must fail closed"),
            ImportStoreTerminalProjectionError::MissingTerminalDimension {
                dimension: ImportStoreTerminalDimension::StartupOutcomeEvidence,
            }
        );
    }

    #[test]
    fn terminal_projection_reads_requested_cause_from_missing_draft_terminal() {
        let owner = start_store_owner();
        let draft = submit_two_node(&owner)
            .expect("the import owner must activate the exact NodeScene request");
        drop(draft);
        let terminal = owner
            .prepare_scene(None)
            .expect("a missing parser descriptor must prepare exact failure evidence")
            .handoff();
        let StoreTerminalCause::Failure(header_cause) =
            terminal.ledger_evidence().header().terminal_cause()
        else {
            panic!("a missing parser descriptor must publish a failure cause")
        };

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the missing-draft terminal must retain complete import evidence");
        assert_eq!(
            evidence.cause().class(),
            ImportStoreTerminalClass::Requested
        );
        assert_eq!(evidence.cause().code(), Some(header_cause.code()));
    }

    #[test]
    fn terminal_projection_reads_requested_cause_from_authoring_rejection() {
        let owner = start_store_owner();
        let result = submit_invalid_parent(&owner);
        assert!(matches!(
            result,
            Err(ImportStoreActivationError::AuthoringRejected)
        ));
        let terminal = owner
            .prepare_scene(None)
            .expect("the authoring rejection must retain its exact terminal")
            .handoff();
        let StoreTerminalCause::Failure(header_cause) =
            terminal.ledger_evidence().header().terminal_cause()
        else {
            panic!("an authoring rejection must publish a failure cause")
        };

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the authoring rejection must retain complete import evidence");
        assert_eq!(
            evidence.cause().class(),
            ImportStoreTerminalClass::Requested
        );
        assert_eq!(evidence.cause().code(), Some(header_cause.code()));
    }

    #[test]
    fn terminal_projection_maps_product_graph_header_class() {
        assert_eq!(
            import_terminal_failure_class(StorePreparationFailureClass::ProductGraph),
            Ok(ImportStoreTerminalFailureClass::ProductGraph)
        );
    }

    #[test]
    fn terminal_projection_reads_control_cause_from_cancelled_scene_terminal() {
        let cancellation = CancelOnSignalPoll::new();
        cancellation.cancel_after_additional_polls(1);
        let control = ImportOperationControl::default().with_cancellation(cancellation);
        let (owner, _account) =
            start_store_owner_with_account_and_control(ResourceLimits::default(), control);
        assert!(matches!(
            submit_two_node(&owner),
            Err(ImportStoreActivationError::Control(
                ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal
                }
            ))
        ));
        let terminal = owner
            .prepare_scene(None)
            .expect("cancelled authoring must retain its exact terminal")
            .handoff();
        assert!(matches!(
            terminal.control_cause(),
            Some(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            })
        ));
        let StoreTerminalCause::Failure(header_cause) =
            terminal.ledger_evidence().header().terminal_cause()
        else {
            panic!("cancelled authoring must publish a failure cause")
        };

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the control terminal must retain complete import evidence");
        assert_eq!(evidence.cause().class(), ImportStoreTerminalClass::Control);
        assert_eq!(evidence.cause().code(), Some(header_cause.code()));
        assert_eq!(evidence.current_live_bytes(), 0);
        assert_eq!(evidence.active_reservations(), 0);
    }

    #[test]
    fn terminal_projection_reads_ledger_cause_from_exhausted_scene_work() {
        let limits = ResourceLimits {
            max_work_units: 10,
            ..ResourceLimits::default()
        };
        let (owner, _account) =
            start_store_owner_with_account_and_control(limits, ImportOperationControl::default());
        assert!(matches!(
            submit_two_node(&owner),
            Err(ImportStoreActivationError::MaterializationFailed)
        ));
        let terminal = owner
            .prepare_scene(None)
            .expect("work exhaustion must retain its exact terminal")
            .handoff();
        let StoreTerminalCause::Failure(header_cause) =
            terminal.ledger_evidence().header().terminal_cause()
        else {
            panic!("work exhaustion must publish a failure cause")
        };

        let evidence = ImportStoreTerminalEvidence::try_from_terminal(&terminal)
            .expect("the ledger terminal must retain complete import evidence");
        assert_eq!(evidence.cause().class(), ImportStoreTerminalClass::Ledger);
        assert_eq!(evidence.cause().code(), Some(header_cause.code()));
        assert_eq!(evidence.current_live_bytes(), 0);
        assert_eq!(evidence.active_reservations(), 0);
    }

    #[test]
    fn missing_scene_draft_cannot_reserve_or_publish_success() {
        let (owner, account) = start_store_owner_with_account();
        let draft = submit_two_node(&owner)
            .expect("the import owner must activate the exact NodeScene request");
        drop(draft);

        let terminal = owner
            .prepare_scene(None)
            .expect("a missing parser descriptor must prepare exact failure evidence")
            .handoff();
        assert!(!terminal.is_success());
        assert!(matches!(
            terminal.ledger_evidence().header().terminal_cause(),
            StoreTerminalCause::Failure(_)
        ));

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn sibling_scene_drafts_cannot_be_substituted() {
        let (left, left_account) = start_store_owner_with_account();
        let (right, right_account) = start_store_owner_with_account();
        let left_draft =
            submit_two_node(&left).expect("the left import owner must activate its draft");
        let right_draft =
            submit_two_node(&right).expect("the right import owner must activate its draft");

        let left_terminal = left
            .prepare_scene(Some(right_draft))
            .expect("the left owner must freeze a foreign-draft failure")
            .handoff();
        let right_terminal = right
            .prepare_scene(Some(left_draft))
            .expect("the right owner must freeze a foreign-draft failure")
            .handoff();
        assert!(!left_terminal.is_success());
        assert!(!right_terminal.is_success());

        drop(left_terminal);
        drop(right_terminal);
        for account in [left_account, right_account] {
            let released = account.snapshot();
            assert_eq!(released.live_retained_bytes(), 0);
            assert_eq!(released.live_retained_allocations(), 0);
            assert!(!released.poisoned());
        }
    }

    #[test]
    fn second_scene_submission_cannot_replace_the_pending_owner() {
        let (owner, account) = start_store_owner_with_account();
        let draft = submit_two_node(&owner)
            .expect("the first Scene request must install its pending owner");
        let second = submit_two_node(&owner);
        assert!(matches!(
            second,
            Err(ImportStoreActivationError::StoreUnavailable(
                ImportStoreHostError::OperationConsumed
            ))
        ));

        let terminal = owner
            .prepare_scene(Some(draft))
            .expect("the first descriptor must remain paired after the rejected submission")
            .handoff();
        assert!(terminal.is_success());

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn authoring_rejection_defers_terminal_prepare_until_parser_storage_is_released() {
        let (owner, account) = start_store_owner_with_account();
        let parser_scratch = owner
            .host()
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::ScratchBytes,
                8,
            )
            .expect("parser scratch must remain live across the synchronous Scene submission");
        let result = submit_invalid_parent(&owner);
        assert!(matches!(
            result,
            Err(ImportStoreActivationError::AuthoringRejected)
        ));
        let pending = account.snapshot();
        assert!(pending.live_retained_bytes() >= 8);
        assert!(pending.live_retained_allocations() > 0);
        assert!(!pending.poisoned());

        drop(parser_scratch);

        let terminal = owner
            .prepare_scene(None)
            .expect("the quiescent session must prepare its deferred authoring failure")
            .handoff();
        assert!(!terminal.is_success());
        assert!(matches!(
            terminal.ledger_evidence().header().terminal_cause(),
            StoreTerminalCause::Failure(_)
        ));

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn ledger_rejection_can_prepare_deferred_terminal_after_the_session_is_quiescent() {
        let limits = ResourceLimits {
            max_open_assets: 0,
            ..ResourceLimits::default()
        };
        let (owner, account) =
            start_store_owner_with_account_and_control(limits, ImportOperationControl::default());
        let failure = owner
            .host()
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .expect_err("the zero asset-open budget must latch one ledger failure");
        assert!(matches!(failure, ImportStoreHostError::Ledger(_)));

        assert!(submit_invalid_parent(&owner).is_err());
        let terminal = owner
            .prepare_scene(None)
            .expect("a quiescent ledger rejection must still prepare its terminal evidence")
            .handoff();
        assert!(!terminal.is_success());

        drop(terminal);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn dropping_a_pending_scene_owner_releases_unpublished_storage() {
        let (owner, account) = start_store_owner_with_account();
        let draft = submit_two_node(&owner)
            .expect("the import owner must retain one unpublished Scene draft");
        assert!(account.snapshot().live_retained_allocations() > 0);

        drop(draft);
        drop(owner);
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    #[test]
    fn grouped_observed_work_uses_import_dimensions_on_the_same_operation() {
        let owner = start_store_owner();
        let host = owner.host();

        host.with_observed_work(
            [
                (ImportDimension::PrimaryBytes, 3),
                (ImportDimension::TotalAssetBytes, 3),
            ],
            |work| {
                assert_eq!(work.remaining(), [3, 3]);
                work.complete([1, 1])
                    .expect("the first physical prefix must complete atomically");
                assert_eq!(work.remaining(), [2, 2]);
                work.complete([2, 2])
                    .expect("the admitted suffix must complete atomically");
                Ok::<_, ()>(())
            },
        )
        .expect("the exact observed-work envelope must finish");

        for dimension in [
            ImportDimension::PrimaryBytes,
            ImportDimension::TotalAssetBytes,
        ] {
            let state = host
                .dimension_state(dimension)
                .expect("the import dimension must remain observable through the same host");
            assert_eq!(state.attempted(), 3);
            assert_eq!(state.consumed(), 3);
        }
    }

    #[test]
    fn split_parser_accounting_releases_the_session_borrow_between_actions() {
        let owner = start_store_owner();
        let host = owner.host();

        host.charge_attempted(ImportDimension::DecodeWork, 2)
            .expect("the parser admits its exact decode envelope");
        let reservation = host
            .reserve(
                ImportLedgerPhase::BackendDecode,
                ImportDimension::ScratchBytes,
                8,
            )
            .expect("metered storage can borrow the same session between work charges");
        host.charge_consumed(ImportDimension::DecodeWork, 1)
            .expect("the first completed decode prefix is retained");
        host.charge_consumed_batch(&[(ImportDimension::DecodeWork, 1)])
            .expect("the final completed prefix is retained");

        let decode = host
            .dimension_state(ImportDimension::DecodeWork)
            .expect("decode work remains observable");
        assert_eq!(decode.attempted(), 2);
        assert_eq!(decode.consumed(), 2);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .expect("scratch storage remains on the same operation")
                .live_bytes(),
            8
        );

        drop(reservation);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .expect("dropping the reservation updates the same operation")
                .live_bytes(),
            0
        );
    }

    #[test]
    fn action_failure_resolves_observed_work_with_the_exact_completed_prefix() {
        let owner = start_store_owner();
        let host = owner.host();

        let result = host.with_observed_work(
            [
                (ImportDimension::PrimaryBytes, 3),
                (ImportDimension::TotalAssetBytes, 3),
            ],
            |work| {
                work.complete([1, 1])
                    .expect("the completed physical prefix must be recorded");
                Err::<(), _>("short-read")
            },
        );
        assert!(matches!(
            result,
            Err(ImportObservedWorkFailure::Action("short-read"))
        ));

        for dimension in [
            ImportDimension::PrimaryBytes,
            ImportDimension::TotalAssetBytes,
        ] {
            let state = host
                .dimension_state(dimension)
                .expect("partial work remains observable on the sole operation");
            assert_eq!(state.attempted(), 3);
            assert_eq!(state.consumed(), 1);
        }
    }

    #[test]
    fn host_reservations_remain_bound_to_the_import_operation() {
        let owner = start_store_owner();
        let host = owner.host();
        let mut reservation = host
            .reserve(
                ImportLedgerPhase::Detection,
                ImportDimension::ScratchBytes,
                8,
            )
            .expect("the host must reserve import scratch through the retained operation");
        assert_eq!(reservation.bytes(), 8);
        host.validate_live_owner(&reservation)
            .expect("the import wrapper must preserve the sole operation owner");

        host.resize(&mut reservation, 16)
            .expect("the same host must resize its reservation");
        assert_eq!(reservation.bytes(), 16);
        host.transition(
            &mut reservation,
            ImportLedgerPhase::DependencyAcquisition,
            ImportDimension::DependencyBytes,
            12,
        )
        .expect("the same host must transition its reservation");
        assert_eq!(reservation.bytes(), 12);
        host.validate_live_owner(&reservation)
            .expect("transition must preserve the sole operation owner");
        assert_eq!(
            host.dimension_state(ImportDimension::DependencyBytes)
                .expect("the transitioned dimension must be observable")
                .live_bytes(),
            12
        );

        drop(reservation);
        assert_eq!(
            host.dimension_state(ImportDimension::DependencyBytes)
                .expect("dropping the reservation must update the same operation")
                .live_bytes(),
            0
        );
    }

    #[test]
    fn host_tightens_import_policy_without_exposing_store_keys() {
        let owner = start_store_owner();
        let host = owner.host();

        host.tighten_dimension_policy(
            ImportDimension::DetectionWork,
            DimensionPolicy::counter(3, 2, 0),
        )
        .expect("the import host must tighten its own qualified dimension");

        let result = host.with_observed_work([(ImportDimension::DetectionWork, 3)], |work| {
            work.complete([3])
                .expect("the completed prefix must be retained before projection fails");
            Ok::<_, ()>(())
        });
        assert!(matches!(
            result,
            Err(ImportObservedWorkFailure::Store(
                ImportStoreHostError::Ledger(_)
            ))
        ));
        let state = host
            .dimension_state(ImportDimension::DetectionWork)
            .expect("the tightened dimension must remain observable");
        assert_eq!(state.attempted(), 3);
        assert_eq!(state.consumed(), 0);
    }

    #[test]
    fn host_projects_runtime_ledger_keys_back_to_import_semantics() {
        let owner = start_store_owner();
        let host = owner.host();
        host.tighten_dimension_policy(
            ImportDimension::DetectionWork,
            DimensionPolicy::counter(1, 1, 0),
        )
        .expect("the detection policy must tighten before work");

        let cause = host
            .charge_attempted(ImportDimension::DetectionWork, 2)
            .expect_err("the attempted prefix crosses the tightened limit");
        assert!(matches!(
            &cause,
            ImportStoreHostError::Ledger(LedgerCause::LimitExceeded {
                dimension,
                value,
                limit,
                ..
            }) if *dimension == ImportDimension::DetectionWork && *value == 2 && *limit == 1
        ));

        let projected = cause.into_scope_abort_cause();
        assert!(matches!(
            projected,
            ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                dimension: ImportDimension::DetectionWork,
                value: 2,
                limit: 1,
                ..
            })
        ));
    }

    #[test]
    fn host_rejects_reentrant_operation_borrows_without_panicking() {
        let owner = start_store_owner();
        let host = owner.host();
        let _state = owner.state.borrow_mut();

        assert!(matches!(
            host.poll(),
            Err(ImportStoreHostError::ReentrantBorrow)
        ));
    }

    #[test]
    fn abandoned_scene_transition_becomes_explicit_containment() {
        let (owner, account) = start_store_owner_with_account();
        let host = owner.host();
        let (operation, transition) = ImportStoreActivationTransition::begin(&owner.state)
            .expect("the active import operation must enter one Scene transition");

        drop(operation);
        drop(transition);

        assert!(matches!(
            host.poll(),
            Err(ImportStoreHostError::OperationContained)
        ));
        assert!(matches!(
            owner.prepare_scene(None),
            Err(ImportStoreActivationError::StoreUnavailable(
                ImportStoreHostError::OperationContained
            ))
        ));
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
    }

    #[test]
    fn bootstrap_rejects_aggregate_policy_overflow_before_building_store_plans() {
        let limits = ResourceLimits {
            max_detection_work: u64::MAX,
            max_work_units: u64::MAX,
            ..ResourceLimits::default()
        };
        let requirements = ImportStoreBootstrapRequirements::new(
            2,
            2,
            1,
            ImportStoreWorkBudget::new(u64::MAX, u64::MAX, 1, 1),
            1,
            4,
            4,
        )
        .expect("the startup envelope is structurally valid");

        assert!(matches!(
            try_import_store_bootstrap(&limits, requirements),
            Err(ImportStoreBootstrapError::StorePlan(
                StorePlanError::SizeOverflow
            ))
        ));
    }

    #[test]
    fn bootstrap_rejects_a_total_that_does_not_leave_the_exact_ordinary_remainder() {
        let limits = ResourceLimits::default();
        let (ordinary_attempted, ordinary_consumed) = ordinary_work_limits(&limits);
        let structural_work = 4;
        let protected =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                .expect("test protected terminal work is valid");
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted + structural_work + protected.attempted_work_limit() - 1,
            ordinary_consumed + structural_work + protected.consumed_work_limit(),
            1,
            1,
        );
        let requirements =
            ImportStoreBootstrapRequirements::new(2, 2, 1, work, structural_work, 4, 4)
                .expect("the writer and publication envelope is valid");

        assert!(matches!(
            try_import_store_bootstrap(&limits, requirements),
            Err(ImportStoreBootstrapError::OrdinaryWorkPartitionMismatch {
                actual_attempted,
                expected_attempted,
                actual_consumed,
                expected_consumed,
            }) if actual_attempted == ordinary_attempted + structural_work
                && expected_attempted == ordinary_attempted + structural_work - 1
                && actual_consumed == ordinary_consumed + structural_work
                && expected_consumed == ordinary_consumed + structural_work
        ));
    }

    #[test]
    fn bootstrap_rejects_a_consumed_total_below_the_exact_ordinary_remainder() {
        let limits = ResourceLimits::default();
        let (ordinary_attempted, ordinary_consumed) = ordinary_work_limits(&limits);
        let structural_work = 4;
        let protected =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                .expect("test protected terminal work is valid");
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted + structural_work + protected.attempted_work_limit(),
            ordinary_consumed + structural_work + protected.consumed_work_limit() - 1,
            1,
            1,
        );
        let requirements = ImportStoreBootstrapRequirements::new(
            2,
            2,
            1,
            work,
            structural_work,
            4,
            STARTUP_OUTCOME_EVIDENCE_TOTAL,
        )
        .expect("the writer and publication envelope is valid");

        assert!(matches!(
            try_import_store_bootstrap(&limits, requirements),
            Err(ImportStoreBootstrapError::OrdinaryWorkPartitionMismatch {
                actual_attempted,
                expected_attempted,
                actual_consumed,
                expected_consumed,
            }) if actual_attempted == ordinary_attempted + structural_work
                && expected_attempted == ordinary_attempted + structural_work
                && actual_consumed == ordinary_consumed + structural_work
                && expected_consumed == ordinary_consumed + structural_work - 1
        ));
    }

    #[test]
    fn bootstrap_rejects_an_outcome_evidence_limit_below_the_startup_commitments() {
        let work = ImportStoreWorkBudget::new(2, 2, 0, 0);

        assert!(matches!(
            ImportStoreBootstrapRequirements::new(
                2,
                2,
                1,
                work,
                1,
                4,
                STARTUP_OUTCOME_EVIDENCE_TOTAL - 1,
            ),
            Err(ImportStoreBootstrapError::PublishedOutcomeEvidenceLimitTooSmall {
                limit,
                required,
            }) if limit == STARTUP_OUTCOME_EVIDENCE_TOTAL - 1
                && required == STARTUP_OUTCOME_EVIDENCE_TOTAL
        ));
    }

    #[test]
    fn canonical_retained_member_policy_preserves_the_exact_caller_limit() {
        let requirements = ImportStoreBootstrapRequirements::new(
            2,
            2,
            1,
            ImportStoreWorkBudget::new(2, 2, 0, 0),
            1,
            7,
            STARTUP_OUTCOME_EVIDENCE_TOTAL,
        )
        .expect("a nonzero retained-member limit is valid");
        let policy = requirements.canonical_retained_member_policy();

        assert_eq!(policy.attempted_limit(), 0);
        assert_eq!(policy.consumed_limit(), 0);
        assert_eq!(policy.committed_limit(), 7);
        assert_eq!(policy.published_outcome_evidence_limit(), 0);
        assert_eq!(policy.live_byte_limit(), None);
    }

    #[test]
    fn bootstrap_rejects_a_zero_canonical_retained_member_limit() {
        assert!(matches!(
            ImportStoreBootstrapRequirements::new(
                2,
                2,
                1,
                ImportStoreWorkBudget::new(2, 2, 0, 0),
                1,
                MIN_CANONICAL_RETAINED_MEMBER_LIMIT - 1,
                STARTUP_OUTCOME_EVIDENCE_TOTAL,
            ),
            Err(
                ImportStoreBootstrapError::PublishedCanonicalRetainedMemberLimitTooSmall {
                    limit,
                    required,
                }
            ) if limit == 0 && required == MIN_CANONICAL_RETAINED_MEMBER_LIMIT
        ));
    }

    #[test]
    fn bootstrap_rejects_a_zero_canonical_structural_work_limit() {
        assert!(matches!(
            ImportStoreBootstrapRequirements::new(
                2,
                2,
                1,
                ImportStoreWorkBudget::new(2, 2, 0, 0),
                0,
                MIN_CANONICAL_RETAINED_MEMBER_LIMIT,
                STARTUP_OUTCOME_EVIDENCE_TOTAL,
            ),
            Err(ImportStoreBootstrapError::StorePlan(
                StorePlanError::EmptyPayload
            ))
        ));
    }

    #[test]
    fn canonical_structural_work_policy_preserves_the_exact_private_limit() {
        let requirements = ImportStoreBootstrapRequirements::new(
            2,
            2,
            1,
            ImportStoreWorkBudget::new(7, 7, 0, 0),
            7,
            MIN_CANONICAL_RETAINED_MEMBER_LIMIT,
            STARTUP_OUTCOME_EVIDENCE_TOTAL,
        )
        .expect("a nonzero structural-work limit is valid");
        let policy = requirements.canonical_structural_work_policy();

        assert_eq!(policy.attempted_limit(), 7);
        assert_eq!(policy.consumed_limit(), 7);
        assert_eq!(policy.committed_limit(), 0);
        assert_eq!(policy.published_outcome_evidence_limit(), 0);
        assert_eq!(policy.live_byte_limit(), None);
    }

    #[test]
    fn private_store_roles_are_reserved_outside_public_import_dimensions() {
        assert_eq!(STARTUP_OUTCOME_EVIDENCE_TOTAL, 2);
        let private_roles = [
            STARTUP_OUTCOME_EVIDENCE_SEMANTIC,
            FAILURE_TERMINAL_WORK_SEMANTIC,
            CANONICAL_RETAINED_MEMBERS_SEMANTIC,
            CANONICAL_STRUCTURAL_WORK_SEMANTIC,
        ];
        for dimension in ImportDimension::AssetOpen as u8..=ImportDimension::DecompressedBytes as u8
        {
            for role in private_roles {
                assert_ne!(u64::from(dimension) + 1, role.local().get());
            }
        }
        for (index, role) in private_roles.iter().enumerate() {
            for other in &private_roles[index + 1..] {
                assert_ne!(role, other);
            }
        }
    }
}
