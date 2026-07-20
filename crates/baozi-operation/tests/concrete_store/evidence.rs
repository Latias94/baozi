use super::*;

#[test]
fn minimal_fallback_keeps_two_by_two_terminal_evidence_after_producer_unload() {
    let factory = factory(1);
    let account = account(&factory);
    let producer = fake_producer();
    let producer_unloaded = Arc::downgrade(&producer);
    let TestVocabulary { domain, keys } = test_vocabulary_for(producer.catalog);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [(keys.dimension, policy), (keys.other_dimension, policy)],
    )
    .unwrap();
    let ledger_plan =
        StoreLedgerPlan::try_new(domain, limits, &[keys.phase, keys.other_phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, 32, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start)
            .unwrap();
    operation.try_complete_phase(keys.phase).unwrap();
    operation.try_complete_phase(keys.other_phase).unwrap();
    let prepared = operation.prepare_minimal_fallback(75).unwrap();

    drop(producer);
    assert!(
        producer_unloaded.upgrade().is_none(),
        "prepared fallback must not retain executable producer state"
    );
    let published = prepared.handoff();
    let fallback = published.fallback;
    let evidence = published.ledger_evidence;
    assert!(account.is_same_account(fallback.account()));
    assert_eq!(fallback.value().code(), 91);
    drop(fallback);

    let first_dimension = evidence
        .dimension_by_semantic(keys.dimension.semantic_id())
        .expect("first semantic dimension");
    let second_dimension = evidence
        .dimension_by_semantic(keys.other_dimension.semantic_id())
        .expect("second semantic dimension");
    assert_eq!(evidence.dimension_at(0), Some(first_dimension));
    assert_eq!(evidence.dimension_at(1), Some(second_dimension));
    assert_ne!(first_dimension.dimension(), second_dimension.dimension());

    let first_phase = evidence
        .phase_by_semantic(keys.phase.semantic_id())
        .expect("first semantic phase");
    let second_phase = evidence
        .phase_by_semantic(keys.other_phase.semantic_id())
        .expect("second semantic phase");
    assert_eq!(evidence.phase_at(0), Some(first_phase));
    assert_eq!(evidence.phase_at(1), Some(second_phase));
    assert_ne!(first_phase.phase(), second_phase.phase());
    assert_eq!(evidence.dimension_at(2), None);
    assert_eq!(evidence.phase_at(2), None);
    assert_eq!(evidence.header().dimension_count(), 2);
    assert_eq!(evidence.header().phase_count(), 2);

    drop(evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
