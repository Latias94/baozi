#![allow(
    clippy::result_large_err,
    reason = "tests preserve the actual inline startup and invocation errors"
)]

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use baozi_operation::__retained_store_integration::activation::{
    CanonicalInvocationHost, CanonicalInvocationOutcome, CanonicalOperationPublication,
    StoreInvocationError,
};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;
use baozi_operation::{ProgressDecision, ProgressEvent};

use super::*;

fn derived_binding_plans() -> (StoreLedgerPlan, RetainedStoreStartPlan, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_work = unbounded_failure_terminal_work(keys);
    let storage_work_limit = failure_work.ordinary_attempted_work_limit() - 16;
    let storage_policy = DimensionPolicy::reservable_bytes(
        storage_work_limit,
        storage_work_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, storage_policy),
            (keys.other_dimension, DimensionPolicy::counter(16, 16, 0)),
            (
                keys.failure_terminal_dimension,
                failure_work.requirement().dimension_policy(),
            ),
        ],
    )
    .unwrap();
    let ledger = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, TEST_TERMINAL_ATTRIBUTION_SLOTS, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_work,
    );
    (ledger, start, keys)
}

#[test]
fn derived_binding_preserves_prework_account_and_original_source_through_inheritance() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let original_root = source.canonical.root_ref();
    let source_sequence_address = source
        .canonical
        .sequence(source.canonical_sequence)
        .unwrap()
        .as_slice()
        .as_ptr();
    let source_bytes_address = source
        .canonical
        .bytes(source.canonical_bytes)
        .unwrap()
        .as_ptr();
    let (ledger, start, keys) = derived_binding_plans();
    let mut invocation_root = TestInvocationRoot::new();
    let mut bound = TestOperation::bind_derived(
        &mut invocation_root,
        OperationControl::default(),
        ledger,
        &source.canonical,
        start,
    )
    .unwrap();
    let before_prework = account.snapshot();
    let mut prework_value = 0;
    let (scratch, prework_units) = bound
        .with_exclusive_execution::<_, StoreInvocationError>(|work| {
            let scratch = work.allocate_bytes(keys.phase, keys.dimension, 5)?;
            let units = work.bind_group([keys.other_dimension])?;
            work.run_action(units, [2], 2, || prework_value = 17)?;
            assert_eq!(work.work_counters(keys.other_dimension)?, (2, 2, 0));
            Ok((scratch, work.dimension_state(keys.other_dimension)?))
        })
        .unwrap();
    assert_eq!(prework_value, 17);
    let capacity = scratch.as_ref().capacity() as u64;
    assert_eq!(
        physical_delta(before_prework, account.snapshot()),
        (capacity, 1)
    );

    let mut operation = bound.attach().unwrap();
    assert!(operation.account().is_same_account(&account));
    assert!(
        operation
            .account()
            .is_same_account(source.canonical.account())
    );
    assert_eq!(source.canonical.root_ref(), original_root);
    assert_eq!(
        operation
            .producer_host()
            .dimension_state(keys.other_dimension)
            .unwrap(),
        prework_units
    );
    let before_scratch_drop = account.snapshot();
    drop(scratch);
    assert_eq!(
        physical_delta(account.snapshot(), before_scratch_drop),
        (capacity, 1)
    );

    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .unwrap();
    let before_inheritance = account.snapshot();
    assembly
        .try_inherit_all_canonical_leaves(&source.canonical, keys.dimension)
        .unwrap();
    let sequence = assembly
        .try_share_canonical(&source.canonical, source.canonical_sequence)
        .unwrap();
    let bytes = assembly
        .try_share_canonical(&source.canonical, source.canonical_bytes)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_inheritance);
    let root = assembly
        .try_rewrite_canonical_records_controlled(
            &source.canonical,
            original_root,
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceRootTag,
            |writer| writer.push(7),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.sequence_ref::<u32, SequenceTag>(0, 0) == Some(sequence)
            && root.bytes_ref::<BytesTag>(0, 1) == Some(bytes)
            && session
                .canonical_sequence(sequence)
                .and_then(|view| view.get(0))
                == Some(11)
            && session.canonical_bytes(bytes) == Some(&[12, 13][..])
            && session
                .outcome_evidence_sequence(evidence)
                .and_then(|view| view.get(0))
                == Some(7)
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("the original source must remain inheritable after bound prework"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("bound derived publication must preserve successful prework"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    let units = ledger_evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (2, 2));
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(ledger_evidence.header().active_reservations(), 0);
    assert!(
        canonical
            .account()
            .is_same_account(source.canonical.account())
    );
    assert_ne!(canonical.root_ref(), original_root);
    assert!(canonical.sequence(source.canonical_sequence).is_none());
    assert!(canonical.bytes(source.canonical_bytes).is_none());
    assert_eq!(source.canonical.root_ref(), original_root);
    drop(ledger_evidence);
    drop(invocation_root);
    drop(source);
    assert_eq!(canonical.sequence(sequence).unwrap().as_slice(), &[11]);
    assert_eq!(
        canonical.sequence(sequence).unwrap().as_slice().as_ptr(),
        source_sequence_address
    );
    assert_eq!(canonical.bytes(bytes), Some(&[12, 13][..]));
    assert_eq!(
        canonical.bytes(bytes).unwrap().as_ptr(),
        source_bytes_address
    );
    drop(canonical);
    drop(outcome_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn derived_binding_rejects_same_account_generation_before_callbacks_and_cleans_up() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let sibling = publish_sequence_source(&account);
    assert!(
        source
            .canonical
            .account()
            .is_same_account(sibling.canonical.account())
    );
    assert_eq!(source.canonical.len(), sibling.canonical.len());
    assert_ne!(source.canonical.root_ref(), sibling.canonical.root_ref());
    assert_eq!(
        sibling
            .canonical
            .sequence(sibling.canonical_sequence)
            .unwrap()
            .get(0),
        Some(11)
    );
    let sources_only = account.snapshot();
    let (ledger, start, keys) = derived_binding_plans();
    let mut invocation_root = TestInvocationRoot::new();
    let mut bound = TestOperation::bind_derived(
        &mut invocation_root,
        OperationControl::default(),
        ledger,
        &source.canonical,
        start,
    )
    .unwrap();
    let prework_called = Cell::new(false);
    bound
        .with_exclusive_execution::<_, StoreInvocationError>(|work| {
            let units = work.bind_group([keys.other_dimension])?;
            work.run_action(units, [1], 1, || prework_called.set(true))
        })
        .unwrap();
    assert!(prework_called.get());
    let operation = bound.attach().unwrap();
    assert!(
        operation
            .account()
            .is_same_account(source.canonical.account())
    );
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 2, 1))
        .unwrap();
    let before_rejections = account.snapshot();
    assert!(matches!(
        assembly.try_bind_canonical_source(&sibling.canonical, keys.other_dimension),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        assembly.try_inherit_all_canonical_leaves(&sibling.canonical, keys.other_dimension),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    let rewrite_called = Cell::new(false);
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence_controlled(
            &sibling.canonical,
            sibling.canonical_sequence,
            SequencePlan::<u32>::new(1).unwrap(),
            keys.other_dimension,
            SequenceTag,
            |writer, _control| {
                rewrite_called.set(true);
                writer.push(99).map_err(Into::into)
            },
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(!rewrite_called.get());
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), before_rejections);
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .unwrap();
    assembly
        .try_inherit_all_canonical_leaves(&source.canonical, keys.dimension)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_rejections);
    let published = assembly.prepare_minimal_fallback().unwrap().handoff();
    let units = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .unwrap();
    assert_eq!((units.attempted(), units.consumed()), (1, 1));
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
    drop(published);
    drop(invocation_root);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        sources_only.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        sources_only.live_retained_allocations()
    );
    assert_eq!(
        source
            .canonical
            .sequence(source.canonical_sequence)
            .unwrap()
            .get(0),
        Some(11)
    );
    assert_eq!(
        sibling
            .canonical
            .sequence(sibling.canonical_sequence)
            .unwrap()
            .get(0),
        Some(11)
    );
    drop(source);
    drop(sibling);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn canonical_bound_derived_body_keeps_prework_and_releases_only_its_own_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let original_root = source.canonical.root_ref();
    let sources_only = account.snapshot();
    let progress_calls = Arc::new(AtomicUsize::new(0));
    let observed_progress = Arc::clone(&progress_calls);
    let control = OperationControl::default().with_progress_observer(
        move |event: ProgressEvent<TestProgress, TestUnit>| {
            assert_eq!(event.phase(), TestProgress::Build);
            assert_eq!(event.unit(), TestUnit::Items);
            assert_eq!(event.completed(), 1);
            observed_progress.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        },
    );
    let (ledger, start, keys) = derived_binding_plans();
    let mut host = CanonicalInvocationHost::<TestOwner, TestProgress, TestUnit, (), ()>::new();
    let prework_value = Cell::new(0);
    let finish = host
        .run_bound_derived_body(
            control,
            ledger,
            &source.canonical,
            start,
            |mut parts, disposition| {
                let (scratch, prework_units) = parts
                    .startup
                    .with_exclusive_execution::<_, StoreInvocationError>(|work| {
                        let scratch = work.allocate_bytes(keys.phase, keys.dimension, 5)?;
                        let units = work.bind_group([keys.other_dimension])?;
                        work.run_action(units, [2], 2, || prework_value.set(23))?;
                        Ok((scratch, work.dimension_state(keys.other_dimension)?))
                    })
                    .unwrap();
                let capacity = scratch.as_ref().capacity() as u64;
                assert_eq!(
                    physical_delta(sources_only, account.snapshot()),
                    (capacity, 1)
                );
                let mut operation = parts.startup.attach().unwrap();
                assert_eq!(operation.physical_storage_snapshot(), account.snapshot());
                assert_eq!(
                    operation
                        .producer_host()
                        .dimension_state(keys.other_dimension)
                        .unwrap(),
                    prework_units
                );
                assert_eq!(source.canonical.root_ref(), original_root);
                let before_scratch_drop = account.snapshot();
                drop(scratch);
                assert_eq!(
                    physical_delta(account.snapshot(), before_scratch_drop),
                    (capacity, 1)
                );
                operation.request_unchanged(disposition).unwrap();
            },
        )
        .unwrap();
    assert_eq!(prework_value.get(), 23);
    assert_eq!(progress_calls.load(Ordering::SeqCst), 0);
    let CanonicalInvocationOutcome::Operation(operation) = finish
        .into_attached()
        .unwrap_or_else(|_| panic!("the derived body attached its original operation"))
        .resume()
        .outcome
    else {
        panic!("the bound derived body requested unchanged publication");
    };
    let CanonicalOperationPublication::Unchanged(PreparedStoreEvidenceOnlyOutcome::Success(
        prepared,
    )) = operation.finish(final_progress()).unwrap()
    else {
        panic!("bound derived prework must complete through the outer host");
    };
    assert_eq!(progress_calls.load(Ordering::SeqCst), 1);
    let evidence = prepared.handoff().ledger_evidence;
    let units = evidence.dimension(keys.other_dimension).unwrap();
    assert_eq!((units.attempted(), units.consumed()), (2, 2));
    assert_eq!(evidence.header().current_live_bytes(), 0);
    assert_eq!(evidence.header().active_reservations(), 0);
    drop(evidence);
    drop(host);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        sources_only.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        sources_only.live_retained_allocations()
    );
    assert_eq!(source.canonical.root_ref(), original_root);
    assert_eq!(
        source
            .canonical
            .sequence(source.canonical_sequence)
            .unwrap()
            .get(0),
        Some(11)
    );
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}
