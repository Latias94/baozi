use super::*;

#[test]
fn ignored_failure_evidence_fault_plus_complete_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let result = state.build_failure_evidence(|lane| {
        let root = lane
            .try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                1,
                EvidenceLeafTag,
                |writer, _control| writer.push(1),
            )
            .unwrap();
        let ignored = lane.try_retain_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            1,
            EvidenceLeafTag,
            |writer, _control| writer.push(2),
        );
        assert_slot_limit(ignored.map(|_| ()), StoreDestination::OutcomeEvidence);
        lane.complete(root)
    });
    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn failure_evidence_abort_without_a_fault_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let result = state.build_failure_evidence(|lane| lane.abort::<(), ()>());
    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn failure_evidence_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);
    let capture = PanicOnDrop;

    let result = state.build_failure_evidence(
        move |_lane| -> FailureEvidenceDecision<(), EvidenceLeafTag> {
            let _capture = &capture;
            panic!("contained failure-evidence callback panic")
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn failure_evidence_builder_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let result =
        state.build_failure_evidence(|lane| -> FailureEvidenceDecision<(), EvidenceLeafTag> {
            let capture = PanicOnDrop;
            let cause = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    move |_writer, _control| {
                        let _capture = &capture;
                        panic!("contained failure-evidence sequence builder panic")
                    },
                )
                .unwrap_err();
            assert!(matches!(cause, StoreRuntimeError::BuilderPanicked));
            lane.abort()
        });

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn trusted_failure_evidence_fault_plus_abort_prepares_minimal_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let outcome = state
        .build_failure_evidence(|lane| {
            let _ = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(1),
                )
                .unwrap();
            let ignored = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                1,
                EvidenceLeafTag,
                |writer, _control| writer.push(2),
            );
            assert_slot_limit(ignored.map(|_| ()), StoreDestination::OutcomeEvidence);
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => {
            panic!("trusted evidence fault plus Abort must select minimal fallback")
        }
    };
    assert_eq!(
        fallback.cause(),
        &StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::ProductGraph);
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn later_failure_evidence_contract_fault_contains_an_earlier_slot_fault() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let result = state.build_failure_evidence(|lane| {
        let _root = lane
            .try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                1,
                EvidenceLeafTag,
                |writer, _control| writer.push(1),
            )
            .unwrap();
        let first = lane.try_retain_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            1,
            EvidenceLeafTag,
            |writer, _control| writer.push(2),
        );
        assert_slot_limit(first.map(|_| ()), StoreDestination::OutcomeEvidence);
        let later = lane.commit(foreign_dimension(), 1);
        assert_slot_limit(later, StoreDestination::OutcomeEvidence);
        lane.abort::<(), ()>()
    });

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn exact_failure_work_budget_preserves_rich_failure_and_zero_canonical_output() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);

    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("valid evidence must retain a rich failure owner")
        }
    };
    let validation = rooted
        .validate_rich_failure(|session, root, control| {
            control
                .try_begin_chunk(TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT)
                .unwrap();
            session.sequence(root).and_then(|view| view.get(0)) == Some(7)
        })
        .unwrap();
    let validated = match validation {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("valid evidence must pass validation")
        }
    };
    let (prepared, cause) = validated
        .try_prepare(final_progress())
        .unwrap()
        .into_parts();
    assert_eq!(
        cause,
        StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );
    let prepared = match prepared {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated evidence must prepare rich failure")
        }
    };
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::Requested);
    assert_eq!(terminal.code(), 11);
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn one_short_failure_construction_budget_falls_back_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let outcome = state
        .build_failure_evidence(|lane| {
            let error = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT + 1,
                EvidenceLeafTag,
                |_writer, _control| unreachable!("the one-short build must be rejected first"),
            );
            assert!(matches!(error, Err(StoreRuntimeError::Build(_))));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => panic!("one-short construction must downgrade"),
    };
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(cause, StoreNeverActivatedCause::Requested(_)));
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn one_short_failure_validation_budget_falls_back_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => panic!("valid evidence must remain rooted"),
    };

    let validation = rooted
        .validate_rich_failure(|_session, _root, control| {
            assert!(matches!(
                control.try_begin_chunk(TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT + 1),
                Err(StoreRuntimeError::Build(_))
            ));
            false
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("one-short validation must downgrade")
        }
    };
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(cause, StoreRuntimeError::Build(_)));
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn final_control_fault_upgrades_requested_rich_failure_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (state, keys) = rejected_session_with_control(&account, control);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("valid evidence must retain a rich failure owner")
        }
    };
    let validation = rooted
        .validate_rich_failure(|session, root, control| {
            control.try_begin_chunk(1).unwrap();
            session.sequence(root).and_then(|view| view.get(0)) == Some(7)
        })
        .unwrap();
    let validated = match validation {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("valid evidence must pass validation")
        }
    };
    cancellation.cancel();

    let (prepared, _cause) = validated
        .try_prepare(final_progress())
        .unwrap()
        .into_parts();
    let prepared = match prepared {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated evidence must prepare rich failure")
        }
    };
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
}

#[test]
fn final_control_fault_upgrades_requested_minimal_fallback_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (state, _keys) = rejected_session_with_control(&account, control);
    cancellation.cancel();

    let (prepared, _cause) = state.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
}

#[test]
fn failure_construction_cancellation_uses_protected_fallback_budget() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (state, _keys) = rejected_session_with_control(&account, control);
    cancellation.cancel();

    let outcome = state
        .build_failure_evidence(|lane| {
            let result = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                1,
                EvidenceLeafTag,
                |_writer, _control| unreachable!("cancellation must be polled before construction"),
            );
            assert!(matches!(result, Err(StoreRuntimeError::Control(_))));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => panic!("cancellation must downgrade to fallback"),
    };
    assert!(!account.snapshot().poisoned());
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn rich_failure_validation_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let capture = PanicOnDrop;

    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("valid evidence must retain a rich failure owner")
        }
    };

    let result = rooted.validate_rich_failure(move |_session, _root, _control| {
        let _capture = &capture;
        panic!("contained rich-failure validation panic")
    });

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn clean_rich_failure_validation_panic_prepares_fallback_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => panic!("valid evidence must remain rooted"),
    };

    let validation = rooted
        .validate_rich_failure(|_session, _root, _control| {
            panic!("recoverable rich-failure validator panic")
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("a clean validator panic must select minimal fallback")
        }
    };
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(cause, StoreRuntimeError::Build(_)));
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn rich_failure_semantic_rejection_preserves_the_rejected_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    1,
                    EvidenceLeafTag,
                    |writer, _control| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => panic!("valid evidence must remain rooted"),
    };

    let validation = rooted
        .validate_rich_failure(|session, root, control| {
            control.try_begin_chunk(1).unwrap();
            assert_eq!(session.sequence(root).and_then(|view| view.get(0)), Some(7));
            false
        })
        .unwrap();
    let rejected = match validation {
        RootedFailureValidation::Rejected(rejected) => rejected,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Fallback(_) => {
            panic!("semantic rejection must retain the rejected failure owner")
        }
    };
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    assert_eq!(
        cause,
        StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
