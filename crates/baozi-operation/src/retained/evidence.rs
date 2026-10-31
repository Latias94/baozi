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
    static FAIL_NEXT_TERMINAL_EVIDENCE_QUALIFICATION: Cell<bool> = const { Cell::new(false) };
    static PANIC_NEXT_TERMINAL_EVIDENCE_QUALIFICATION: Cell<bool> = const { Cell::new(false) };
    static PANIC_NEXT_TERMINAL_EVIDENCE_WRITE: Cell<bool> = const { Cell::new(false) };
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

#[cfg(test)]
pub(super) fn fail_next_terminal_evidence_qualification() {
    FAIL_NEXT_TERMINAL_EVIDENCE_QUALIFICATION.with(|fail| fail.set(true));
}

#[cfg(test)]
pub(super) fn panic_next_terminal_evidence_qualification() {
    PANIC_NEXT_TERMINAL_EVIDENCE_QUALIFICATION.with(|panic| panic.set(true));
}

#[cfg(test)]
pub(super) fn panic_next_terminal_evidence_write() {
    PANIC_NEXT_TERMINAL_EVIDENCE_WRITE.with(|panic| panic.set(true));
}

#[cfg(test)]
fn take_terminal_evidence_qualification_failure() -> bool {
    FAIL_NEXT_TERMINAL_EVIDENCE_QUALIFICATION.with(|fail| fail.replace(false))
}

#[cfg(test)]
fn take_terminal_evidence_qualification_panic() -> bool {
    PANIC_NEXT_TERMINAL_EVIDENCE_QUALIFICATION.with(|panic| panic.replace(false))
}

#[cfg(test)]
fn take_terminal_evidence_write_panic() -> bool {
    PANIC_NEXT_TERMINAL_EVIDENCE_WRITE.with(|panic| panic.replace(false))
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

    pub(crate) const fn product_graph(code: u32) -> Self {
        Self {
            class: PreparationFailureClass::ProductGraph,
            code,
        }
    }

    pub(crate) const fn ledger() -> Self {
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
pub(crate) enum TerminalEvidenceCause {
    Success,
    Failure(PreparationFailureCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FallbackHeaderSnapshot<P: TerminalEvidenceKey> {
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
    pub(crate) const fn schema_revision(self) -> u32 {
        self.schema_revision
    }

    pub(crate) const fn terminal_cause(self) -> TerminalEvidenceCause {
        self.terminal_cause
    }

    pub(crate) const fn last_completed_phase(self) -> Option<P> {
        self.last_completed_phase
    }

    pub(crate) const fn dimension_count(self) -> usize {
        self.dimension_count
    }

    pub(crate) const fn phase_count(self) -> usize {
        self.phase_count
    }

    pub(crate) const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    pub(crate) const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    pub(crate) const fn active_reservations(self) -> u64 {
        self.active_reservations
    }

    pub(crate) const fn current_evidence_bytes(self) -> u64 {
        self.current_evidence_bytes
    }

    pub(crate) const fn peak_evidence_bytes(self) -> u64 {
        self.peak_evidence_bytes
    }
}

const TERMINAL_CAUSE_UNSET: u8 = 0;
const TERMINAL_CAUSE_SUCCESS: u8 = 1;
const TERMINAL_CAUSE_FAILURE: u8 = 2;

pub(super) struct FallbackHeader<P> {
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

pub(super) struct FallbackDimensionCell<D> {
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

pub(super) struct FallbackPhaseCell<P> {
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
    #[cfg(test)]
    InjectedFailure,
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
            #[cfg(test)]
            Self::InjectedFailure => {
                formatter.write_str("injected terminal ledger evidence freeze failure")
            }
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

pub(super) struct TerminalLedgerEvidence<P, D> {
    header: FallbackHeader<P>,
    dimensions: Vec<FallbackDimensionCell<D>>,
    phases: Vec<FallbackPhaseCell<P>>,
    freeze_started: AtomicBool,
    final_frozen: AtomicBool,
}

pub(super) struct TerminalEvidenceFreezeQualification {
    last_completed_phase_ordinal_plus_one: usize,
}

impl<P, D> TerminalLedgerEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub(super) fn freeze_from(
        &self,
        qualification: &TerminalEvidenceFreezeQualification,
        terminal_cause: TerminalEvidenceCause,
        snapshot: &LedgerSnapshot<P, D>,
    ) {
        #[cfg(test)]
        if take_terminal_evidence_write_panic() {
            panic!("injected terminal ledger evidence write panic");
        }
        for (cell, (_, value)) in self.dimensions.iter().zip(snapshot.dimensions()) {
            cell.write(value);
        }
        for (cell, (_, value)) in self.phases.iter().zip(snapshot.phases()) {
            cell.write(value);
        }
        self.header.write(
            terminal_cause,
            qualification.last_completed_phase_ordinal_plus_one,
            snapshot,
        );
        self.final_frozen.store(true, Ordering::Release);
    }
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

    pub(super) fn try_qualify(
        &self,
        expected_dimensions: usize,
        mut dimension_key_at: impl FnMut(usize) -> Option<D>,
        expected_phases: usize,
        mut phase_key_at: impl FnMut(usize) -> Option<P>,
        last_completed_phase: Option<P>,
    ) -> Result<TerminalEvidenceFreezeQualification, TerminalEvidenceFreezeError> {
        let last_completed_phase_ordinal_plus_one = self.validate_qualification(
            expected_dimensions,
            &mut dimension_key_at,
            expected_phases,
            &mut phase_key_at,
            last_completed_phase,
        )?;
        self.finish_qualification(last_completed_phase_ordinal_plus_one)
    }

    fn validate_qualification(
        &self,
        expected_dimensions: usize,
        dimension_key_at: &mut impl FnMut(usize) -> Option<D>,
        expected_phases: usize,
        phase_key_at: &mut impl FnMut(usize) -> Option<P>,
        last_completed_phase: Option<P>,
    ) -> Result<usize, TerminalEvidenceFreezeError> {
        if self.freeze_started.load(Ordering::Acquire) || self.final_frozen.load(Ordering::Acquire)
        {
            return Err(TerminalEvidenceFreezeError::AlreadyFrozen);
        }

        #[cfg(test)]
        if take_terminal_evidence_qualification_failure() {
            return Err(TerminalEvidenceFreezeError::InjectedFailure);
        }

        #[cfg(test)]
        if take_terminal_evidence_qualification_panic() {
            panic!("injected terminal ledger evidence qualification panic");
        }

        if self.dimensions.len() != expected_dimensions || self.phases.len() != expected_phases {
            return Err(TerminalEvidenceFreezeError::ShapeMismatch {
                expected_dimensions: self.dimensions.len(),
                actual_dimensions: expected_dimensions,
                expected_phases: self.phases.len(),
                actual_phases: expected_phases,
            });
        }

        for (index, cell) in self.dimensions.iter().enumerate() {
            if dimension_key_at(index) != Some(cell.key) {
                return Err(TerminalEvidenceFreezeError::DimensionKeyMismatch { index });
            }
        }
        if dimension_key_at(expected_dimensions).is_some() {
            return Err(TerminalEvidenceFreezeError::ShapeMismatch {
                expected_dimensions: self.dimensions.len(),
                actual_dimensions: expected_dimensions.saturating_add(1),
                expected_phases: self.phases.len(),
                actual_phases: expected_phases,
            });
        }
        for (index, cell) in self.phases.iter().enumerate() {
            if phase_key_at(index) != Some(cell.key) {
                return Err(TerminalEvidenceFreezeError::PhaseKeyMismatch { index });
            }
        }
        if phase_key_at(expected_phases).is_some() {
            return Err(TerminalEvidenceFreezeError::ShapeMismatch {
                expected_dimensions: self.dimensions.len(),
                actual_dimensions: expected_dimensions,
                expected_phases: self.phases.len(),
                actual_phases: expected_phases.saturating_add(1),
            });
        }
        let ordinal = match last_completed_phase {
            Some(phase) => self
                .phases
                .binary_search_by_key(&phase, FallbackPhaseCell::key)
                .map_err(|_| TerminalEvidenceFreezeError::LastCompletedPhaseUnknown)?
                .checked_add(1)
                .ok_or(TerminalEvidenceFreezeError::LastCompletedPhaseOrdinalOverflow)?,
            None => 0,
        };
        Ok(ordinal)
    }

    fn finish_qualification(
        &self,
        last_completed_phase_ordinal_plus_one: usize,
    ) -> Result<TerminalEvidenceFreezeQualification, TerminalEvidenceFreezeError> {
        if self
            .freeze_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TerminalEvidenceFreezeError::AlreadyFrozen);
        }
        Ok(TerminalEvidenceFreezeQualification {
            last_completed_phase_ordinal_plus_one,
        })
    }

    #[cfg(test)]
    pub(super) fn try_freeze_from(
        &self,
        terminal_cause: TerminalEvidenceCause,
        snapshot: &LedgerSnapshot<P, D>,
    ) -> Result<(), TerminalEvidenceFreezeError> {
        let last_completed_phase_ordinal_plus_one = self.validate_qualification(
            snapshot.dimensions().len(),
            &mut |index| snapshot.dimensions().nth(index).map(|(key, _)| key),
            snapshot.phases().len(),
            &mut |index| snapshot.phases().nth(index).map(|(key, _)| key),
            snapshot.last_completed_phase(),
        )?;
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
        let qualification = self.finish_qualification(last_completed_phase_ordinal_plus_one)?;
        self.freeze_from(&qualification, terminal_cause, snapshot);
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

    pub(super) fn dimension_key_at(&self, index: usize) -> Option<D> {
        self.is_frozen()
            .then(|| self.dimensions.get(index).map(FallbackDimensionCell::key))
            .flatten()
    }

    pub(super) fn phase_key_at(&self, index: usize) -> Option<P> {
        self.is_frozen()
            .then(|| self.phases.get(index).map(FallbackPhaseCell::key))
            .flatten()
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
        self.try_build_indexed(
            |index| dimension_keys.get(index).copied(),
            |index| phase_keys.get(index).copied(),
        )
    }

    pub(super) fn try_build_indexed(
        self,
        mut dimension_key_at: impl FnMut(usize) -> Option<D>,
        mut phase_key_at: impl FnMut(usize) -> Option<P>,
    ) -> Result<TerminalLedgerEvidence<P, D>, TerminalEvidenceBuildError> {
        // U4 startup will expose this allocation tail only through its ready two-plane guard.
        // Keeping it here binds both real Vec backings to the qualified plans without importing
        // ledger/account lifecycle authority into this leaf model.
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

        for index in 0..self.dimension_capacity.desired_capacity() {
            let key = dimension_key_at(index).ok_or(TerminalEvidenceBuildError::ShapeMismatch {
                planned_dimensions: self.dimension_capacity.desired_capacity(),
                actual_dimensions: index,
                planned_phases: self.phase_capacity.desired_capacity(),
                actual_phases: self.phase_capacity.desired_capacity(),
            })?;
            if dimensions
                .last()
                .is_some_and(|previous: &FallbackDimensionCell<D>| previous.key() >= key)
            {
                return Err(TerminalEvidenceBuildError::Catalog(
                    TerminalEvidenceCatalogError::DimensionKeysNotStrictlyOrdered { index },
                ));
            }
            dimensions.push(FallbackDimensionCell::new(key));
        }
        if dimension_key_at(self.dimension_capacity.desired_capacity()).is_some() {
            return Err(TerminalEvidenceBuildError::ShapeMismatch {
                planned_dimensions: self.dimension_capacity.desired_capacity(),
                actual_dimensions: self.dimension_capacity.desired_capacity().saturating_add(1),
                planned_phases: self.phase_capacity.desired_capacity(),
                actual_phases: self.phase_capacity.desired_capacity(),
            });
        }
        for index in 0..self.phase_capacity.desired_capacity() {
            let key = phase_key_at(index).ok_or(TerminalEvidenceBuildError::ShapeMismatch {
                planned_dimensions: self.dimension_capacity.desired_capacity(),
                actual_dimensions: self.dimension_capacity.desired_capacity(),
                planned_phases: self.phase_capacity.desired_capacity(),
                actual_phases: index,
            })?;
            if phases
                .last()
                .is_some_and(|previous: &FallbackPhaseCell<P>| previous.key() >= key)
            {
                return Err(TerminalEvidenceBuildError::Catalog(
                    TerminalEvidenceCatalogError::PhaseKeysNotStrictlyOrdered { index },
                ));
            }
            phases.push(FallbackPhaseCell::new(key));
        }
        if phase_key_at(self.phase_capacity.desired_capacity()).is_some() {
            return Err(TerminalEvidenceBuildError::ShapeMismatch {
                planned_dimensions: self.dimension_capacity.desired_capacity(),
                actual_dimensions: self.dimension_capacity.desired_capacity(),
                planned_phases: self.phase_capacity.desired_capacity(),
                actual_phases: self.phase_capacity.desired_capacity().saturating_add(1),
            });
        }
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

#[cfg(test)]
mod tests;
