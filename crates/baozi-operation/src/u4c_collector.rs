//! Temporary sealed collector for the U4c reproducibility gate.
//!
//! The collector deliberately owns no measured storage and accepts only local
//! event facts from the retained-store implementation.

use std::cell::Cell;
use std::fmt;
use std::hint::spin_loop;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const NO_SESSION: u64 = 0;

const FAULT_COUNTER_OVERFLOW: u8 = 1 << 0;
const FAULT_LOOKUP_ORDER: u8 = 1 << 1;
const FAULT_LOOKUP_TERMINATION: u8 = 1 << 2;
const FAULT_BLOCK_LIFECYCLE: u8 = 1 << 3;
const FAULT_MEMBER_PROBE: u8 = 1 << 4;

static COLLECTOR_GATE: AtomicBool = AtomicBool::new(false);
static ACTIVE_SESSION_ID: AtomicU64 = AtomicU64::new(NO_SESSION);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
static CROSS_THREAD_VIOLATION_ID: AtomicU64 = AtomicU64::new(NO_SESSION);

thread_local! {
    static COLLECTOR_STATE: Cell<CollectorState> =
        const { Cell::new(CollectorState::inactive()) };
}

/// A complete, validated U4c measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeasurementReport {
    /// Counted `write_semantic_cell` producer events.
    semantic_cell_writes: u64,
    /// Counted `write_reference` producer events.
    reference_writes: u64,
    /// Total events in the five-kind producer taxonomy.
    producer_events: u64,
    /// Bytes copied while producing or sharing retained payloads.
    payload_copy_bytes: u64,
    /// Successful new retained-payload backing allocations.
    payload_backing_allocations: u64,
    /// Counted `begin_block` events across new and inherited memberships.
    block_begins: u64,
    /// Counted `seal_block` events across new and inherited memberships.
    block_seals: u64,
    /// Counted `publish_slot` events across new and inherited memberships.
    slot_publications: u64,
    /// New-membership `begin_block` events.
    new_block_begins: u64,
    /// New-membership `seal_block` events.
    new_block_seals: u64,
    /// New-membership `publish_slot` events.
    new_slot_publications: u64,
    /// Inherited-membership `begin_block` events.
    inherited_block_begins: u64,
    /// Inherited-membership `seal_block` events.
    inherited_block_seals: u64,
    /// Inherited-membership `publish_slot` events.
    inherited_slot_publications: u64,
    /// Elements decoded individually by the measured implementation.
    element_decodes: u64,
    /// Successful and rejected lookups that reached an explicit completion.
    completed_lookups: u64,
    /// Lookups that completed through an explicit rejection.
    rejected_lookups: u64,
    /// Events emitted from the fixed lookup taxonomy.
    lookup_events: u64,
    /// Largest taxonomy-event count observed for one completed lookup.
    max_lookup_events: u64,
    /// Actual retained-member index probes performed by checked lookups.
    member_probes: u64,
    /// Largest retained-member probe count observed for one completed lookup.
    max_member_probes_per_lookup: u64,
}

impl MeasurementReport {
    pub const fn semantic_cell_writes(self) -> u64 {
        self.semantic_cell_writes
    }

    pub const fn reference_writes(self) -> u64 {
        self.reference_writes
    }

    pub const fn producer_events(self) -> u64 {
        self.producer_events
    }

    pub const fn payload_copy_bytes(self) -> u64 {
        self.payload_copy_bytes
    }

    pub const fn payload_backing_allocations(self) -> u64 {
        self.payload_backing_allocations
    }

    pub const fn block_begins(self) -> u64 {
        self.block_begins
    }

    pub const fn block_seals(self) -> u64 {
        self.block_seals
    }

    pub const fn slot_publications(self) -> u64 {
        self.slot_publications
    }

    pub const fn new_block_begins(self) -> u64 {
        self.new_block_begins
    }

    pub const fn new_block_seals(self) -> u64 {
        self.new_block_seals
    }

    pub const fn new_slot_publications(self) -> u64 {
        self.new_slot_publications
    }

    pub const fn inherited_block_begins(self) -> u64 {
        self.inherited_block_begins
    }

    pub const fn inherited_block_seals(self) -> u64 {
        self.inherited_block_seals
    }

    pub const fn inherited_slot_publications(self) -> u64 {
        self.inherited_slot_publications
    }

    pub const fn element_decodes(self) -> u64 {
        self.element_decodes
    }

    pub const fn completed_lookups(self) -> u64 {
        self.completed_lookups
    }

    pub const fn rejected_lookups(self) -> u64 {
        self.rejected_lookups
    }

    pub const fn lookup_events(self) -> u64 {
        self.lookup_events
    }

    pub const fn max_lookup_events(self) -> u64 {
        self.max_lookup_events
    }

    pub const fn member_probes(self) -> u64 {
        self.member_probes
    }

    pub const fn max_member_probes_per_lookup(self) -> u64 {
        self.max_member_probes_per_lookup
    }
}

/// A failure to start or validate a U4c measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MeasurementError {
    /// Another process-wide measurement session is active.
    SessionAlreadyActive,
    /// The non-reused process-wide session identifier space is exhausted.
    SessionIdExhausted,
    /// This thread's collector state cannot be accessed.
    ThreadLocalStateUnavailable,
    /// This thread contains collector state owned by a different session.
    ThreadLocalStateConflict,
    /// The session no longer owns the process-wide collector slot.
    SessionOwnershipLost,
    /// At least one measured counter overflowed.
    CounterOverflow,
    /// A hook ran from a thread other than the session owner.
    CrossThreadViolation,
    /// Lookup taxonomy events were emitted out of order.
    LookupOrderViolation,
    /// A rejected lookup was recorded without an active lookup sequence.
    LookupTerminationWithoutActiveLookup,
    /// A lookup sequence was not explicitly completed.
    IncompleteLookup,
    /// A lookup reached the retained member range without exactly one indexed probe.
    MemberProbeViolation,
    /// A block was sealed or published outside its required lifecycle order.
    BlockLifecycleViolation,
    /// A begun block was not both sealed and published.
    IncompleteBlockLifecycle,
    /// The session observed no retained-store producer, payload, decode, or lookup event.
    NoObservedStoreWork,
}

impl fmt::Display for MeasurementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::SessionAlreadyActive => "another measurement session is already active",
            Self::SessionIdExhausted => "measurement session identifiers are exhausted",
            Self::ThreadLocalStateUnavailable => {
                "the measurement thread-local state is unavailable"
            }
            Self::ThreadLocalStateConflict => {
                "the measurement thread-local state belongs to another session"
            }
            Self::SessionOwnershipLost => "the measurement session no longer owns the collector",
            Self::CounterOverflow => "a measurement counter overflowed",
            Self::CrossThreadViolation => "a measurement hook ran outside the session owner thread",
            Self::LookupOrderViolation => "lookup taxonomy events were emitted out of order",
            Self::LookupTerminationWithoutActiveLookup => {
                "a rejected lookup was recorded without an active lookup"
            }
            Self::IncompleteLookup => "a lookup was left incomplete",
            Self::MemberProbeViolation => {
                "a checked lookup did not perform exactly one retained member probe"
            }
            Self::BlockLifecycleViolation => "a block lifecycle event was emitted out of order",
            Self::IncompleteBlockLifecycle => "a block lifecycle was left incomplete",
            Self::NoObservedStoreWork => "the measurement session observed no retained-store work",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for MeasurementError {}

/// Exclusive ownership of one process-wide measurement interval.
///
/// The `Rc` marker intentionally makes a session neither `Send` nor `Sync`.
#[must_use = "dropping the session abandons the measurement without producing a report"]
pub struct MeasurementSession {
    id: u64,
    armed: bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl MeasurementSession {
    /// Starts a process-wide measurement on the current thread.
    pub fn begin() -> Result<Self, MeasurementError> {
        let _gate = CollectorGateGuard::acquire();

        if ACTIVE_SESSION_ID.load(Ordering::Relaxed) != NO_SESSION {
            return Err(MeasurementError::SessionAlreadyActive);
        }

        let local_is_empty = COLLECTOR_STATE
            .try_with(|state| state.get().session_id == NO_SESSION)
            .map_err(|_| MeasurementError::ThreadLocalStateUnavailable)?;
        if !local_is_empty {
            return Err(MeasurementError::ThreadLocalStateConflict);
        }

        let id = take_next_session_id()?;
        COLLECTOR_STATE
            .try_with(|state| state.set(CollectorState::for_session(id)))
            .map_err(|_| MeasurementError::ThreadLocalStateUnavailable)?;
        ACTIVE_SESSION_ID.store(id, Ordering::Relaxed);

        Ok(Self {
            id,
            armed: true,
            not_send_or_sync: PhantomData,
        })
    }

    /// Ends the session and returns a report only when every invariant holds.
    pub fn finish(mut self) -> Result<MeasurementReport, MeasurementError> {
        let result = finish_session(self.id);
        self.armed = false;
        result
    }
}

impl Drop for MeasurementSession {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        abandon_session(self.id);
        self.armed = false;
    }
}

/// The retained membership represented by one block lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockKind {
    New,
    Inherited,
}

/// One event in the fixed checked-lookup taxonomy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LookupStage {
    CheckTableBrand,
    CheckSlotSet,
    CheckRange,
    LoadMember,
    CheckVariant,
    CheckSchemaTag,
    ProjectView,
    ReturnView,
}

impl LookupStage {
    const fn ordinal(self) -> u8 {
        match self {
            Self::CheckTableBrand => 0,
            Self::CheckSlotSet => 1,
            Self::CheckRange => 2,
            Self::LoadMember => 3,
            Self::CheckVariant => 4,
            Self::CheckSchemaTag => 5,
            Self::ProjectView => 6,
            Self::ReturnView => 7,
        }
    }
}

/// Records one semantic scalar cell written to retained output.
pub(crate) fn record_semantic_cell_write() {
    record_semantic_cell_writes(1);
}

/// Records semantic scalar cells derived from one concrete store write.
pub(crate) fn record_semantic_cell_writes(count: usize) {
    record_event(Event::SemanticCellWrites(count));
}

/// Records one target-table-qualified retained reference occurrence.
pub(crate) fn record_reference_write() {
    record_event(Event::ReferenceWrite);
}

/// Records bytes actually copied for a retained payload operation.
pub(crate) fn record_payload_copy_bytes(bytes: usize) {
    record_event(Event::PayloadCopyBytes(bytes));
}

/// Records one successful new retained-payload backing allocation.
pub(crate) fn record_payload_backing_allocation() {
    record_event(Event::PayloadBackingAllocation);
}

/// Records one element decoded individually by a retained view.
#[allow(
    dead_code,
    reason = "the zero-decode implementation retains a private event hook so source gates can reject future decoder bypasses"
)]
pub(crate) fn record_element_decode() {
    record_event(Event::ElementDecode);
}

/// Records the start of one new or inherited retained block membership.
pub(crate) fn record_block_begin(kind: BlockKind) {
    record_event(Event::BlockBegin(kind));
}

/// Records the seal of one new or inherited retained block membership.
pub(crate) fn record_block_seal(kind: BlockKind) {
    record_event(Event::BlockSeal(kind));
}

/// Records publication of one new or inherited target-table slot.
pub(crate) fn record_slot_publication(kind: BlockKind) {
    record_event(Event::SlotPublication(kind));
}

/// Records one checked-lookup taxonomy stage.
pub(crate) fn record_lookup_stage(stage: LookupStage) {
    record_event(Event::LookupStage(stage));
}

/// Records one actual indexed access to the retained member directory.
pub(crate) fn record_lookup_member_probe() {
    record_event(Event::LookupMemberProbe);
}

/// Completes a rejected lookup without inventing later taxonomy events.
pub(crate) fn record_lookup_rejected() {
    record_event(Event::LookupRejected);
}

#[derive(Clone, Copy)]
enum Event {
    SemanticCellWrites(usize),
    ReferenceWrite,
    PayloadCopyBytes(usize),
    PayloadBackingAllocation,
    #[allow(
        dead_code,
        reason = "the zero-decode implementation retains a private event variant for future source-gated decoders"
    )]
    ElementDecode,
    BlockBegin(BlockKind),
    BlockSeal(BlockKind),
    SlotPublication(BlockKind),
    LookupStage(LookupStage),
    LookupMemberProbe,
    LookupRejected,
}

#[derive(Clone, Copy)]
struct CollectorState {
    session_id: u64,
    faults: u8,
    semantic_cell_writes: u64,
    reference_writes: u64,
    producer_events: u64,
    payload_copy_bytes: u64,
    payload_backing_allocations: u64,
    block_begins: u64,
    block_seals: u64,
    slot_publications: u64,
    new_block_begins: u64,
    new_block_seals: u64,
    new_slot_publications: u64,
    inherited_block_begins: u64,
    inherited_block_seals: u64,
    inherited_slot_publications: u64,
    new_unsealed_blocks: u64,
    new_sealed_blocks: u64,
    inherited_unsealed_blocks: u64,
    inherited_sealed_blocks: u64,
    element_decodes: u64,
    completed_lookups: u64,
    rejected_lookups: u64,
    lookup_events: u64,
    max_lookup_events: u64,
    current_lookup_events: u64,
    member_probes: u64,
    max_member_probes_per_lookup: u64,
    current_member_probes: u64,
    next_lookup_stage: u8,
    lookup_active: bool,
}

impl CollectorState {
    const fn inactive() -> Self {
        Self::for_session(NO_SESSION)
    }

    const fn for_session(session_id: u64) -> Self {
        Self {
            session_id,
            faults: 0,
            semantic_cell_writes: 0,
            reference_writes: 0,
            producer_events: 0,
            payload_copy_bytes: 0,
            payload_backing_allocations: 0,
            block_begins: 0,
            block_seals: 0,
            slot_publications: 0,
            new_block_begins: 0,
            new_block_seals: 0,
            new_slot_publications: 0,
            inherited_block_begins: 0,
            inherited_block_seals: 0,
            inherited_slot_publications: 0,
            new_unsealed_blocks: 0,
            new_sealed_blocks: 0,
            inherited_unsealed_blocks: 0,
            inherited_sealed_blocks: 0,
            element_decodes: 0,
            completed_lookups: 0,
            rejected_lookups: 0,
            lookup_events: 0,
            max_lookup_events: 0,
            current_lookup_events: 0,
            member_probes: 0,
            max_member_probes_per_lookup: 0,
            current_member_probes: 0,
            next_lookup_stage: 0,
            lookup_active: false,
        }
    }

    fn record(&mut self, event: Event) {
        match event {
            Event::SemanticCellWrites(count) => match u64::try_from(count) {
                Ok(count) => {
                    self.increment_counter(Counter::ProducerEvents, count);
                    self.increment_counter(Counter::SemanticCellWrites, count);
                }
                Err(_) => self.mark_fault(FAULT_COUNTER_OVERFLOW),
            },
            Event::ReferenceWrite => {
                self.increment_producer_event();
                self.increment_counter(Counter::ReferenceWrites, 1);
            }
            Event::PayloadCopyBytes(bytes) => match u64::try_from(bytes) {
                Ok(bytes) => self.increment_counter(Counter::PayloadCopyBytes, bytes),
                Err(_) => self.mark_fault(FAULT_COUNTER_OVERFLOW),
            },
            Event::PayloadBackingAllocation => {
                self.increment_counter(Counter::PayloadBackingAllocations, 1);
            }
            Event::ElementDecode => self.increment_counter(Counter::ElementDecodes, 1),
            Event::BlockBegin(kind) => self.record_block_begin(kind),
            Event::BlockSeal(kind) => self.record_block_seal(kind),
            Event::SlotPublication(kind) => self.record_slot_publication(kind),
            Event::LookupStage(stage) => self.record_lookup_stage(stage),
            Event::LookupMemberProbe => self.record_lookup_member_probe(),
            Event::LookupRejected => self.record_lookup_rejected(),
        }
    }

    fn increment_producer_event(&mut self) {
        self.increment_counter(Counter::ProducerEvents, 1);
    }

    fn record_block_begin(&mut self, kind: BlockKind) {
        self.increment_producer_event();
        self.increment_counter(Counter::BlockBegins, 1);

        match kind {
            BlockKind::New => {
                self.increment_counter(Counter::NewBlockBegins, 1);
                self.increment_counter(Counter::NewUnsealedBlocks, 1);
            }
            BlockKind::Inherited => {
                self.increment_counter(Counter::InheritedBlockBegins, 1);
                self.increment_counter(Counter::InheritedUnsealedBlocks, 1);
            }
        }
    }

    fn record_block_seal(&mut self, kind: BlockKind) {
        self.increment_producer_event();
        self.increment_counter(Counter::BlockSeals, 1);

        match kind {
            BlockKind::New => {
                self.increment_counter(Counter::NewBlockSeals, 1);
                if checked_decrement(&mut self.new_unsealed_blocks) {
                    self.increment_counter(Counter::NewSealedBlocks, 1);
                } else {
                    self.mark_fault(FAULT_BLOCK_LIFECYCLE);
                }
            }
            BlockKind::Inherited => {
                self.increment_counter(Counter::InheritedBlockSeals, 1);
                if checked_decrement(&mut self.inherited_unsealed_blocks) {
                    self.increment_counter(Counter::InheritedSealedBlocks, 1);
                } else {
                    self.mark_fault(FAULT_BLOCK_LIFECYCLE);
                }
            }
        }
    }

    fn record_slot_publication(&mut self, kind: BlockKind) {
        self.increment_producer_event();
        self.increment_counter(Counter::SlotPublications, 1);

        match kind {
            BlockKind::New => {
                self.increment_counter(Counter::NewSlotPublications, 1);
                if !checked_decrement(&mut self.new_sealed_blocks) {
                    self.mark_fault(FAULT_BLOCK_LIFECYCLE);
                }
            }
            BlockKind::Inherited => {
                self.increment_counter(Counter::InheritedSlotPublications, 1);
                if !checked_decrement(&mut self.inherited_sealed_blocks) {
                    self.mark_fault(FAULT_BLOCK_LIFECYCLE);
                }
            }
        }
    }

    fn record_lookup_stage(&mut self, stage: LookupStage) {
        self.increment_counter(Counter::LookupEvents, 1);
        self.increment_counter(Counter::CurrentLookupEvents, 1);

        if !self.lookup_active {
            if stage != LookupStage::CheckTableBrand {
                self.mark_fault(FAULT_LOOKUP_ORDER);
                return;
            }

            self.lookup_active = true;
            self.next_lookup_stage = LookupStage::CheckTableBrand.ordinal();
        }

        if stage.ordinal() != self.next_lookup_stage {
            self.mark_fault(FAULT_LOOKUP_ORDER);
            return;
        }

        if stage == LookupStage::ReturnView {
            self.complete_lookup(false);
        } else {
            self.next_lookup_stage += 1;
        }
    }

    fn record_lookup_member_probe(&mut self) {
        if !self.lookup_active || self.next_lookup_stage != LookupStage::LoadMember.ordinal() {
            self.mark_fault(FAULT_MEMBER_PROBE);
            return;
        }

        self.increment_counter(Counter::MemberProbes, 1);
        self.increment_counter(Counter::CurrentMemberProbes, 1);
    }

    fn record_lookup_rejected(&mut self) {
        if !self.lookup_active {
            self.mark_fault(FAULT_LOOKUP_TERMINATION);
            return;
        }

        self.complete_lookup(true);
    }

    fn complete_lookup(&mut self, rejected: bool) {
        let reached_member_directory = self.next_lookup_stage > LookupStage::CheckRange.ordinal();
        let expected_member_probes = u64::from(reached_member_directory);
        if self.current_member_probes != expected_member_probes {
            self.mark_fault(FAULT_MEMBER_PROBE);
        }

        self.max_lookup_events = self.max_lookup_events.max(self.current_lookup_events);
        self.max_member_probes_per_lookup = self
            .max_member_probes_per_lookup
            .max(self.current_member_probes);
        self.increment_counter(Counter::CompletedLookups, 1);
        if rejected {
            self.increment_counter(Counter::RejectedLookups, 1);
        }
        self.current_lookup_events = 0;
        self.current_member_probes = 0;
        self.next_lookup_stage = 0;
        self.lookup_active = false;
    }

    fn increment_counter(&mut self, counter: Counter, amount: u64) {
        let target = match counter {
            Counter::SemanticCellWrites => &mut self.semantic_cell_writes,
            Counter::ReferenceWrites => &mut self.reference_writes,
            Counter::ProducerEvents => &mut self.producer_events,
            Counter::PayloadCopyBytes => &mut self.payload_copy_bytes,
            Counter::PayloadBackingAllocations => &mut self.payload_backing_allocations,
            Counter::BlockBegins => &mut self.block_begins,
            Counter::BlockSeals => &mut self.block_seals,
            Counter::SlotPublications => &mut self.slot_publications,
            Counter::NewBlockBegins => &mut self.new_block_begins,
            Counter::NewBlockSeals => &mut self.new_block_seals,
            Counter::NewSlotPublications => &mut self.new_slot_publications,
            Counter::InheritedBlockBegins => &mut self.inherited_block_begins,
            Counter::InheritedBlockSeals => &mut self.inherited_block_seals,
            Counter::InheritedSlotPublications => &mut self.inherited_slot_publications,
            Counter::NewUnsealedBlocks => &mut self.new_unsealed_blocks,
            Counter::NewSealedBlocks => &mut self.new_sealed_blocks,
            Counter::InheritedUnsealedBlocks => &mut self.inherited_unsealed_blocks,
            Counter::InheritedSealedBlocks => &mut self.inherited_sealed_blocks,
            Counter::ElementDecodes => &mut self.element_decodes,
            Counter::CompletedLookups => &mut self.completed_lookups,
            Counter::RejectedLookups => &mut self.rejected_lookups,
            Counter::LookupEvents => &mut self.lookup_events,
            Counter::CurrentLookupEvents => &mut self.current_lookup_events,
            Counter::MemberProbes => &mut self.member_probes,
            Counter::CurrentMemberProbes => &mut self.current_member_probes,
        };

        if !checked_add(target, amount) {
            self.mark_fault(FAULT_COUNTER_OVERFLOW);
        }
    }

    fn mark_fault(&mut self, fault: u8) {
        self.faults |= fault;
    }

    fn into_report(self) -> Result<MeasurementReport, MeasurementError> {
        if self.faults & FAULT_COUNTER_OVERFLOW != 0 {
            return Err(MeasurementError::CounterOverflow);
        }
        if self.faults & FAULT_LOOKUP_ORDER != 0 {
            return Err(MeasurementError::LookupOrderViolation);
        }
        if self.faults & FAULT_LOOKUP_TERMINATION != 0 {
            return Err(MeasurementError::LookupTerminationWithoutActiveLookup);
        }
        if self.lookup_active {
            return Err(MeasurementError::IncompleteLookup);
        }
        if self.faults & FAULT_MEMBER_PROBE != 0 {
            return Err(MeasurementError::MemberProbeViolation);
        }
        if self.faults & FAULT_BLOCK_LIFECYCLE != 0 {
            return Err(MeasurementError::BlockLifecycleViolation);
        }
        if self.new_unsealed_blocks != 0
            || self.new_sealed_blocks != 0
            || self.inherited_unsealed_blocks != 0
            || self.inherited_sealed_blocks != 0
        {
            return Err(MeasurementError::IncompleteBlockLifecycle);
        }
        if self.producer_events == 0
            && self.payload_copy_bytes == 0
            && self.payload_backing_allocations == 0
            && self.element_decodes == 0
            && self.lookup_events == 0
        {
            return Err(MeasurementError::NoObservedStoreWork);
        }

        Ok(MeasurementReport {
            semantic_cell_writes: self.semantic_cell_writes,
            reference_writes: self.reference_writes,
            producer_events: self.producer_events,
            payload_copy_bytes: self.payload_copy_bytes,
            payload_backing_allocations: self.payload_backing_allocations,
            block_begins: self.block_begins,
            block_seals: self.block_seals,
            slot_publications: self.slot_publications,
            new_block_begins: self.new_block_begins,
            new_block_seals: self.new_block_seals,
            new_slot_publications: self.new_slot_publications,
            inherited_block_begins: self.inherited_block_begins,
            inherited_block_seals: self.inherited_block_seals,
            inherited_slot_publications: self.inherited_slot_publications,
            element_decodes: self.element_decodes,
            completed_lookups: self.completed_lookups,
            rejected_lookups: self.rejected_lookups,
            lookup_events: self.lookup_events,
            max_lookup_events: self.max_lookup_events,
            member_probes: self.member_probes,
            max_member_probes_per_lookup: self.max_member_probes_per_lookup,
        })
    }
}

#[derive(Clone, Copy)]
enum Counter {
    SemanticCellWrites,
    ReferenceWrites,
    ProducerEvents,
    PayloadCopyBytes,
    PayloadBackingAllocations,
    BlockBegins,
    BlockSeals,
    SlotPublications,
    NewBlockBegins,
    NewBlockSeals,
    NewSlotPublications,
    InheritedBlockBegins,
    InheritedBlockSeals,
    InheritedSlotPublications,
    NewUnsealedBlocks,
    NewSealedBlocks,
    InheritedUnsealedBlocks,
    InheritedSealedBlocks,
    ElementDecodes,
    CompletedLookups,
    RejectedLookups,
    LookupEvents,
    CurrentLookupEvents,
    MemberProbes,
    CurrentMemberProbes,
}

struct CollectorGateGuard;

impl CollectorGateGuard {
    fn acquire() -> Self {
        while COLLECTOR_GATE
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }

        Self
    }
}

impl Drop for CollectorGateGuard {
    fn drop(&mut self) {
        COLLECTOR_GATE.store(false, Ordering::Release);
    }
}

fn take_next_session_id() -> Result<u64, MeasurementError> {
    let id = NEXT_SESSION_ID.load(Ordering::Relaxed);
    let next = id
        .checked_add(1)
        .ok_or(MeasurementError::SessionIdExhausted)?;
    NEXT_SESSION_ID.store(next, Ordering::Relaxed);
    Ok(id)
}

fn finish_session(session_id: u64) -> Result<MeasurementReport, MeasurementError> {
    let _gate = CollectorGateGuard::acquire();

    if ACTIVE_SESSION_ID.load(Ordering::Relaxed) != session_id {
        return Err(MeasurementError::SessionOwnershipLost);
    }

    let state = COLLECTOR_STATE
        .try_with(|local| {
            let state = local.get();
            if state.session_id == session_id {
                local.set(CollectorState::inactive());
                Ok(state)
            } else {
                Err(MeasurementError::ThreadLocalStateConflict)
            }
        })
        .map_err(|_| MeasurementError::ThreadLocalStateUnavailable)
        .and_then(|state| state);

    let cross_thread_violation = CROSS_THREAD_VIOLATION_ID.load(Ordering::Relaxed) == session_id;
    ACTIVE_SESSION_ID.store(NO_SESSION, Ordering::Relaxed);

    let state = state?;
    if cross_thread_violation {
        return Err(MeasurementError::CrossThreadViolation);
    }

    state.into_report()
}

fn abandon_session(session_id: u64) {
    let _gate = CollectorGateGuard::acquire();

    if ACTIVE_SESSION_ID.load(Ordering::Relaxed) != session_id {
        return;
    }

    let _ = COLLECTOR_STATE.try_with(|local| {
        if local.get().session_id == session_id {
            local.set(CollectorState::inactive());
        }
    });
    ACTIVE_SESSION_ID.store(NO_SESSION, Ordering::Relaxed);
}

fn record_event(event: Event) {
    let _gate = CollectorGateGuard::acquire();
    let session_id = ACTIVE_SESSION_ID.load(Ordering::Relaxed);
    if session_id == NO_SESSION {
        return;
    }

    let recorded = COLLECTOR_STATE
        .try_with(|local| {
            let mut state = local.get();
            if state.session_id != session_id {
                return false;
            }

            state.record(event);
            local.set(state);
            true
        })
        .unwrap_or(false);

    if !recorded {
        CROSS_THREAD_VIOLATION_ID.store(session_id, Ordering::Relaxed);
    }
}

fn checked_add(target: &mut u64, amount: u64) -> bool {
    let Some(value) = target.checked_add(amount) else {
        return false;
    };
    *target = value;
    true
}

fn checked_decrement(target: &mut u64) -> bool {
    let Some(value) = target.checked_sub(1) else {
        return false;
    };
    *target = value;
    true
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "the isolated U4c integration target includes this module and invokes its private state-machine probes"
)]
pub(crate) fn run_state_machine_tests() {
    tests::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn run() {
        success_report_includes_normative_totals();
        nested_and_concurrent_sessions_fail_closed();
        drop_and_unwind_allow_recovery();
        overflow_invalidates_the_report();
        cross_thread_hooks_invalidate_the_report();
        lookup_order_completion_and_rejection_are_validated();
        member_probe_contract_is_fail_closed();
        incomplete_block_lifecycles_are_rejected();
        inactive_hooks_are_no_ops();
    }

    fn success_report_includes_normative_totals() {
        let session = MeasurementSession::begin().expect("the session should start");

        record_semantic_cell_write();
        record_reference_write();
        record_payload_copy_bytes(17);
        record_payload_backing_allocation();
        record_element_decode();

        record_block_begin(BlockKind::New);
        record_block_seal(BlockKind::New);
        record_slot_publication(BlockKind::New);
        record_block_begin(BlockKind::Inherited);
        record_block_seal(BlockKind::Inherited);
        record_slot_publication(BlockKind::Inherited);

        record_complete_lookup();
        record_lookup_stage(LookupStage::CheckTableBrand);
        record_lookup_stage(LookupStage::CheckSlotSet);
        record_lookup_rejected();

        let report = session.finish().expect("the report should be valid");
        assert_eq!(report.semantic_cell_writes(), 1);
        assert_eq!(report.reference_writes(), 1);
        assert_eq!(report.producer_events(), 8);
        assert_eq!(report.payload_copy_bytes(), 17);
        assert_eq!(report.payload_backing_allocations(), 1);
        assert_eq!(report.block_begins(), 2);
        assert_eq!(report.block_seals(), 2);
        assert_eq!(report.slot_publications(), 2);
        assert_eq!(report.new_block_begins(), 1);
        assert_eq!(report.new_block_seals(), 1);
        assert_eq!(report.new_slot_publications(), 1);
        assert_eq!(report.inherited_block_begins(), 1);
        assert_eq!(report.inherited_block_seals(), 1);
        assert_eq!(report.inherited_slot_publications(), 1);
        assert_eq!(report.element_decodes(), 1);
        assert_eq!(report.completed_lookups(), 2);
        assert_eq!(report.rejected_lookups(), 1);
        assert_eq!(report.lookup_events(), 10);
        assert_eq!(report.max_lookup_events(), 8);
        assert_eq!(report.member_probes(), 1);
        assert_eq!(report.max_member_probes_per_lookup(), 1);
    }

    fn nested_and_concurrent_sessions_fail_closed() {
        let session = MeasurementSession::begin().expect("the outer session should start");
        assert!(matches!(
            MeasurementSession::begin(),
            Err(MeasurementError::SessionAlreadyActive)
        ));

        let concurrent_was_rejected = std::thread::spawn(|| {
            matches!(
                MeasurementSession::begin(),
                Err(MeasurementError::SessionAlreadyActive)
            )
        })
        .join()
        .expect("the concurrent begin thread should not panic");
        assert!(concurrent_was_rejected);
        record_semantic_cell_write();
        session
            .finish()
            .expect("failed begin attempts must not invalidate the owner");
    }

    fn drop_and_unwind_allow_recovery() {
        drop(MeasurementSession::begin().expect("the dropped session should start"));
        let recovered =
            MeasurementSession::begin().expect("a session should start after abandonment");
        record_semantic_cell_write();
        recovered
            .finish()
            .expect("the recovered session should finish");

        let unwind = std::panic::catch_unwind(|| {
            let _session = MeasurementSession::begin().expect("the unwind session should start");
            panic!("exercise measurement-session unwind cleanup");
        });
        assert!(unwind.is_err());

        let recovered =
            MeasurementSession::begin().expect("a session should start after unwind cleanup");
        record_semantic_cell_write();
        recovered
            .finish()
            .expect("the post-unwind session should finish");
    }

    fn overflow_invalidates_the_report() {
        let session = MeasurementSession::begin().expect("the overflow session should start");
        set_semantic_cell_writes_for_test(u64::MAX);
        record_semantic_cell_write();
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::CounterOverflow)
        ));
    }

    fn cross_thread_hooks_invalidate_the_report() {
        let session = MeasurementSession::begin().expect("the cross-thread session should start");
        std::thread::spawn(record_semantic_cell_write)
            .join()
            .expect("the hook thread should not panic");
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::CrossThreadViolation)
        ));
    }

    fn lookup_order_completion_and_rejection_are_validated() {
        let session = MeasurementSession::begin().expect("the lookup-order session should start");
        record_lookup_stage(LookupStage::CheckSlotSet);
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::LookupOrderViolation)
        ));

        let session =
            MeasurementSession::begin().expect("the incomplete-lookup session should start");
        record_lookup_stage(LookupStage::CheckTableBrand);
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::IncompleteLookup)
        ));

        let session =
            MeasurementSession::begin().expect("the rejected-lookup session should start");
        record_lookup_rejected();
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::LookupTerminationWithoutActiveLookup)
        ));
    }

    fn member_probe_contract_is_fail_closed() {
        let session =
            MeasurementSession::begin().expect("the misplaced-probe session should start");
        record_lookup_stage(LookupStage::CheckTableBrand);
        record_lookup_member_probe();
        record_lookup_rejected();
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::MemberProbeViolation)
        ));

        let session = MeasurementSession::begin().expect("the missing-probe session should start");
        record_lookup_stage(LookupStage::CheckTableBrand);
        record_lookup_stage(LookupStage::CheckSlotSet);
        record_lookup_stage(LookupStage::CheckRange);
        record_lookup_stage(LookupStage::LoadMember);
        record_lookup_stage(LookupStage::CheckVariant);
        record_lookup_stage(LookupStage::CheckSchemaTag);
        record_lookup_stage(LookupStage::ProjectView);
        record_lookup_stage(LookupStage::ReturnView);
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::MemberProbeViolation)
        ));

        let session = MeasurementSession::begin().expect("the repeated-probe session should start");
        record_lookup_stage(LookupStage::CheckTableBrand);
        record_lookup_stage(LookupStage::CheckSlotSet);
        record_lookup_stage(LookupStage::CheckRange);
        record_lookup_member_probe();
        record_lookup_member_probe();
        record_lookup_rejected();
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::MemberProbeViolation)
        ));
    }

    fn incomplete_block_lifecycles_are_rejected() {
        let session =
            MeasurementSession::begin().expect("the incomplete-block session should start");
        record_block_begin(BlockKind::New);
        record_block_seal(BlockKind::New);
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::IncompleteBlockLifecycle)
        ));

        let session = MeasurementSession::begin().expect("the invalid-block session should start");
        record_block_seal(BlockKind::Inherited);
        assert!(matches!(
            session.finish(),
            Err(MeasurementError::BlockLifecycleViolation)
        ));

        let session =
            MeasurementSession::begin().expect("the interleaved lifecycle session should start");
        record_block_begin(BlockKind::New);
        record_block_begin(BlockKind::Inherited);
        record_block_seal(BlockKind::New);
        record_block_seal(BlockKind::Inherited);
        record_slot_publication(BlockKind::Inherited);
        record_slot_publication(BlockKind::New);
        session
            .finish()
            .expect("independent block lifecycles may interleave");
    }

    fn inactive_hooks_are_no_ops() {
        record_semantic_cell_write();
        record_reference_write();
        record_payload_copy_bytes(usize::MAX);
        record_payload_backing_allocation();
        record_element_decode();
        record_block_begin(BlockKind::New);
        record_block_seal(BlockKind::New);
        record_slot_publication(BlockKind::New);
        record_lookup_stage(LookupStage::ReturnView);
        record_lookup_member_probe();
        record_lookup_rejected();

        assert!(matches!(
            MeasurementSession::begin()
                .expect("the no-op verification session should start")
                .finish(),
            Err(MeasurementError::NoObservedStoreWork)
        ));
    }

    fn record_complete_lookup() {
        record_lookup_stage(LookupStage::CheckTableBrand);
        record_lookup_stage(LookupStage::CheckSlotSet);
        record_lookup_stage(LookupStage::CheckRange);
        record_lookup_member_probe();
        record_lookup_stage(LookupStage::LoadMember);
        record_lookup_stage(LookupStage::CheckVariant);
        record_lookup_stage(LookupStage::CheckSchemaTag);
        record_lookup_stage(LookupStage::ProjectView);
        record_lookup_stage(LookupStage::ReturnView);
    }

    fn set_semantic_cell_writes_for_test(value: u64) {
        COLLECTOR_STATE
            .try_with(|local| {
                let mut state = local.get();
                state.semantic_cell_writes = value;
                local.set(state);
            })
            .expect("test thread-local state should be available");
    }
}
