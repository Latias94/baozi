use super::*;

#[test]
fn builder_panic_is_fatal_and_cannot_publish_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let before = account.snapshot();
    let error = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |_writer| -> Result<(), baozi_operation::StoreBuildError> {
                panic!("contained builder panic")
            },
        )
        .unwrap_err();
    assert!(matches!(error, StoreRuntimeError::BuilderPanicked));
    let after = account.snapshot();
    assert!(after.poisoned());
    assert_eq!(
        after
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert!(after.live_retained_bytes() > before.live_retained_bytes());
    assert!(after.live_retained_allocations() > before.live_retained_allocations());
    assert!(matches!(
        assembly.try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(1_u8)),
        ),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    for dimension in [foreign_dimension(), keys.other_dimension] {
        assert!(matches!(
            assembly.try_commit_canonical(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        assert!(matches!(
            assembly.try_commit_outcome_evidence(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn failure_assembly_commit_preserves_fallback_first_error_precedence() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 2))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(7_u8)),
        )
        .unwrap();
    let mut failure = match assembly.validate_success(canonical, evidence, |_session, _, _| false) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must produce a failure assembly"),
    };
    let error = failure
        .try_retain_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |_writer| -> Result<(), baozi_operation::StoreBuildError> {
                panic!("contained failure builder panic")
            },
        )
        .unwrap_err();
    assert!(matches!(error, StoreRuntimeError::BuilderPanicked));

    for dimension in [foreign_dimension(), keys.other_dimension] {
        assert!(matches!(
            failure.try_commit(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    assert!(matches!(
        failure.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn semantic_rejection_can_publish_rich_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let producer = fake_producer();
    let producer_unloaded = Arc::downgrade(&producer);
    let (operation, keys) = operation_with_catalog_and_evidence_limit(
        &account,
        OperationControl::default(),
        producer.catalog,
        u64::MAX,
    );
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(7_u8)),
        )
        .unwrap();

    let failure = match assembly.validate_success(canonical, evidence, |_session, _, _| false) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must preserve an evidence-only builder"),
    };
    let validated = match failure.validate_rich_failure(evidence, |session, evidence| {
        session
            .records(evidence)
            .and_then(|records| records.scalar::<u8>(0, 0))
            == Some(7)
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("evidence-only graph must validate"),
    };
    let prepared = match validated.try_prepare(42, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated evidence must prepare rich failure")
        }
    };
    drop(producer);
    assert!(
        producer_unloaded.upgrade().is_none(),
        "prepared rich failure must not retain executable producer state"
    );
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(
        outcome_evidence
            .outcome_evidence()
            .root()
            .scalar::<u8>(0, 0),
        Some(7)
    );
    drop(outcome_evidence);

    assert_eq!(store_failure_cause(&ledger_evidence).code(), 42);
    assert_semantic_and_ordinal_evidence_queries(&ledger_evidence, keys);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn success_primary_rejection_returns_the_exact_minimal_fallback_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_evidence_limit(&account, 4);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |writer| writer.push(7),
        )
        .unwrap();
    assembly.try_commit_canonical(keys.dimension, 9).unwrap();
    assembly
        .try_commit_outcome_evidence(keys.dimension, 4)
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some() && session.outcome_evidence_sequence(e).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("success primary-rejection graph must validate"),
    };
    let physical_before_prepare = account.snapshot();
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::MinimalFallback(prepared) => prepared,
        PreparedStoreSuccessOutcome::Success(_) => {
            panic!("primary ledger rejection must not expose canonical success")
        }
        PreparedStoreSuccessOutcome::RichFailure(_) => {
            panic!("primary ledger rejection must not retry as rich failure")
        }
    };
    let physical_after_prepare = account.snapshot();
    assert!(
        physical_after_prepare.live_retained_bytes()
            < physical_before_prepare.live_retained_bytes()
    );
    assert!(
        physical_after_prepare.live_retained_allocations()
            < physical_before_prepare.live_retained_allocations()
    );

    let published = prepared.handoff();
    assert!(account.is_same_account(published.fallback.account()));
    assert_eq!(published.fallback.value().code(), 91);
    assert_semantic_and_ordinal_evidence_queries(&published.ledger_evidence, keys);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(store_failure_cause(&published.ledger_evidence).code(), 1);
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .expect("success rejection ledger evidence")
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn rich_failure_primary_rejection_returns_the_exact_minimal_fallback_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_evidence_limit(&account, 4);
    let mut assembly = operation
        .try_publication_assembly(rich_failure_plan(keys, 1))
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_bytes(BytesPlan::new(2).unwrap(), EvidenceBytesTag, |writer| {
            writer.write_bytes(&[7, 8])
        })
        .unwrap();
    assembly
        .try_commit_outcome_evidence(keys.dimension, 4)
        .unwrap();
    let validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
        session.bytes(evidence) == Some(&[7, 8][..])
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("rich-failure primary-rejection graph must validate"),
    };
    let physical_before_prepare = account.snapshot();
    let prepared = match validated.try_prepare(73, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::MinimalFallback(prepared) => prepared,
        PreparedStoreFailureOutcome::RichFailure(_) => {
            panic!("primary ledger rejection must not expose rich failure")
        }
    };
    let physical_after_prepare = account.snapshot();
    assert!(
        physical_after_prepare.live_retained_bytes()
            < physical_before_prepare.live_retained_bytes()
    );
    assert!(
        physical_after_prepare.live_retained_allocations()
            < physical_before_prepare.live_retained_allocations()
    );

    let published = prepared.handoff();
    assert!(account.is_same_account(published.fallback.account()));
    assert_eq!(published.fallback.value().code(), 91);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(store_failure_cause(&published.ledger_evidence).code(), 1);
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .expect("failure rejection ledger evidence")
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn downgrade_revalidation_panic_exposes_only_minimal_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(7_u8)),
        )
        .unwrap();
    assembly.try_commit_canonical(keys.dimension, 9).unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some() && session.outcome_evidence_records(e).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("success candidate must validate before downgrade"),
    };
    let failure = validated.downgrade_to_rich_failure(82);
    assert_eq!(failure.evidence_root(), evidence);
    let fallback_only = match failure.validate_rich_failure(evidence, |_session, _evidence| {
        panic!("contained downgrade validation panic")
    }) {
        StoreFailureValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("validation panic must leave only the pre-admitted fallback"),
    };
    let published = fallback_only.prepare_minimal_fallback().unwrap().handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert_eq!(store_failure_cause(&published.ledger_evidence).code(), 5);
    let dimension = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("fallback downgrade ledger evidence");
    assert_eq!(dimension.committed_canonical_output(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
