use crate::ledger::{DimensionSnapshot, DimensionSnapshotScalars, LedgerSnapshot, PhaseSnapshot};
use crate::{CapacityPlan, CapacityPlanError};
use std::alloc::Layout;
#[cfg(test)]
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub(super) const TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION: u32 = 1;

pub(crate) trait TerminalEvidenceKeySeal {}

pub(crate) trait TerminalEvidenceKey:
    TerminalEvidenceKeySeal + Copy + Ord + Send + Sync + 'static
{
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_DIMENSION_BACKING_ALLOCATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_NEXT_PHASE_BACKING_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
fn fail_next_dimension_backing_allocation() {
    FAIL_NEXT_DIMENSION_BACKING_ALLOCATION.with(|fail| fail.set(true));
}

#[cfg(test)]
fn fail_next_phase_backing_allocation() {
    FAIL_NEXT_PHASE_BACKING_ALLOCATION.with(|fail| fail.set(true));
}

#[cfg(test)]
fn take_dimension_backing_allocation_failure() -> bool {
    FAIL_NEXT_DIMENSION_BACKING_ALLOCATION.with(|fail| fail.replace(false))
}

#[cfg(test)]
fn take_phase_backing_allocation_failure() -> bool {
    FAIL_NEXT_PHASE_BACKING_ALLOCATION.with(|fail| fail.replace(false))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PreparationFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

impl PreparationFailureClass {
    const fn atomic_tag(self) -> u8 {
        match self {
            Self::Requested => 1,
            Self::Control => 2,
            Self::ProductGraph => 3,
            Self::Ledger => 4,
        }
    }

    const fn from_atomic_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Requested),
            2 => Some(Self::Control),
            3 => Some(Self::ProductGraph),
            4 => Some(Self::Ledger),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparationFailureCode {
    class: PreparationFailureClass,
    code: u32,
}

impl PreparationFailureCode {
    const fn from_parts(class: PreparationFailureClass, code: u32) -> Self {
        Self { class, code }
    }
    pub const fn domain(code: u32) -> Self {
        Self {
            class: PreparationFailureClass::Requested,
            code,
        }
    }

    pub(super) const fn requested() -> Self {
        Self {
            class: PreparationFailureClass::Requested,
            code: 0,
        }
    }

    pub(super) const fn control(code: u32) -> Self {
        Self {
            class: PreparationFailureClass::Control,
            code,
        }
    }

    pub(super) const fn product_graph(code: u32) -> Self {
        Self {
            class: PreparationFailureClass::ProductGraph,
            code,
        }
    }

    pub(super) const fn ledger() -> Self {
        Self {
            class: PreparationFailureClass::Ledger,
            code: 1,
        }
    }

    pub const fn class(self) -> PreparationFailureClass {
        self.class
    }

    pub const fn code(self) -> u32 {
        self.code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalEvidenceCause {
    Success,
    Failure(PreparationFailureCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FallbackHeaderSnapshot<P: TerminalEvidenceKey> {
    schema_revision: u32,
    terminal_cause: TerminalEvidenceCause,
    last_completed_phase: Option<P>,
    dimension_count: usize,
    phase_count: usize,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

impl<P: TerminalEvidenceKey> FallbackHeaderSnapshot<P> {
    pub(super) const fn schema_revision(self) -> u32 {
        self.schema_revision
    }

    pub(super) const fn terminal_cause(self) -> TerminalEvidenceCause {
        self.terminal_cause
    }

    pub(super) const fn last_completed_phase(self) -> Option<P> {
        self.last_completed_phase
    }

    pub(super) const fn dimension_count(self) -> usize {
        self.dimension_count
    }

    pub(super) const fn phase_count(self) -> usize {
        self.phase_count
    }

    pub(super) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    pub(super) const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    pub(super) const fn active_reservations(self) -> u64 {
        self.active_reservations
    }

    pub(super) const fn current_evidence_bytes(self) -> u64 {
        self.current_evidence_bytes
    }

    pub(super) const fn peak_evidence_bytes(self) -> u64 {
        self.peak_evidence_bytes
    }
}

const TERMINAL_CAUSE_UNSET: u8 = 0;
const TERMINAL_CAUSE_SUCCESS: u8 = 1;
const TERMINAL_CAUSE_FAILURE: u8 = 2;

pub(super) struct FallbackHeader<P: TerminalEvidenceKey> {
    schema_revision: u32,
    dimension_count: usize,
    phase_count: usize,
    terminal_cause: AtomicU8,
    failure_class: AtomicU8,
    failure_code: AtomicU32,
    last_completed_phase_ordinal_plus_one: AtomicUsize,
    current_live_bytes: AtomicU64,
    peak_live_bytes: AtomicU64,
    active_reservations: AtomicU64,
    current_evidence_bytes: AtomicU64,
    peak_evidence_bytes: AtomicU64,
    key_type: PhantomData<P>,
}

impl<P: TerminalEvidenceKey> FallbackHeader<P> {
    const fn new(dimension_count: usize, phase_count: usize) -> Self {
        Self {
            schema_revision: TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION,
            dimension_count,
            phase_count,
            terminal_cause: AtomicU8::new(TERMINAL_CAUSE_UNSET),
            failure_class: AtomicU8::new(0),
            failure_code: AtomicU32::new(0),
            last_completed_phase_ordinal_plus_one: AtomicUsize::new(0),
            current_live_bytes: AtomicU64::new(0),
            peak_live_bytes: AtomicU64::new(0),
            active_reservations: AtomicU64::new(0),
            current_evidence_bytes: AtomicU64::new(0),
            peak_evidence_bytes: AtomicU64::new(0),
            key_type: PhantomData,
        }
    }

    fn write<D: TerminalEvidenceKey>(
        &self,
        terminal_cause: TerminalEvidenceCause,
        last_completed_phase_ordinal_plus_one: usize,
        snapshot: &LedgerSnapshot<P, D>,
    ) {
        let cause_tag = match terminal_cause {
            TerminalEvidenceCause::Success => TERMINAL_CAUSE_SUCCESS,
            TerminalEvidenceCause::Failure(cause) => {
                self.failure_class
                    .store(cause.class().atomic_tag(), Ordering::Relaxed);
                self.failure_code.store(cause.code(), Ordering::Relaxed);
                TERMINAL_CAUSE_FAILURE
            }
        };
        self.last_completed_phase_ordinal_plus_one
            .store(last_completed_phase_ordinal_plus_one, Ordering::Relaxed);
        self.current_live_bytes
            .store(snapshot.current_live_bytes(), Ordering::Relaxed);
        self.peak_live_bytes
            .store(snapshot.peak_live_bytes(), Ordering::Relaxed);
        self.active_reservations
            .store(snapshot.active_reservations(), Ordering::Relaxed);
        self.current_evidence_bytes
            .store(snapshot.current_evidence_bytes(), Ordering::Relaxed);
        self.peak_evidence_bytes
            .store(snapshot.peak_evidence_bytes(), Ordering::Relaxed);
        self.terminal_cause.store(cause_tag, Ordering::Relaxed);
    }

    fn snapshot(&self, phases: &[FallbackPhaseCell<P>]) -> Option<FallbackHeaderSnapshot<P>> {
        let terminal_cause = match self.terminal_cause.load(Ordering::Relaxed) {
            TERMINAL_CAUSE_SUCCESS => TerminalEvidenceCause::Success,
            TERMINAL_CAUSE_FAILURE => {
                let class = PreparationFailureClass::from_atomic_tag(
                    self.failure_class.load(Ordering::Relaxed),
                )?;
                TerminalEvidenceCause::Failure(PreparationFailureCode::from_parts(
                    class,
                    self.failure_code.load(Ordering::Relaxed),
                ))
            }
            _ => return None,
        };
        let ordinal = self
            .last_completed_phase_ordinal_plus_one
            .load(Ordering::Relaxed);
        let last_completed_phase = if ordinal == 0 {
            None
        } else {
            Some(phases.get(ordinal.checked_sub(1)?)?.key())
        };
        Some(FallbackHeaderSnapshot {
            schema_revision: self.schema_revision,
            terminal_cause,
            last_completed_phase,
            dimension_count: self.dimension_count,
            phase_count: self.phase_count,
            current_live_bytes: self.current_live_bytes.load(Ordering::Relaxed),
            peak_live_bytes: self.peak_live_bytes.load(Ordering::Relaxed),
            active_reservations: self.active_reservations.load(Ordering::Relaxed),
            current_evidence_bytes: self.current_evidence_bytes.load(Ordering::Relaxed),
            peak_evidence_bytes: self.peak_evidence_bytes.load(Ordering::Relaxed),
        })
    }
}

pub(super) struct FallbackDimensionCell<D: TerminalEvidenceKey> {
    key: D,
    attempted: AtomicU64,
    consumed: AtomicU64,
    current_live_bytes: AtomicU64,
    peak_live_bytes: AtomicU64,
    pending_publication: AtomicU64,
    committed: AtomicU64,
    committed_canonical_output: AtomicU64,
    published_outcome_evidence: AtomicU64,
}

impl<D: TerminalEvidenceKey> FallbackDimensionCell<D> {
    const fn new(key: D) -> Self {
        Self {
            key,
            attempted: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            current_live_bytes: AtomicU64::new(0),
            peak_live_bytes: AtomicU64::new(0),
            pending_publication: AtomicU64::new(0),
            committed: AtomicU64::new(0),
            committed_canonical_output: AtomicU64::new(0),
            published_outcome_evidence: AtomicU64::new(0),
        }
    }

    pub(super) const fn key(&self) -> D {
        self.key
    }

    fn write(&self, snapshot: &DimensionSnapshot) {
        self.attempted
            .store(snapshot.attempted(), Ordering::Relaxed);
        self.consumed.store(snapshot.consumed(), Ordering::Relaxed);
        self.current_live_bytes
            .store(snapshot.current_live_bytes(), Ordering::Relaxed);
        self.peak_live_bytes
            .store(snapshot.peak_live_bytes(), Ordering::Relaxed);
        self.pending_publication
            .store(snapshot.pending_publication(), Ordering::Relaxed);
        self.committed
            .store(snapshot.committed(), Ordering::Relaxed);
        self.committed_canonical_output
            .store(snapshot.committed_canonical_output(), Ordering::Relaxed);
        self.published_outcome_evidence
            .store(snapshot.published_outcome_evidence(), Ordering::Relaxed);
    }

    fn snapshot(&self) -> DimensionSnapshot {
        DimensionSnapshot::from_terminal_evidence(DimensionSnapshotScalars {
            attempted: self.attempted.load(Ordering::Relaxed),
            consumed: self.consumed.load(Ordering::Relaxed),
            current_live_bytes: self.current_live_bytes.load(Ordering::Relaxed),
            peak_live_bytes: self.peak_live_bytes.load(Ordering::Relaxed),
            pending_publication: self.pending_publication.load(Ordering::Relaxed),
            committed: self.committed.load(Ordering::Relaxed),
            committed_canonical_output: self.committed_canonical_output.load(Ordering::Relaxed),
            published_outcome_evidence: self.published_outcome_evidence.load(Ordering::Relaxed),
        })
    }
}

pub(super) struct FallbackPhaseCell<P: TerminalEvidenceKey> {
    key: P,
    current_live_bytes: AtomicU64,
    peak_live_bytes: AtomicU64,
}

impl<P: TerminalEvidenceKey> FallbackPhaseCell<P> {
    const fn new(key: P) -> Self {
        Self {
            key,
            current_live_bytes: AtomicU64::new(0),
            peak_live_bytes: AtomicU64::new(0),
        }
    }

    pub(super) const fn key(&self) -> P {
        self.key
    }

    fn write(&self, snapshot: &PhaseSnapshot) {
        self.current_live_bytes
            .store(snapshot.current_live_bytes(), Ordering::Relaxed);
        self.peak_live_bytes
            .store(snapshot.peak_live_bytes(), Ordering::Relaxed);
    }

    fn snapshot(&self) -> PhaseSnapshot {
        PhaseSnapshot::from_terminal_evidence(
            self.current_live_bytes.load(Ordering::Relaxed),
            self.peak_live_bytes.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalEvidenceFreezeError {
    AlreadyFrozen,
    ShapeMismatch {
        expected_dimensions: usize,
        actual_dimensions: usize,
        expected_phases: usize,
        actual_phases: usize,
    },
    DimensionKeyMismatch {
        index: usize,
    },
    PhaseKeyMismatch {
        index: usize,
    },
    LastCompletedPhaseUnknown,
    LastCompletedPhaseOrdinalOverflow,
    NonTerminalGlobalState {
        current_live_bytes: u64,
        active_reservations: u64,
        current_evidence_bytes: u64,
    },
    NonTerminalDimensionState {
        index: usize,
        current_live_bytes: u64,
        pending_publication: u64,
    },
    NonTerminalPhaseState {
        index: usize,
        current_live_bytes: u64,
    },
}

impl fmt::Display for TerminalEvidenceFreezeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyFrozen => {
                formatter.write_str("terminal ledger evidence is already frozen")
            }
            Self::ShapeMismatch {
                expected_dimensions,
                actual_dimensions,
                expected_phases,
                actual_phases,
            } => write!(
                formatter,
                "terminal ledger evidence expected {expected_dimensions}/{expected_phases} dimension/phase entries, observed {actual_dimensions}/{actual_phases}"
            ),
            Self::DimensionKeyMismatch { index } => write!(
                formatter,
                "terminal ledger evidence dimension key {index} does not match its frozen catalog"
            ),
            Self::PhaseKeyMismatch { index } => write!(
                formatter,
                "terminal ledger evidence phase key {index} does not match its frozen catalog"
            ),
            Self::LastCompletedPhaseUnknown => formatter.write_str(
                "terminal ledger evidence last-completed phase is absent from its frozen catalog",
            ),
            Self::LastCompletedPhaseOrdinalOverflow => formatter
                .write_str("terminal ledger evidence last-completed phase ordinal overflowed"),
            Self::NonTerminalGlobalState {
                current_live_bytes,
                active_reservations,
                current_evidence_bytes,
            } => write!(
                formatter,
                "terminal ledger evidence still has {current_live_bytes} live bytes, {active_reservations} reservations, and {current_evidence_bytes} evidence bytes"
            ),
            Self::NonTerminalDimensionState {
                index,
                current_live_bytes,
                pending_publication,
            } => write!(
                formatter,
                "terminal ledger dimension {index} still has {current_live_bytes} live bytes and {pending_publication} pending publication units"
            ),
            Self::NonTerminalPhaseState {
                index,
                current_live_bytes,
            } => write!(
                formatter,
                "terminal ledger phase {index} still has {current_live_bytes} live bytes"
            ),
        }
    }
}

impl std::error::Error for TerminalEvidenceFreezeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalEvidenceCatalogError {
    DimensionKeysNotStrictlyOrdered { index: usize },
    PhaseKeysNotStrictlyOrdered { index: usize },
}

impl fmt::Display for TerminalEvidenceCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionKeysNotStrictlyOrdered { index } => write!(
                formatter,
                "terminal dimension key {index} does not strictly follow its predecessor"
            ),
            Self::PhaseKeysNotStrictlyOrdered { index } => write!(
                formatter,
                "terminal phase key {index} does not strictly follow its predecessor"
            ),
        }
    }
}

impl std::error::Error for TerminalEvidenceCatalogError {}

pub(super) struct TerminalLedgerEvidence<P: TerminalEvidenceKey, D: TerminalEvidenceKey> {
    header: FallbackHeader<P>,
    dimensions: Vec<FallbackDimensionCell<D>>,
    phases: Vec<FallbackPhaseCell<P>>,
    freeze_started: AtomicBool,
    final_frozen: AtomicBool,
}

impl<P, D> TerminalLedgerEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    fn from_validated_cells(
        dimensions: Vec<FallbackDimensionCell<D>>,
        phases: Vec<FallbackPhaseCell<P>>,
    ) -> Self {
        let dimension_count = dimensions.len();
        let phase_count = phases.len();
        Self {
            header: FallbackHeader::new(dimension_count, phase_count),
            dimensions,
            phases,
            freeze_started: AtomicBool::new(false),
            final_frozen: AtomicBool::new(false),
        }
    }

    pub(super) fn try_freeze_from(
        &self,
        terminal_cause: TerminalEvidenceCause,
        snapshot: &LedgerSnapshot<P, D>,
    ) -> Result<(), TerminalEvidenceFreezeError> {
        if self.freeze_started.load(Ordering::Acquire) || self.final_frozen.load(Ordering::Acquire)
        {
            return Err(TerminalEvidenceFreezeError::AlreadyFrozen);
        }

        let actual_dimensions = snapshot.dimensions().len();
        let actual_phases = snapshot.phases().len();
        if self.dimensions.len() != actual_dimensions || self.phases.len() != actual_phases {
            return Err(TerminalEvidenceFreezeError::ShapeMismatch {
                expected_dimensions: self.dimensions.len(),
                actual_dimensions,
                expected_phases: self.phases.len(),
                actual_phases,
            });
        }

        for (index, (cell, (key, _))) in self
            .dimensions
            .iter()
            .zip(snapshot.dimensions())
            .enumerate()
        {
            if cell.key != key {
                return Err(TerminalEvidenceFreezeError::DimensionKeyMismatch { index });
            }
        }
        for (index, (cell, (key, _))) in self.phases.iter().zip(snapshot.phases()).enumerate() {
            if cell.key != key {
                return Err(TerminalEvidenceFreezeError::PhaseKeyMismatch { index });
            }
        }
        let last_completed_phase_ordinal_plus_one = match snapshot.last_completed_phase() {
            Some(phase) => self
                .phases
                .binary_search_by_key(&phase, FallbackPhaseCell::key)
                .map_err(|_| TerminalEvidenceFreezeError::LastCompletedPhaseUnknown)?
                .checked_add(1)
                .ok_or(TerminalEvidenceFreezeError::LastCompletedPhaseOrdinalOverflow)?,
            None => 0,
        };
        for (index, (_, value)) in snapshot.phases().enumerate() {
            if value.current_live_bytes() != 0 {
                return Err(TerminalEvidenceFreezeError::NonTerminalPhaseState {
                    index,
                    current_live_bytes: value.current_live_bytes(),
                });
            }
        }
        for (index, (_, value)) in snapshot.dimensions().enumerate() {
            if value.current_live_bytes() != 0 || value.pending_publication() != 0 {
                return Err(TerminalEvidenceFreezeError::NonTerminalDimensionState {
                    index,
                    current_live_bytes: value.current_live_bytes(),
                    pending_publication: value.pending_publication(),
                });
            }
        }
        if snapshot.current_live_bytes() != 0
            || snapshot.active_reservations() != 0
            || snapshot.current_evidence_bytes() != 0
        {
            return Err(TerminalEvidenceFreezeError::NonTerminalGlobalState {
                current_live_bytes: snapshot.current_live_bytes(),
                active_reservations: snapshot.active_reservations(),
                current_evidence_bytes: snapshot.current_evidence_bytes(),
            });
        }

        if self
            .freeze_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TerminalEvidenceFreezeError::AlreadyFrozen);
        }

        for (cell, (_, value)) in self.dimensions.iter().zip(snapshot.dimensions()) {
            cell.write(value);
        }
        for (cell, (_, value)) in self.phases.iter().zip(snapshot.phases()) {
            cell.write(value);
        }
        self.header.write(
            terminal_cause,
            last_completed_phase_ordinal_plus_one,
            snapshot,
        );
        self.final_frozen.store(true, Ordering::Release);
        Ok(())
    }

    pub(super) fn is_frozen(&self) -> bool {
        self.final_frozen.load(Ordering::Acquire)
    }

    pub(super) fn header(&self) -> Option<FallbackHeaderSnapshot<P>> {
        if !self.is_frozen() {
            return None;
        }
        self.header.snapshot(&self.phases)
    }

    pub(super) fn dimension(&self, key: D) -> Option<DimensionSnapshot> {
        if !self.is_frozen() {
            return None;
        }
        let position = self
            .dimensions
            .binary_search_by_key(&key, FallbackDimensionCell::key)
            .ok()?;
        Some(self.dimensions[position].snapshot())
    }

    pub(super) fn phase(&self, key: P) -> Option<PhaseSnapshot> {
        if !self.is_frozen() {
            return None;
        }
        let position = self
            .phases
            .binary_search_by_key(&key, FallbackPhaseCell::key)
            .ok()?;
        Some(self.phases[position].snapshot())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FallbackSemanticLayout {
    complete: Layout,
    dimension_offset: usize,
    phase_offset: usize,
}

impl FallbackSemanticLayout {
    pub(super) fn try_new<P, D>(
        dimension_count: usize,
        phase_count: usize,
    ) -> Result<Self, FallbackSemanticLayoutError>
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey,
    {
        let complete = Layout::new::<FallbackHeader<P>>();
        let dimensions = Layout::array::<FallbackDimensionCell<D>>(dimension_count)
            .map_err(|_| FallbackSemanticLayoutError::DimensionArray)?;
        let phases = Layout::array::<FallbackPhaseCell<P>>(phase_count)
            .map_err(|_| FallbackSemanticLayoutError::PhaseArray)?;
        let (complete, dimension_offset) = complete
            .extend(dimensions)
            .map_err(|_| FallbackSemanticLayoutError::DimensionExtend)?;
        let (complete, phase_offset) = complete
            .extend(phases)
            .map_err(|_| FallbackSemanticLayoutError::PhaseExtend)?;
        Ok(Self {
            complete: complete.pad_to_align(),
            dimension_offset,
            phase_offset,
        })
    }

    pub(super) const fn complete(self) -> Layout {
        self.complete
    }

    pub(super) const fn dimension_offset(self) -> usize {
        self.dimension_offset
    }

    pub(super) const fn phase_offset(self) -> usize {
        self.phase_offset
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackSemanticLayoutError {
    DimensionArray,
    PhaseArray,
    DimensionExtend,
    PhaseExtend,
}

impl fmt::Display for FallbackSemanticLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::DimensionArray => "terminal dimension-cell array layout overflowed",
            Self::PhaseArray => "terminal phase-cell array layout overflowed",
            Self::DimensionExtend => "terminal dimension-cell layout extension overflowed",
            Self::PhaseExtend => "terminal phase-cell layout extension overflowed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for FallbackSemanticLayoutError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TerminalEvidenceBackingFootprint {
    bytes: u64,
    allocations: u64,
}

impl TerminalEvidenceBackingFootprint {
    pub(super) const fn bytes(self) -> u64 {
        self.bytes
    }

    pub(super) const fn allocations(self) -> u64 {
        self.allocations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TerminalEvidenceBackingPlan<P, D> {
    dimension_capacity: CapacityPlan,
    phase_capacity: CapacityPlan,
    semantic_layout: FallbackSemanticLayout,
    footprint: TerminalEvidenceBackingFootprint,
    key_types: PhantomData<(P, D)>,
}

impl<P, D> TerminalEvidenceBackingPlan<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(super) fn try_new(
        dimension_count: usize,
        phase_count: usize,
    ) -> Result<Self, TerminalEvidenceBackingPlanError> {
        let semantic_layout = FallbackSemanticLayout::try_new::<P, D>(dimension_count, phase_count)
            .map_err(TerminalEvidenceBackingPlanError::SemanticLayout)?;
        let dimension_capacity =
            CapacityPlan::for_type::<FallbackDimensionCell<D>>(0, 0, dimension_count)
                .map_err(TerminalEvidenceBackingPlanError::DimensionBacking)?;
        let phase_capacity = CapacityPlan::for_type::<FallbackPhaseCell<P>>(0, 0, phase_count)
            .map_err(TerminalEvidenceBackingPlanError::PhaseBacking)?;
        let footprint = try_new_backing_footprint(&[dimension_capacity, phase_capacity])?;
        Ok(Self {
            dimension_capacity,
            phase_capacity,
            semantic_layout,
            footprint,
            key_types: PhantomData,
        })
    }

    pub(super) fn try_build(
        self,
        dimension_keys: &[D],
        phase_keys: &[P],
    ) -> Result<TerminalLedgerEvidence<P, D>, TerminalEvidenceBuildError> {
        // U4 startup will expose this allocation tail only through its ready two-plane guard.
        // Keeping it here binds both real Vec backings to the qualified plans without importing
        // ledger/account lifecycle authority into this leaf model.
        if dimension_keys.len() != self.dimension_capacity.desired_capacity()
            || phase_keys.len() != self.phase_capacity.desired_capacity()
        {
            return Err(TerminalEvidenceBuildError::ShapeMismatch {
                planned_dimensions: self.dimension_capacity.desired_capacity(),
                actual_dimensions: dimension_keys.len(),
                planned_phases: self.phase_capacity.desired_capacity(),
                actual_phases: phase_keys.len(),
            });
        }
        validate_key_catalogs(dimension_keys, phase_keys)
            .map_err(TerminalEvidenceBuildError::Catalog)?;

        let mut dimensions = Vec::new();
        self.dimension_capacity
            .verify_preflight(dimensions.len(), dimensions.capacity())
            .map_err(TerminalEvidenceBuildError::DimensionCapacity)?;
        #[cfg(test)]
        if take_dimension_backing_allocation_failure() {
            return Err(TerminalEvidenceBuildError::DimensionAllocationFailed);
        }
        dimensions
            .try_reserve_exact(self.dimension_capacity.allocator_additional())
            .map_err(|_| TerminalEvidenceBuildError::DimensionAllocationFailed)?;
        self.dimension_capacity
            .finish(dimensions.len(), dimensions.capacity())
            .map_err(TerminalEvidenceBuildError::DimensionCapacity)?;

        let mut phases = Vec::new();
        self.phase_capacity
            .verify_preflight(phases.len(), phases.capacity())
            .map_err(TerminalEvidenceBuildError::PhaseCapacity)?;
        #[cfg(test)]
        if take_phase_backing_allocation_failure() {
            return Err(TerminalEvidenceBuildError::PhaseAllocationFailed);
        }
        phases
            .try_reserve_exact(self.phase_capacity.allocator_additional())
            .map_err(|_| TerminalEvidenceBuildError::PhaseAllocationFailed)?;
        self.phase_capacity
            .finish(phases.len(), phases.capacity())
            .map_err(TerminalEvidenceBuildError::PhaseCapacity)?;

        dimensions.extend(
            dimension_keys
                .iter()
                .copied()
                .map(FallbackDimensionCell::new),
        );
        phases.extend(phase_keys.iter().copied().map(FallbackPhaseCell::new));
        Ok(TerminalLedgerEvidence::from_validated_cells(
            dimensions, phases,
        ))
    }

    pub(super) const fn dimension_capacity(self) -> CapacityPlan {
        self.dimension_capacity
    }

    pub(super) const fn phase_capacity(self) -> CapacityPlan {
        self.phase_capacity
    }

    pub(super) const fn semantic_layout(self) -> FallbackSemanticLayout {
        self.semantic_layout
    }

    pub(super) const fn footprint(self) -> TerminalEvidenceBackingFootprint {
        self.footprint
    }
}

fn try_new_backing_footprint(
    plans: &[CapacityPlan],
) -> Result<TerminalEvidenceBackingFootprint, TerminalEvidenceBackingPlanError> {
    let mut bytes = 0_u64;
    let mut allocations = 0_u64;
    for (index, plan) in plans.iter().copied().enumerate() {
        if plan.original_len() != 0 || plan.original_capacity() != 0 {
            return Err(TerminalEvidenceBackingPlanError::BackingPlanIsNotNew {
                index,
                original_len: plan.original_len(),
                original_capacity: plan.original_capacity(),
            });
        }
        let backing_bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| TerminalEvidenceBackingPlanError::BackingFootprint { index, cause })?;
        bytes = bytes
            .checked_add(backing_bytes)
            .ok_or(TerminalEvidenceBackingPlanError::NestedByteCountOverflow)?;
        if backing_bytes != 0 {
            allocations = allocations
                .checked_add(1)
                .ok_or(TerminalEvidenceBackingPlanError::NestedAllocationCountOverflow)?;
        }
    }
    Ok(TerminalEvidenceBackingFootprint { bytes, allocations })
}

fn validate_key_catalogs<P, D>(
    dimension_keys: &[D],
    phase_keys: &[P],
) -> Result<(), TerminalEvidenceCatalogError>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    if let Some(index) = dimension_keys
        .windows(2)
        .position(|pair| pair[0] >= pair[1])
        .map(|position| position + 1)
    {
        return Err(TerminalEvidenceCatalogError::DimensionKeysNotStrictlyOrdered { index });
    }
    if let Some(index) = phase_keys
        .windows(2)
        .position(|pair| pair[0] >= pair[1])
        .map(|position| position + 1)
    {
        return Err(TerminalEvidenceCatalogError::PhaseKeysNotStrictlyOrdered { index });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalEvidenceBackingPlanError {
    SemanticLayout(FallbackSemanticLayoutError),
    DimensionBacking(CapacityPlanError),
    PhaseBacking(CapacityPlanError),
    BackingPlanIsNotNew {
        index: usize,
        original_len: usize,
        original_capacity: usize,
    },
    BackingFootprint {
        index: usize,
        cause: CapacityPlanError,
    },
    NestedByteCountOverflow,
    NestedAllocationCountOverflow,
}

impl fmt::Display for TerminalEvidenceBackingPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SemanticLayout(cause) => write!(formatter, "semantic layout failed: {cause}"),
            Self::DimensionBacking(cause) => {
                write!(formatter, "dimension backing plan failed: {cause}")
            }
            Self::PhaseBacking(cause) => write!(formatter, "phase backing plan failed: {cause}"),
            Self::BackingPlanIsNotNew {
                index,
                original_len,
                original_capacity,
            } => write!(
                formatter,
                "terminal backing plan {index} starts at len/capacity {original_len}/{original_capacity}"
            ),
            Self::BackingFootprint { index, cause } => {
                write!(
                    formatter,
                    "terminal backing plan {index} is invalid: {cause}"
                )
            }
            Self::NestedByteCountOverflow => {
                formatter.write_str("terminal backing byte count overflowed")
            }
            Self::NestedAllocationCountOverflow => {
                formatter.write_str("terminal backing allocation count overflowed")
            }
        }
    }
}

impl std::error::Error for TerminalEvidenceBackingPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SemanticLayout(cause) => Some(cause),
            Self::DimensionBacking(cause) | Self::PhaseBacking(cause) => Some(cause),
            Self::BackingFootprint { cause, .. } => Some(cause),
            Self::BackingPlanIsNotNew { .. }
            | Self::NestedByteCountOverflow
            | Self::NestedAllocationCountOverflow => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalEvidenceBuildError {
    ShapeMismatch {
        planned_dimensions: usize,
        actual_dimensions: usize,
        planned_phases: usize,
        actual_phases: usize,
    },
    Catalog(TerminalEvidenceCatalogError),
    DimensionCapacity(CapacityPlanError),
    PhaseCapacity(CapacityPlanError),
    DimensionAllocationFailed,
    PhaseAllocationFailed,
}

impl fmt::Display for TerminalEvidenceBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeMismatch {
                planned_dimensions,
                actual_dimensions,
                planned_phases,
                actual_phases,
            } => write!(
                formatter,
                "terminal backing plan expected {planned_dimensions}/{planned_phases} dimension/phase keys, observed {actual_dimensions}/{actual_phases}"
            ),
            Self::Catalog(cause) => cause.fmt(formatter),
            Self::DimensionCapacity(cause) => {
                write!(formatter, "dimension backing capacity failed: {cause}")
            }
            Self::PhaseCapacity(cause) => {
                write!(formatter, "phase backing capacity failed: {cause}")
            }
            Self::DimensionAllocationFailed => {
                formatter.write_str("dimension backing allocation failed")
            }
            Self::PhaseAllocationFailed => formatter.write_str("phase backing allocation failed"),
        }
    }
}

impl std::error::Error for TerminalEvidenceBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(cause) => Some(cause),
            Self::DimensionCapacity(cause) | Self::PhaseCapacity(cause) => Some(cause),
            Self::ShapeMismatch { .. }
            | Self::DimensionAllocationFailed
            | Self::PhaseAllocationFailed => None,
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) struct PublicationDimensionCell<D> {
    dimension: D,
    attempted: AtomicU64,
    consumed: AtomicU64,
    peak_live_bytes: AtomicU64,
    committed_canonical_output: AtomicU64,
    published_outcome_evidence: AtomicU64,
}

impl<D> PublicationDimensionCell<D> {
    pub(super) const fn new(dimension: D) -> Self {
        Self {
            dimension,
            attempted: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            peak_live_bytes: AtomicU64::new(0),
            committed_canonical_output: AtomicU64::new(0),
            published_outcome_evidence: AtomicU64::new(0),
        }
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(super) struct PublicationEvidenceCells<D> {
    dimensions: Vec<PublicationDimensionCell<D>>,
    frozen: AtomicBool,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy + Ord> PublicationEvidenceCells<D> {
    pub(super) const fn new(dimensions: Vec<PublicationDimensionCell<D>>) -> Self {
        Self {
            dimensions,
            frozen: AtomicBool::new(false),
        }
    }

    pub(super) fn freeze_from(&self, snapshot: &LedgerSnapshot<impl Copy + Ord, D>) {
        for (cell, (_, evidence)) in self.dimensions.iter().zip(snapshot.dimensions()) {
            cell.attempted
                .store(evidence.attempted(), Ordering::Release);
            cell.consumed.store(evidence.consumed(), Ordering::Release);
            cell.peak_live_bytes
                .store(evidence.peak_live_bytes(), Ordering::Release);
            cell.committed_canonical_output
                .store(evidence.committed_canonical_output(), Ordering::Release);
            cell.published_outcome_evidence
                .store(evidence.published_outcome_evidence(), Ordering::Release);
        }
        self.frozen.store(true, Ordering::Release);
    }

    pub(super) fn dimension(&self, dimension: D) -> Option<PublishedDimensionSnapshot<D>> {
        if !self.frozen.load(Ordering::Acquire) {
            return None;
        }
        let position = self
            .dimensions
            .binary_search_by_key(&dimension, |cell| cell.dimension)
            .ok()?;
        let cell = &self.dimensions[position];
        Some(PublishedDimensionSnapshot {
            dimension,
            attempted: cell.attempted.load(Ordering::Acquire),
            consumed: cell.consumed.load(Ordering::Acquire),
            peak_live_bytes: cell.peak_live_bytes.load(Ordering::Acquire),
            committed_canonical_output: cell.committed_canonical_output.load(Ordering::Acquire),
            published_outcome_evidence: cell.published_outcome_evidence.load(Ordering::Acquire),
        })
    }
}

/// One copied destination-aware dimension snapshot from a published operation.
#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PublishedDimensionSnapshot<D> {
    dimension: D,
    attempted: u64,
    consumed: u64,
    peak_live_bytes: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
impl<D: Copy> PublishedDimensionSnapshot<D> {
    pub const fn dimension(self) -> D {
        self.dimension
    }

    pub const fn attempted(self) -> u64 {
        self.attempted
    }

    pub const fn consumed(self) -> u64 {
        self.consumed
    }

    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    pub const fn committed_canonical_output(self) -> u64 {
        self.committed_canonical_output
    }

    pub const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

#[cfg(test)]
mod tests;
