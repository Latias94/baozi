use super::*;
use crate::ledger::Ledger;
use crate::{
    Commitment, DimensionPolicy, LedgerLimits, OperationOwner, PublicationReservationBatch,
};
use std::error::Error;
use std::mem::{align_of, needs_drop, size_of};
use std::sync::Barrier;
use std::sync::atomic::Ordering;
use std::thread;

struct EvidenceOwner;

impl OperationOwner for EvidenceOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum TestDimension {
    Bytes,
    Items,
    Work,
}

impl TerminalEvidenceKeySeal for TestDimension {}

impl TerminalEvidenceKey for TestDimension {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum TestPhase {
    Read,
    Decode,
    Validate,
    Publish,
    Finalize,
}

impl TerminalEvidenceKeySeal for TestPhase {}

impl TerminalEvidenceKey for TestPhase {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
struct ShapeKey(u8);

impl TerminalEvidenceKeySeal for ShapeKey {}

impl TerminalEvidenceKey for ShapeKey {}

fn terminal_snapshot() -> LedgerSnapshot<TestPhase, TestDimension> {
    let ledger = Ledger::<EvidenceOwner, TestPhase, TestDimension>::try_new_with_phase_catalog(
        test_limits(),
        &[TestPhase::Read, TestPhase::Decode],
    )
    .expect("terminal phase catalog is frozen");
    ledger
        .charge_attempted(TestDimension::Bytes, 17)
        .expect("attempted evidence fits");
    ledger
        .charge_consumed(TestDimension::Bytes, 11)
        .expect("consumed evidence fits");
    let read = ledger
        .reserve(TestPhase::Read, TestDimension::Bytes, 23)
        .expect("read reservation fits");
    let decode = ledger
        .reserve(TestPhase::Decode, TestDimension::Items, 7)
        .expect("decode reservation fits");
    drop(decode);
    drop(read);
    ledger
        .try_complete_phase(TestPhase::Decode)
        .expect("decode phase completes explicitly");
    ledger.finish().expect("released reservations finalize")
}

fn test_ledger() -> Ledger<EvidenceOwner, TestPhase, TestDimension> {
    Ledger::new(test_limits())
}

fn test_limits() -> LedgerLimits<TestDimension> {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX);
    LedgerLimits::new(
        u64::MAX,
        [
            (TestDimension::Bytes, policy),
            (TestDimension::Items, policy),
        ],
    )
    .expect("test policies are ordered and bounded")
}

fn build_evidence(
    dimension_keys: &[TestDimension],
    phase_keys: &[TestPhase],
) -> TerminalLedgerEvidence<TestPhase, TestDimension> {
    TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(
        dimension_keys.len(),
        phase_keys.len(),
    )
    .expect("small backing plan fits")
    .try_build(dimension_keys, phase_keys)
    .expect("test catalogs are ordered and match the plan")
}

fn terminal_evidence() -> TerminalLedgerEvidence<TestPhase, TestDimension> {
    build_evidence(
        &[TestDimension::Bytes, TestDimension::Items],
        &[TestPhase::Read, TestPhase::Decode],
    )
}

#[test]
fn terminal_evidence_freezes_exact_header_cells_and_queries() {
    let snapshot = terminal_snapshot();
    let evidence = terminal_evidence();
    assert!(!evidence.is_frozen());
    assert!(evidence.header().is_none());
    assert!(evidence.dimension(TestDimension::Bytes).is_none());
    assert!(evidence.phase(TestPhase::Read).is_none());

    let cause = PreparationFailureCode::domain(47);
    evidence
        .try_freeze_from(TerminalEvidenceCause::Failure(cause), &snapshot)
        .expect("exact terminal evidence freezes");

    let header = evidence.header().expect("frozen header is published");
    assert_eq!(
        header.schema_revision(),
        TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION
    );
    assert_eq!(
        header.terminal_cause(),
        TerminalEvidenceCause::Failure(cause)
    );
    assert_eq!(header.last_completed_phase(), Some(TestPhase::Decode));
    assert_eq!(
        evidence
            .header
            .last_completed_phase_ordinal_plus_one
            .load(Ordering::Relaxed),
        2
    );
    assert_eq!(header.dimension_count(), snapshot.dimensions().len());
    assert_eq!(header.phase_count(), snapshot.phases().len());
    assert_eq!(header.current_live_bytes(), snapshot.current_live_bytes());
    assert_eq!(header.peak_live_bytes(), snapshot.peak_live_bytes());
    assert_eq!(header.active_reservations(), snapshot.active_reservations());
    assert_eq!(
        header.current_evidence_bytes(),
        snapshot.current_evidence_bytes()
    );
    assert_eq!(header.peak_evidence_bytes(), snapshot.peak_evidence_bytes());
    assert_eq!(
        evidence.dimension(TestDimension::Bytes),
        snapshot.dimension(TestDimension::Bytes).copied()
    );
    assert_eq!(
        evidence.phase(TestPhase::Decode),
        snapshot.phase(TestPhase::Decode).copied()
    );
    assert!(evidence.dimension(TestDimension::Work).is_none());
    assert!(evidence.phase(TestPhase::Validate).is_none());
    assert_eq!(
        evidence.try_freeze_from(TerminalEvidenceCause::Success, &snapshot),
        Err(TerminalEvidenceFreezeError::AlreadyFrozen)
    );
}

#[test]
fn terminal_evidence_does_not_infer_completion_from_the_last_accounting_phase() {
    let ledger = Ledger::<EvidenceOwner, TestPhase, TestDimension>::try_new_with_phase_catalog(
        test_limits(),
        &[TestPhase::Read, TestPhase::Decode],
    )
    .expect("terminal phase catalog is frozen");
    let decode_writer = ledger
        .reserve(TestPhase::Decode, TestDimension::Bytes, 5)
        .expect("decode writer reservation fits");
    drop(decode_writer);
    ledger
        .try_complete_phase(TestPhase::Read)
        .expect("read completes explicitly");
    let snapshot = ledger.finish().expect("writer reservation is released");
    assert_eq!(
        snapshot
            .phase(TestPhase::Decode)
            .expect("decode accounting evidence")
            .peak_live_bytes(),
        5
    );

    let evidence = terminal_evidence();
    evidence
        .try_freeze_from(TerminalEvidenceCause::Success, &snapshot)
        .expect("terminal evidence freezes");

    assert_eq!(
        evidence
            .header()
            .expect("frozen header")
            .last_completed_phase(),
        Some(TestPhase::Read)
    );
}

#[test]
fn terminal_evidence_preserves_completion_when_catalog_position_opposes_key_order() {
    let ledger = Ledger::<EvidenceOwner, TestPhase, TestDimension>::try_new_with_phase_catalog(
        test_limits(),
        &[TestPhase::Decode, TestPhase::Read],
    )
    .expect("reverse-order terminal phase catalog is frozen");
    ledger
        .try_complete_phase(TestPhase::Decode)
        .expect("first catalog position completes");
    ledger
        .try_complete_phase(TestPhase::Read)
        .expect("second catalog position completes despite its earlier key order");
    let snapshot = ledger.finish().expect("terminal ledger finishes");
    assert_eq!(
        snapshot
            .phases()
            .map(|(phase, _)| phase)
            .collect::<Vec<_>>(),
        vec![TestPhase::Read, TestPhase::Decode]
    );

    let evidence = terminal_evidence();
    evidence
        .try_freeze_from(TerminalEvidenceCause::Success, &snapshot)
        .expect("terminal evidence freezes");

    assert_eq!(
        evidence
            .header()
            .expect("frozen header")
            .last_completed_phase(),
        Some(TestPhase::Read)
    );
    assert_eq!(
        evidence
            .header
            .last_completed_phase_ordinal_plus_one
            .load(Ordering::Relaxed),
        1,
        "the stored header ordinal follows the validated key catalog, not state_position"
    );
}

#[test]
fn atomic_cells_round_trip_every_snapshot_scalar() {
    let dimension = FallbackDimensionCell::new(TestDimension::Bytes);
    let expected_dimension = DimensionSnapshot::from_terminal_evidence(DimensionSnapshotScalars {
        attempted: 1,
        consumed: 2,
        current_live_bytes: 3,
        peak_live_bytes: 4,
        pending_publication: 5,
        committed: 6,
        committed_canonical_output: 7,
        published_outcome_evidence: 8,
    });
    dimension.write(&expected_dimension);
    assert_eq!(dimension.snapshot(), expected_dimension);

    let phase = FallbackPhaseCell::new(TestPhase::Read);
    let expected_phase = PhaseSnapshot::from_terminal_evidence(9, 10);
    phase.write(&expected_phase);
    assert_eq!(phase.snapshot(), expected_phase);
}

#[test]
fn terminal_evidence_rejects_shape_and_keys_before_any_cell_write() {
    let snapshot = terminal_snapshot();
    let shape = build_evidence(&[TestDimension::Bytes], &[]);
    assert_eq!(
        shape.try_freeze_from(TerminalEvidenceCause::Success, &snapshot),
        Err(TerminalEvidenceFreezeError::ShapeMismatch {
            expected_dimensions: 1,
            actual_dimensions: 2,
            expected_phases: 0,
            actual_phases: 2,
        })
    );
    assert!(!shape.is_frozen());
    assert_evidence_cells_pristine(&shape);

    let wrong_dimension = build_evidence(
        &[TestDimension::Bytes, TestDimension::Work],
        &[TestPhase::Read, TestPhase::Decode],
    );
    assert_eq!(
        wrong_dimension.try_freeze_from(TerminalEvidenceCause::Success, &snapshot),
        Err(TerminalEvidenceFreezeError::DimensionKeyMismatch { index: 1 })
    );
    assert_evidence_cells_pristine(&wrong_dimension);

    let wrong_phase = build_evidence(
        &[TestDimension::Bytes, TestDimension::Items],
        &[TestPhase::Read, TestPhase::Validate],
    );
    assert_eq!(
        wrong_phase.try_freeze_from(TerminalEvidenceCause::Success, &snapshot),
        Err(TerminalEvidenceFreezeError::PhaseKeyMismatch { index: 1 })
    );
    assert_evidence_cells_pristine(&wrong_phase);
}

#[test]
fn terminal_evidence_rejects_unsorted_and_duplicate_catalogs() {
    let duplicate_dimensions =
        TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(2, 0)
            .expect("the shape itself fits")
            .try_build(&[TestDimension::Bytes, TestDimension::Bytes], &[]);
    assert!(matches!(
        duplicate_dimensions,
        Err(TerminalEvidenceBuildError::Catalog(
            TerminalEvidenceCatalogError::DimensionKeysNotStrictlyOrdered { index: 1 }
        ))
    ));

    let unsorted_phases = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(0, 2)
        .expect("the shape itself fits")
        .try_build(&[], &[TestPhase::Decode, TestPhase::Read]);
    assert!(matches!(
        unsorted_phases,
        Err(TerminalEvidenceBuildError::Catalog(
            TerminalEvidenceCatalogError::PhaseKeysNotStrictlyOrdered { index: 1 }
        ))
    ));
}

#[test]
fn backing_builder_rejects_shape_before_catalog_or_allocator_entry() {
    let plan = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(2, 1)
        .expect("the planned shape fits");
    assert!(matches!(
        plan.try_build(&[TestDimension::Bytes], &[TestPhase::Read]),
        Err(TerminalEvidenceBuildError::ShapeMismatch {
            planned_dimensions: 2,
            actual_dimensions: 1,
            planned_phases: 1,
            actual_phases: 1,
        })
    ));
}

#[test]
fn backing_builder_maps_each_allocator_failure_and_remains_reusable() {
    let plan = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(2, 2)
        .expect("the planned shape fits");
    let dimensions = [TestDimension::Bytes, TestDimension::Items];
    let phases = [TestPhase::Read, TestPhase::Decode];

    fail_next_dimension_backing_allocation();
    assert!(matches!(
        plan.try_build(&dimensions, &phases),
        Err(TerminalEvidenceBuildError::DimensionAllocationFailed)
    ));

    fail_next_phase_backing_allocation();
    assert!(matches!(
        plan.try_build(&dimensions, &phases),
        Err(TerminalEvidenceBuildError::PhaseAllocationFailed)
    ));

    let evidence = plan
        .try_build(&dimensions, &phases)
        .expect("consumed failpoints leave the plan usable");
    assert_eq!(evidence.dimensions.len(), dimensions.len());
    assert_eq!(evidence.phases.len(), phases.len());
}

fn assert_evidence_cells_pristine(evidence: &TerminalLedgerEvidence<TestPhase, TestDimension>) {
    assert!(evidence.header.snapshot(&evidence.phases).is_none());
    assert!(!evidence.freeze_started.load(Ordering::Acquire));
    assert!(!evidence.is_frozen());
    assert_eq!(
        evidence.header.terminal_cause.load(Ordering::Relaxed),
        TERMINAL_CAUSE_UNSET
    );
    assert_eq!(evidence.header.failure_class.load(Ordering::Relaxed), 0);
    assert_eq!(evidence.header.failure_code.load(Ordering::Relaxed), 0);
    assert_eq!(
        evidence
            .header
            .last_completed_phase_ordinal_plus_one
            .load(Ordering::Relaxed),
        0
    );
    assert!(evidence.dimensions.iter().all(|cell| {
        cell.attempted.load(Ordering::Relaxed) == 0
            && cell.consumed.load(Ordering::Relaxed) == 0
            && cell.current_live_bytes.load(Ordering::Relaxed) == 0
            && cell.peak_live_bytes.load(Ordering::Relaxed) == 0
            && cell.pending_publication.load(Ordering::Relaxed) == 0
            && cell.committed.load(Ordering::Relaxed) == 0
            && cell.committed_canonical_output.load(Ordering::Relaxed) == 0
            && cell.published_outcome_evidence.load(Ordering::Relaxed) == 0
    }));
    assert!(evidence.phases.iter().all(|cell| {
        cell.current_live_bytes.load(Ordering::Relaxed) == 0
            && cell.peak_live_bytes.load(Ordering::Relaxed) == 0
    }));
}

#[test]
fn nonterminal_phase_dimension_and_global_states_reject_before_any_cell_write() {
    let phase_ledger = test_ledger();
    let phase_live = phase_ledger
        .reserve(TestPhase::Read, TestDimension::Bytes, 9)
        .expect("phase reservation fits");
    let phase_snapshot = phase_ledger
        .try_snapshot()
        .expect("interim phase snapshot fits");
    let phase_evidence = build_evidence(
        &[TestDimension::Bytes, TestDimension::Items],
        &[TestPhase::Read],
    );
    assert_eq!(
        phase_evidence.try_freeze_from(TerminalEvidenceCause::Success, &phase_snapshot),
        Err(TerminalEvidenceFreezeError::NonTerminalPhaseState {
            index: 0,
            current_live_bytes: 9,
        })
    );
    assert_evidence_cells_pristine(&phase_evidence);
    drop(phase_live);
    drop(phase_ledger);

    let pending_ledger = test_ledger();
    let mut empty_batch = PublicationReservationBatch::try_new(Vec::new(), None)
        .expect("an empty covered batch is valid");
    let pending = pending_ledger
        .prepare_publication(
            &mut empty_batch,
            &[Commitment::new(TestDimension::Bytes, 3)],
        )
        .expect("zero-live publication can retain a pending commitment");
    let pending_snapshot = pending_ledger
        .try_snapshot()
        .expect("interim pending snapshot fits");
    let pending_evidence = build_evidence(&[TestDimension::Bytes, TestDimension::Items], &[]);
    assert_eq!(
        pending_evidence.try_freeze_from(TerminalEvidenceCause::Success, &pending_snapshot),
        Err(TerminalEvidenceFreezeError::NonTerminalDimensionState {
            index: 0,
            current_live_bytes: 0,
            pending_publication: 3,
        })
    );
    assert_evidence_cells_pristine(&pending_evidence);
    drop(pending);
    drop(pending_ledger);

    let global_ledger = test_ledger();
    let zero_live = global_ledger
        .reserve(TestPhase::Read, TestDimension::Bytes, 0)
        .expect("zero-byte reservation is still a live capability");
    let global_snapshot = global_ledger
        .try_snapshot()
        .expect("interim global snapshot fits");
    let global_evidence = build_evidence(
        &[TestDimension::Bytes, TestDimension::Items],
        &[TestPhase::Read],
    );
    assert_eq!(
        global_evidence.try_freeze_from(TerminalEvidenceCause::Success, &global_snapshot),
        Err(TerminalEvidenceFreezeError::NonTerminalGlobalState {
            current_live_bytes: global_snapshot.current_live_bytes(),
            active_reservations: global_snapshot.active_reservations(),
            current_evidence_bytes: global_snapshot.current_evidence_bytes(),
        })
    );
    assert_evidence_cells_pristine(&global_evidence);
    drop(zero_live);
    drop(global_ledger);
}

#[test]
fn semantic_layout_matches_ordered_layout_composition_for_zero_one_and_regular_counts() {
    for (dimension_count, phase_count) in [(0, 0), (1, 1), (3, 5)] {
        let actual = FallbackSemanticLayout::try_new::<TestPhase, TestDimension>(
            dimension_count,
            phase_count,
        )
        .expect("small semantic layout fits");
        let header = Layout::new::<FallbackHeader<TestPhase>>();
        let dimensions = Layout::array::<FallbackDimensionCell<TestDimension>>(dimension_count)
            .expect("small dimension layout fits");
        let phases = Layout::array::<FallbackPhaseCell<TestPhase>>(phase_count)
            .expect("small phase layout fits");
        let (expected, dimension_offset) = header
            .extend(dimensions)
            .expect("small dimension extension fits");
        let (expected, phase_offset) = expected.extend(phases).expect("small phase extension fits");
        let expected = expected.pad_to_align();
        assert_eq!(actual.complete(), expected);
        assert_eq!(actual.dimension_offset(), dimension_offset);
        assert_eq!(actual.phase_offset(), phase_offset);
        assert_eq!(actual.complete().size() % actual.complete().align(), 0);
    }
}

#[test]
fn shared_freeze_reference_linearizes_one_complete_snapshot() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TerminalLedgerEvidence<TestPhase, TestDimension>>();

    let snapshot = terminal_snapshot();
    let evidence = terminal_evidence();
    let barrier = Barrier::new(3);
    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            evidence.try_freeze_from(TerminalEvidenceCause::Success, &snapshot)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            evidence.try_freeze_from(
                TerminalEvidenceCause::Failure(PreparationFailureCode::domain(99)),
                &snapshot,
            )
        });
        barrier.wait();
        (
            first.join().expect("first freezer does not panic"),
            second.join().expect("second freezer does not panic"),
        )
    });
    assert!(matches!(
        (first, second),
        (Ok(()), Err(TerminalEvidenceFreezeError::AlreadyFrozen))
            | (Err(TerminalEvidenceFreezeError::AlreadyFrozen), Ok(()))
    ));
    assert!(evidence.is_frozen());
    assert!(evidence.header().is_some());
    assert!(
        evidence
            .dimensions
            .iter()
            .all(|cell| evidence.dimension(cell.key()).is_some())
    );
    assert!(
        evidence
            .phases
            .iter()
            .all(|cell| evidence.phase(cell.key()).is_some())
    );
}

fn count_that_overflows_extend<T>(prefix: Layout) -> usize {
    let maximum = (isize::MAX as usize) / size_of::<T>();
    (0..1024)
        .filter_map(|delta| maximum.checked_sub(delta))
        .find(|count| Layout::array::<T>(*count).is_ok_and(|array| prefix.extend(array).is_err()))
        .expect("a maximum-sized array leaves no room for the nonempty prefix")
}

#[test]
fn semantic_layout_reports_each_reachable_array_and_extend_overflow() {
    assert_eq!(
        FallbackSemanticLayout::try_new::<TestPhase, TestDimension>(usize::MAX, 0),
        Err(FallbackSemanticLayoutError::DimensionArray)
    );
    assert_eq!(
        FallbackSemanticLayout::try_new::<TestPhase, TestDimension>(0, usize::MAX),
        Err(FallbackSemanticLayoutError::PhaseArray)
    );

    let header = Layout::new::<FallbackHeader<TestPhase>>();
    let dimension_count =
        count_that_overflows_extend::<FallbackDimensionCell<TestDimension>>(header);
    assert_eq!(
        FallbackSemanticLayout::try_new::<TestPhase, TestDimension>(dimension_count, 0),
        Err(FallbackSemanticLayoutError::DimensionExtend)
    );

    let one_dimension =
        Layout::array::<FallbackDimensionCell<TestDimension>>(1).expect("one dimension cell fits");
    let (phase_prefix, _) = header
        .extend(one_dimension)
        .expect("one dimension extension fits");
    let phase_count = count_that_overflows_extend::<FallbackPhaseCell<TestPhase>>(phase_prefix);
    assert_eq!(
        FallbackSemanticLayout::try_new::<TestPhase, TestDimension>(1, phase_count),
        Err(FallbackSemanticLayoutError::PhaseExtend)
    );
}

#[test]
fn backing_plan_keeps_actual_vec_backings_independent() {
    let plan = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(3, 5)
        .expect("small backing plans fit");
    let dimensions = plan.dimension_capacity();
    let phases = plan.phase_capacity();
    assert_eq!(dimensions.desired_capacity(), 3);
    assert_eq!(dimensions.envelope_capacity(), 4);
    assert_eq!(phases.desired_capacity(), 5);
    assert_eq!(phases.envelope_capacity(), 8);
    assert_eq!(
        dimensions
            .envelope_delta_bytes()
            .expect("dimension bytes fit"),
        u64::try_from(4 * size_of::<FallbackDimensionCell<TestDimension>>())
            .expect("test target size fits u64")
    );
    assert_eq!(
        phases.envelope_delta_bytes().expect("phase bytes fit"),
        u64::try_from(8 * size_of::<FallbackPhaseCell<TestPhase>>())
            .expect("test target size fits u64")
    );

    let evidence = plan
        .try_build(
            &[
                TestDimension::Bytes,
                TestDimension::Items,
                TestDimension::Work,
            ],
            &[
                TestPhase::Read,
                TestPhase::Decode,
                TestPhase::Validate,
                TestPhase::Publish,
                TestPhase::Finalize,
            ],
        )
        .expect("the checked builder binds both planned backings");
    assert_eq!(evidence.dimensions.len(), 3);
    assert!(evidence.dimensions.capacity() >= dimensions.desired_capacity());
    assert!(evidence.dimensions.capacity() <= dimensions.envelope_capacity());
    assert_eq!(evidence.phases.len(), 5);
    assert!(evidence.phases.capacity() >= phases.desired_capacity());
    assert!(evidence.phases.capacity() <= phases.envelope_capacity());
}

#[test]
fn backing_plan_checked_sums_bytes_and_nonempty_allocation_count() {
    for (dimension_count, phase_count, expected_allocations) in
        [(0, 0, 0), (1, 0, 1), (0, 1, 1), (1, 1, 2), (3, 5, 2)]
    {
        let plan = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(
            dimension_count,
            phase_count,
        )
        .expect("small backing plans fit");
        let expected_bytes = plan
            .dimension_capacity()
            .envelope_delta_bytes()
            .expect("dimension bytes fit")
            .checked_add(
                plan.phase_capacity()
                    .envelope_delta_bytes()
                    .expect("phase bytes fit"),
            )
            .expect("small byte sum fits");
        assert_eq!(plan.footprint().bytes(), expected_bytes);
        assert_eq!(plan.footprint().allocations(), expected_allocations);
    }
}

#[test]
fn semantic_aggregate_is_not_used_as_a_physical_backing_charge() {
    let plan = TerminalEvidenceBackingPlan::<TestPhase, TestDimension>::try_new(3, 5)
        .expect("small backing plans fit");
    assert_ne!(
        u64::try_from(plan.semantic_layout().complete().size())
            .expect("semantic layout size fits u64"),
        plan.footprint().bytes()
    );
    assert_eq!(plan.footprint().allocations(), 2);
}

#[test]
fn nested_backing_aggregation_rejects_nonnew_plans_and_checked_sum_overflow() {
    let nonnew = CapacityPlan::for_type::<u64>(1, 1, 2).expect("nonnew plan is otherwise valid");
    assert_eq!(
        try_new_backing_footprint(&[nonnew]),
        Err(TerminalEvidenceBackingPlanError::BackingPlanIsNotNew {
            index: 0,
            original_len: 1,
            original_capacity: 1,
        })
    );

    #[cfg(target_pointer_width = "64")]
    {
        let large = CapacityPlan::for_type::<u8>(0, 0, 1_usize << 62)
            .expect("one large request is still below isize::MAX");
        assert_eq!(
            try_new_backing_footprint(&[large; 4]),
            Err(TerminalEvidenceBackingPlanError::NestedByteCountOverflow)
        );
    }
}

#[test]
fn terminal_evidence_errors_preserve_typed_sources() {
    let capacity = CapacityPlanError::CapacityOverflow;
    let semantic = FallbackSemanticLayoutError::PhaseExtend;
    let catalog = TerminalEvidenceCatalogError::PhaseKeysNotStrictlyOrdered { index: 1 };

    let semantic_error = TerminalEvidenceBackingPlanError::SemanticLayout(semantic);
    assert_eq!(
        semantic_error
            .source()
            .and_then(|cause| cause.downcast_ref::<FallbackSemanticLayoutError>()),
        Some(&semantic)
    );
    for error in [
        TerminalEvidenceBackingPlanError::DimensionBacking(capacity),
        TerminalEvidenceBackingPlanError::PhaseBacking(capacity),
        TerminalEvidenceBackingPlanError::BackingFootprint {
            index: 0,
            cause: capacity,
        },
    ] {
        assert_eq!(
            error
                .source()
                .and_then(|cause| cause.downcast_ref::<CapacityPlanError>()),
            Some(&capacity)
        );
    }

    let catalog_error = TerminalEvidenceBuildError::Catalog(catalog);
    assert_eq!(
        catalog_error
            .source()
            .and_then(|cause| cause.downcast_ref::<TerminalEvidenceCatalogError>()),
        Some(&catalog)
    );
    for error in [
        TerminalEvidenceBuildError::DimensionCapacity(capacity),
        TerminalEvidenceBuildError::PhaseCapacity(capacity),
    ] {
        assert_eq!(
            error
                .source()
                .and_then(|cause| cause.downcast_ref::<CapacityPlanError>()),
            Some(&capacity)
        );
    }
}

#[test]
fn terminal_evidence_key_marker_is_sealed_to_the_two_production_store_keys() {
    let evidence_source = include_str!("../evidence.rs");
    let store_source = include_str!("../../concrete_store.rs");
    assert!(evidence_source.contains("pub(crate) trait TerminalEvidenceKey:"));
    assert!(evidence_source.contains("P: TerminalEvidenceKey"));
    assert!(evidence_source.contains("D: TerminalEvidenceKey"));
    assert!(!evidence_source.contains("TerminalEvidenceKey for fn"));
    assert!(!evidence_source.contains("impl<T> TerminalEvidenceKey"));
    assert!(!evidence_source.contains("OnceLock"));
    assert!(!evidence_source.contains("unsafe"));
    assert!(!evidence_source.contains("transmute"));
    assert_eq!(
        store_source
            .matches("impl crate::retained::TerminalEvidenceKey for")
            .count(),
        2
    );
    assert!(store_source.contains("TerminalEvidenceKey for StorePhaseKey"));
    assert!(store_source.contains("TerminalEvidenceKey for StoreDimensionKey"));
}

#[test]
fn terminal_evidence_leaf_shapes_are_inert_and_fixed_for_copy_keys() {
    assert!(!needs_drop::<PreparationFailureCode>());
    assert!(!needs_drop::<TerminalEvidenceCause>());
    assert!(!needs_drop::<FallbackHeaderSnapshot<TestPhase>>());
    assert!(!needs_drop::<TestDimension>());
    assert!(!needs_drop::<TestPhase>());
    assert!(!needs_drop::<FallbackHeader<TestPhase>>());
    assert!(!needs_drop::<FallbackDimensionCell<TestDimension>>());
    assert!(!needs_drop::<FallbackPhaseCell<TestPhase>>());
    assert!(!needs_drop::<FallbackSemanticLayout>());
    assert!(!needs_drop::<
        TerminalEvidenceBackingPlan<TestPhase, TestDimension>,
    >());
    assert!(needs_drop::<TerminalLedgerEvidence<TestPhase, TestDimension>>());

    assert_eq!(
        size_of::<FallbackDimensionCell<TestDimension>>(),
        size_of::<FallbackDimensionCell<ShapeKey>>()
    );
    assert_eq!(
        align_of::<FallbackDimensionCell<TestDimension>>(),
        align_of::<FallbackDimensionCell<ShapeKey>>()
    );
    assert_eq!(
        size_of::<FallbackPhaseCell<TestPhase>>(),
        size_of::<FallbackPhaseCell<ShapeKey>>()
    );
    assert_eq!(
        align_of::<FallbackPhaseCell<TestPhase>>(),
        align_of::<FallbackPhaseCell<ShapeKey>>()
    );
}
