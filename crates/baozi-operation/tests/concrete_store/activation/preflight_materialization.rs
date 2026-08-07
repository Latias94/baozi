use super::*;

#[test]
fn verified_preflight_rejection_never_opens_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let after_start = account.snapshot();
    let called = Cell::new(false);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| {
                assert_eq!(account.snapshot(), after_start);
                preflight.reject(PreflightRejection::Unsupported, 11)
            },
            |_lane| -> CanonicalMaterialization<
                '_,
                (),
                PreflightRejection,
                ScalarSequence<u8>,
                SequenceTag,
            > {
                called.set(true);
                unreachable!("verified rejection cannot expose a canonical lane")
            },
        )
        .unwrap();
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("verified preflight rejection must remain NeverActivated")
        }
    };
    assert!(!called.get());
    assert_eq!(
        never_activated.cause(),
        &StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );

    let (prepared, cause) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    assert_eq!(
        cause,
        StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );
    let published = prepared.handoff();
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
fn dropping_an_unconsumed_session_outcome_owner_fails_closed() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    drop(operation.into_session_outcome_owner(evidence_plan(keys, 1)));

    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(
        snapshot.first_poison_evidence().unwrap().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
}

#[test]
fn clean_preflight_panic_prepares_minimal_fallback_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |_preflight| -> CanonicalPreflightDecision<()> {
                panic!("recoverable preflight callback panic")
            },
            |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                unreachable!("preflight panic must skip materialization")
            },
        )
        .unwrap();
    let fallback = match outcome {
        CanonicalActivationOutcome::Fallback(fallback) => fallback,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_) => {
            panic!("a clean preflight panic must publish minimal fallback")
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
fn later_preflight_contract_fault_contains_an_earlier_control_fault() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    cancellation.cancel();

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |mut preflight| {
                {
                    let mut checkpoint = preflight.control_checkpoint();
                    let first = checkpoint
                        .poll()
                        .expect_err("cancellation must be observed");
                    let later = checkpoint
                        .charge_observed(foreign_dimension(), 1)
                        .expect_err("foreign keys must remain a contract fault after cancellation");
                    assert_eq!(later, first);
                }
                preflight.reject(PreflightRejection::Unsupported, 11)
            },
            |_lane| -> CanonicalMaterialization<
                '_,
                (),
                PreflightRejection,
                ScalarSequence<u8>,
                SequenceTag,
            > { unreachable!("preflight rejection cannot materialize") },
        );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn foreign_preflight_envelope_is_contained_instead_of_reported() {
    let factory = factory(2);
    let account_a = account(&factory);
    let account_b = account(&factory);
    let (operation_a, keys_a) = operation(&account_a);
    let (operation_b, keys_b) = operation(&account_b);
    let called = Cell::new(false);

    let result = operation_a
        .into_session_outcome_owner(evidence_plan(keys_a, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys_b, 1)),
            |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                called.set(true);
                unreachable!("foreign envelope cannot expose a canonical lane")
            },
        );
    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("foreign envelope must be contained"),
    };
    assert!(!called.get());
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account_a.snapshot().poisoned());

    let published_b = operation_b.prepare_minimal_fallback(22).unwrap().handoff();
    drop(published_b);
}

#[test]
fn rollback_safe_contract_open_error_is_contained_before_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let materialization_called = Cell::new(false);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, TEST_MAX_FAILURE_EVIDENCE_MEMBERS + 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                materialization_called.set(true);
                unreachable!("a contract-invalid publication plan cannot expose a canonical lane")
            },
        );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a contract-invalid publication plan must fail closed"),
    };
    assert!(!materialization_called.get());
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn uncalled_materialization_callback_drop_panic_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.reject(PreflightRejection::Unsupported, 11),
            move |_lane| -> CanonicalMaterialization<
                '_,
                (),
                PreflightRejection,
                ScalarSequence<u8>,
                SequenceTag,
            > {
                let _capture = &capture;
                unreachable!("preflight rejection cannot invoke materialization")
            },
        );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("an uncalled callback drop panic must not escape as a clean rejection"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn preflight_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            move |_preflight| -> CanonicalPreflightDecision<()> {
                let _capture = &capture;
                panic!("contained preflight callback panic")
            },
            |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                unreachable!("preflight panic must skip materialization")
            },
        );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn materialization_callback_capture_is_destroyed_before_the_activated_outcome_escapes() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let dropped = Cell::new(false);
    let capture = DropProbe(&dropped);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            move |mut lane| {
                let _capture = &capture;
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(7)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<_, (), _, _>(root, root)
            },
        )
        .unwrap();
    assert!(dropped.get());
    drop(outcome);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn materialization_callback_drop_panic_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            move |mut lane| {
                let _capture = &capture;
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(7)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<_, (), _, _>(root, root)
            },
        );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a panicking callback drop must not expose an activation outcome"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn materialization_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            move |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                let _capture = &capture;
                panic!("contained materialization callback panic")
            },
        );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn materialization_rejection_eagerly_cleans_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal(canonical_plan(keys, 1)),
            |mut lane| {
                let _ = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(9)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    41,
                )
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("post-activation semantic rejection must remain Rejected")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Requested(MaterializationRejection::Semantic)
    );
    assert_eq!(rejected.canonical_len(), 0);

    let evidence = rejected
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(1),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match evidence {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("valid failure evidence must remain rooted")
        }
    };
    let (prepared, cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
    assert_eq!(
        cause,
        StoreCanonicalRejection::Requested(MaterializationRejection::Semantic)
    );
    let published = prepared.handoff();
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn materialization_product_graph_fault_is_sticky_through_rejection() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal(canonical_plan(keys, 1)),
            |mut lane| {
                let _ = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(9)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                let cause = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(10)?;
                            Ok(())
                        },
                    )
                    .unwrap_err();
                assert!(matches!(
                    cause,
                    StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical
                    }
                ));
                assert!(matches!(
                    lane.control_checkpoint(),
                    Err(StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical
                    })
                ));
                lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    41,
                )
            },
        )
        .unwrap();

    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a handled materialization fault must remain an ordinary rejection")
        }
    };
    assert!(matches!(
        rejected.cause(),
        StoreCanonicalRejection::Runtime(StoreRuntimeError::SlotLimit {
            destination: StoreDestination::Canonical
        })
    ));
    assert_eq!(rejected.canonical_len(), 0);

    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn materialization_build_fault_preserves_its_exact_product_graph_code() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal(canonical_plan(keys, 1)),
            |mut lane| {
                let expected = StoreBuildError::domain(73).unwrap();
                let cause = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |_writer, _control| Err(expected.into()),
                    )
                    .unwrap_err();
                assert_eq!(cause, StoreRuntimeError::Build(expected));
                lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    41,
                )
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the handled build fault must remain an ordinary rejection")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::ProductGraph);
    assert_eq!(terminal.code(), 73);
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn sealed_activation_plan_rejects_a_foreign_materialization_dimension_before_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let materialize_called = Cell::new(false);
    let table = RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap();
    let activation_plan = CanonicalActivationPlan::new(
        table,
        1,
        foreign_dimension(),
        keys.dimension,
        keys.dimension,
        NonZeroU64::new(1).unwrap(),
    )
    .unwrap();
    let mut activation_plan = Some(activation_plan);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            move |preflight| {
                preflight.seal::<()>(
                    activation_plan
                        .take()
                        .expect("the sealed activation plan is consumed once"),
                )
            },
            |lane| {
                materialize_called.set(true);
                lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 1)
            },
        );

    assert!(result.is_err());
    assert!(!materialize_called.get());
    assert!(account.snapshot().poisoned());
}

#[test]
fn sealed_activation_plan_rejects_a_foreign_validation_dimension_before_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let materialize_called = Cell::new(false);
    let table = RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap();
    let activation_plan = CanonicalActivationPlan::new(
        table,
        1,
        keys.dimension,
        foreign_dimension(),
        keys.dimension,
        NonZeroU64::new(1).unwrap(),
    )
    .unwrap();
    let mut activation_plan = Some(activation_plan);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            move |preflight| {
                preflight.seal::<()>(
                    activation_plan
                        .take()
                        .expect("the sealed activation plan is consumed once"),
                )
            },
            |lane| {
                materialize_called.set(true);
                lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 1)
            },
        );

    assert!(result.is_err());
    assert!(!materialize_called.get());
    assert!(account.snapshot().poisoned());
}

#[test]
fn sealed_activation_plan_rejects_a_foreign_commitment_dimension_before_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let materialize_called = Cell::new(false);
    let table = RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap();
    let activation_plan = CanonicalActivationPlan::new(
        table,
        1,
        keys.dimension,
        keys.dimension,
        foreign_dimension(),
        NonZeroU64::new(1).unwrap(),
    )
    .unwrap();
    let mut activation_plan = Some(activation_plan);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            move |preflight| {
                preflight.seal::<()>(
                    activation_plan
                        .take()
                        .expect("the sealed activation plan is consumed once"),
                )
            },
            |lane| {
                materialize_called.set(true);
                lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 1)
            },
        );

    assert!(result.is_err());
    assert!(!materialize_called.get());
    assert!(account.snapshot().poisoned());
}

#[test]
fn activation_panic_returns_full_containment_and_quarantines_the_account() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                panic!("contained activation panic")
            },
        );
    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("activation panic must be contained"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
}
