use std::cell::{RefCell, RefMut};
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};

use baozi_operation::__canonical_activation::{
    RetainedStoreDimensionState, RetainedStoreObservedWork,
};
use baozi_operation::{
    DimensionPolicy, LedgerLimits, LedgerLimitsError, PhysicalStorageAccount,
    RetainedStoreBatchLimits, RetainedStoreFailureTerminalWorkPlan, RetainedStoreOperation,
    RetainedStoreReservation, RetainedStoreSharePlan, RetainedStoreStartError,
    RetainedStoreStartPlan, RetainedStoreWriterPlan, StoreDimensionKey, StoreDimensionSemanticId,
    StoreFallbackEnvelope, StoreKeyDomain, StoreKeyDomainCreateError, StoreLedgerPlan,
    StoreLedgerPlanError, StorePhaseKey, StorePhaseSemanticId, StorePlanError, StoreRuntimeError,
    StoreSemanticCatalogId,
};

use crate::ledger::{
    ImportDimension, ImportLedgerPhase, ImportOperationControl, ImportOperationOwner,
    ImportProgressPhase, ImportProgressUnit, import_ledger_limits,
};
use crate::limits::ResourceLimits;

const IMPORT_STORE_CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-import-led", NonZeroU32::MIN);
const IMPORT_DIMENSION_COUNT: usize = ImportDimension::DecompressedBytes as usize + 1;
const IMPORT_STORE_FALLBACK_CODE: u32 = 0x494d_0001;
const MIN_CANONICAL_RETAINED_MEMBER_LIMIT: u64 = 1;
const STARTUP_OUTCOME_EVIDENCE_AMOUNT: u64 = 1;
const STARTUP_OUTCOME_EVIDENCE_TOTAL: u64 = STARTUP_OUTCOME_EVIDENCE_AMOUNT * 2;

type ImportStoreOperation =
    RetainedStoreOperation<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

pub(crate) type ImportStoreReservation = RetainedStoreReservation<ImportOperationOwner>;

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
        canonical_retained_member_limit: u64,
        published_outcome_evidence_limit: u64,
    ) -> Result<Self, ImportStoreBootstrapError> {
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
            canonical_retained_member_limit,
            published_outcome_evidence_limit,
        })
    }

    const fn canonical_retained_member_policy(&self) -> DimensionPolicy {
        DimensionPolicy::counter(0, 0, self.canonical_retained_member_limit.get())
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
        Ok(Self {
            phases,
            dimensions,
            startup_outcome_evidence,
            failure_terminal_work,
            canonical_retained_members,
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

    pub(crate) const fn outcome_evidence_dimension(&self) -> StoreDimensionKey {
        self.startup_outcome_evidence
    }

    pub(crate) const fn canonical_retained_member_dimension(&self) -> StoreDimensionKey {
        self.canonical_retained_members
    }

    fn phase_keys(&self) -> [StorePhaseKey; ImportLedgerPhase::ALL.len()] {
        self.phases.map(|(_, key)| key)
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
/// physical account. Consuming this owner is the only path into the later scoped canonical
/// outcome transaction.
pub(crate) struct ImportStoreSessionOwner {
    operation: RefCell<ImportStoreOperation>,
    vocabulary: ImportStoreVocabulary,
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
            operation: RefCell::new(operation),
            vocabulary,
        })
    }

    pub(crate) fn host(&self) -> ImportStoreHost<'_> {
        ImportStoreHost {
            operation: &self.operation,
            vocabulary: &self.vocabulary,
        }
    }

    pub(crate) fn into_operation(self) -> ImportStoreOperation {
        self.operation.into_inner()
    }
}

/// Borrowed import-semantic producer capability backed by the session's sole retained operation.
///
/// This adapter never exposes operation-native keys, publication authority, terminal dimensions,
/// or the physical account. Observed-work permits remain inside one closure so they cannot escape
/// the `RefCell` borrow that owns the operation.
#[derive(Clone, Copy)]
pub(crate) struct ImportStoreHost<'session> {
    operation: &'session RefCell<ImportStoreOperation>,
    vocabulary: &'session ImportStoreVocabulary,
}

#[allow(
    clippy::result_large_err,
    reason = "producer failures retain exact typed store causes without allocating on error paths"
)]
impl ImportStoreHost<'_> {
    fn try_operation(&self) -> Result<RefMut<'_, ImportStoreOperation>, ImportStoreHostError> {
        self.operation
            .try_borrow_mut()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)
    }

    pub(crate) fn poll(&self) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .producer_host()
            .poll()
            .map_err(ImportStoreHostError::Runtime)
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
            .map_err(ImportStoreHostError::Runtime)
    }

    pub(crate) fn dimension_state(
        &self,
        dimension: ImportDimension,
    ) -> Result<RetainedStoreDimensionState, ImportStoreHostError> {
        let key = self.vocabulary.dimension(dimension);
        self.try_operation()?
            .producer_host()
            .dimension_state(key)
            .map_err(ImportStoreHostError::Runtime)
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
            .map_err(ImportStoreHostError::Runtime)
    }

    pub(crate) fn resize(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ImportStoreHostError> {
        self.try_operation()?
            .control_checkpoint()
            .resize(reservation, bytes)
            .map_err(ImportStoreHostError::Runtime)
    }

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
            .transition(reservation, phase, dimension, bytes)
            .map_err(ImportStoreHostError::Runtime)
    }

    pub(crate) fn complete_phase(
        &self,
        phase: ImportLedgerPhase,
    ) -> Result<(), ImportStoreHostError> {
        let operation = self
            .operation
            .try_borrow()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        operation
            .try_complete_phase(self.vocabulary.phase(phase))
            .map_err(ImportStoreHostError::Runtime)
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
        let raw = producer.try_begin_observed_work(charges).map_err(|cause| {
            ImportObservedWorkFailure::Store(ImportStoreHostError::Runtime(cause))
        })?;
        let mut work = ImportObservedWork { raw };
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
}

#[allow(
    clippy::result_large_err,
    reason = "observed-work errors preserve exact ledger and control causes without boxing"
)]
impl<const N: usize> ImportObservedWork<'_, N> {
    pub(crate) fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        self.raw.poll()
    }

    pub(crate) fn report_progress(
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), StoreRuntimeError> {
        self.raw.report_progress(phase, unit, completed, total)
    }

    pub(crate) fn remaining(&self) -> [u64; N] {
        self.raw.remaining()
    }

    pub(crate) fn complete(&mut self, amounts: [u64; N]) -> Result<(), StoreRuntimeError> {
        self.raw.complete(amounts)
    }

    fn finish(self) -> Result<(), ImportStoreHostError> {
        self.raw.finish().map_err(ImportStoreHostError::Runtime)
    }

    fn finish_partial(self) {
        self.raw.finish_partial();
    }
}

#[derive(Debug)]
pub(crate) enum ImportStoreHostError {
    ReentrantBorrow,
    Runtime(StoreRuntimeError),
}

impl fmt::Display for ImportStoreHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReentrantBorrow => {
                formatter.write_str("the import retained-store host was borrowed reentrantly")
            }
            Self::Runtime(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportStoreHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReentrantBorrow => None,
            Self::Runtime(cause) => Some(cause),
        }
    }
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
            vocabulary.dimension(ImportDimension::WorkUnits),
            vocabulary.dimension(ImportDimension::WorkUnits),
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
    use baozi_operation::{
        PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits, RetainedStoreOperation,
    };

    use crate::ledger::{
        ImportOperationControl, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit,
    };

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
        let (ordinary_attempted, ordinary_consumed) = ordinary_work_limits(limits);
        let protected =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                .expect("test protected terminal work is valid");
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted
                .checked_add(protected.attempted_work_limit())
                .expect("test attempted total fits u64"),
            ordinary_consumed
                .checked_add(protected.consumed_work_limit())
                .expect("test consumed total fits u64"),
            1,
            1,
        );
        ImportStoreBootstrapRequirements::new(2, 2, 1, work, 4, STARTUP_OUTCOME_EVIDENCE_TOTAL)
            .expect("test retained-store requirements are valid")
    }

    fn requirements() -> ImportStoreBootstrapRequirements {
        requirements_for(&ResourceLimits::default())
    }

    fn start_store_owner() -> ImportStoreSessionOwner {
        let bootstrap = try_import_store_bootstrap(&ResourceLimits::default(), requirements())
            .expect("default import store bootstrap must be valid");
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
        let owner = ImportStoreSessionOwner::try_start(
            ImportOperationControl::default(),
            &account,
            bootstrap,
        )
        .expect("the import store owner must start");
        drop(account);
        drop(factory);
        owner
    }

    #[test]
    fn bootstrap_maps_the_complete_import_vocabulary_into_one_store_domain() {
        let bootstrap = try_import_store_bootstrap(&ResourceLimits::default(), requirements())
            .expect("default import store bootstrap must be valid");
        let (vocabulary, ledger_plan, _start_plan) = bootstrap.into_parts();

        assert_eq!(ledger_plan.phase_count(), ImportLedgerPhase::ALL.len());
        assert_eq!(ledger_plan.dimension_count(), IMPORT_DIMENSION_COUNT + 3);
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
    fn session_owner_keeps_one_operation_alive_after_the_start_handle_is_released() {
        let owner = start_store_owner();
        let host = owner.host();

        host.poll()
            .expect("the retained operation remains live after the account handle is released");
        host.report_progress(
            ImportProgressPhase::AcquirePrimary,
            ImportProgressUnit::Bytes,
            0,
            Some(1),
        )
        .expect("the sole retained coordinator accepts import progress");

        let published = owner
            .into_operation()
            .prepare_minimal_fallback(IMPORT_STORE_FALLBACK_CODE)
            .expect("the session owner must retain its pre-admitted terminal path")
            .handoff();
        assert_eq!(
            published.fallback.value().code(),
            IMPORT_STORE_FALLBACK_CODE
        );
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

        host.resize(&mut reservation, 16)
            .expect("the same host must resize its reservation");
        host.transition(
            &mut reservation,
            ImportLedgerPhase::DependencyAcquisition,
            ImportDimension::DependencyBytes,
            12,
        )
        .expect("the same host must transition its reservation");
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
    fn host_rejects_reentrant_operation_borrows_without_panicking() {
        let owner = start_store_owner();
        let host = owner.host();
        let _operation = owner.operation.borrow_mut();

        assert!(matches!(
            host.poll(),
            Err(ImportStoreHostError::ReentrantBorrow)
        ));
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
        let protected =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                .expect("test protected terminal work is valid");
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted + protected.attempted_work_limit() - 1,
            ordinary_consumed + protected.consumed_work_limit(),
            1,
            1,
        );
        let requirements = ImportStoreBootstrapRequirements::new(2, 2, 1, work, 4, 4)
            .expect("the writer and publication envelope is valid");

        assert!(matches!(
            try_import_store_bootstrap(&limits, requirements),
            Err(ImportStoreBootstrapError::OrdinaryWorkPartitionMismatch {
                actual_attempted,
                expected_attempted,
                actual_consumed,
                expected_consumed,
            }) if actual_attempted == ordinary_attempted
                && expected_attempted == ordinary_attempted - 1
                && actual_consumed == ordinary_consumed
                && expected_consumed == ordinary_consumed
        ));
    }

    #[test]
    fn bootstrap_rejects_a_consumed_total_below_the_exact_ordinary_remainder() {
        let limits = ResourceLimits::default();
        let (ordinary_attempted, ordinary_consumed) = ordinary_work_limits(&limits);
        let protected =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                .expect("test protected terminal work is valid");
        let work = ImportStoreWorkBudget::new(
            ordinary_attempted + protected.attempted_work_limit(),
            ordinary_consumed + protected.consumed_work_limit() - 1,
            1,
            1,
        );
        let requirements =
            ImportStoreBootstrapRequirements::new(2, 2, 1, work, 4, STARTUP_OUTCOME_EVIDENCE_TOTAL)
                .expect("the writer and publication envelope is valid");

        assert!(matches!(
            try_import_store_bootstrap(&limits, requirements),
            Err(ImportStoreBootstrapError::OrdinaryWorkPartitionMismatch {
                actual_attempted,
                expected_attempted,
                actual_consumed,
                expected_consumed,
            }) if actual_attempted == ordinary_attempted
                && expected_attempted == ordinary_attempted
                && actual_consumed == ordinary_consumed
                && expected_consumed == ordinary_consumed - 1
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
    fn private_store_roles_are_reserved_outside_public_import_dimensions() {
        assert_eq!(STARTUP_OUTCOME_EVIDENCE_TOTAL, 2);
        let private_roles = [
            STARTUP_OUTCOME_EVIDENCE_SEMANTIC,
            FAILURE_TERMINAL_WORK_SEMANTIC,
            CANONICAL_RETAINED_MEMBERS_SEMANTIC,
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
