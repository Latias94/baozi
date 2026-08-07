use super::*;

fn split_dimension_operation(account: &PhysicalStorageAccount) -> (TestOperation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let ordinary_attempted = failure_terminal_work.ordinary_attempted_work_limit();
    let ordinary_consumed = failure_terminal_work.ordinary_consumed_work_limit();
    let first_attempted = ordinary_attempted / 2;
    let first_consumed = ordinary_consumed / 2;
    let first_policy =
        DimensionPolicy::reservable_bytes(first_attempted, first_consumed, 100, u64::MAX)
            .with_published_outcome_evidence_limit(100);
    let second_policy = DimensionPolicy::reservable_bytes(
        ordinary_attempted - first_attempted,
        ordinary_consumed - first_consumed,
        100,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(100);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, first_policy),
            (keys.other_dimension, second_policy),
            (
                keys.failure_terminal_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )
    .unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, TEST_TERMINAL_ATTRIBUTION_SLOTS, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
        failure_terminal_work,
    );
    (
        TestOperation::try_start(OperationControl::default(), ledger_plan, account, start).unwrap(),
        keys,
    )
}

fn centrally_rejected_sequence_work(elements: usize) -> (u64, u64) {
    let values = (0..elements)
        .map(|value| u8::try_from(value).expect("the test sequence fits in u8"))
        .collect::<Vec<_>>();
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(elements).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&values),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, _control| {
                Ok(CanonicalValidationDecision::<(), _>::reject(
                    ValidationRejection::Semantic,
                    73,
                ))
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("semantic rejection must retain the evidence owner")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let dimension = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("ordinary work evidence remains queryable");
    let work = (dimension.attempted(), dimension.consumed());
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure work remains queryable");
    assert_eq!(
        (protected.attempted(), protected.consumed()),
        (0, 0),
        "success-path evidence construction must not consume protected work"
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    work
}

#[test]
fn matching_activation_can_only_publish_through_central_validation() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("matching preflight must activate the canonical lane")
        }
    };
    let expected_output = *activated.output();
    let validated = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |session, canonical, evidence, _output, control| {
                control.poll()?;
                let canonical_value = session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0));
                let evidence_value = session
                    .outcome_evidence_sequence(evidence)
                    .and_then(|view| view.get(0));
                if canonical_value == Some(7) && evidence_value == Some(9) {
                    Ok(CanonicalValidationDecision::accept("validated"))
                } else {
                    Ok(CanonicalValidationDecision::reject(
                        ValidationRejection::Semantic,
                        71,
                    ))
                }
            },
        )
        .unwrap();
    let candidate = match validated {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("valid canonical storage must remain a success candidate")
        }
    };
    assert_eq!(candidate.proof(), &"validated");
    assert_eq!(candidate.output(), &expected_output);
    drop(candidate);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_validation_uses_the_root_sealed_during_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 2)),
            |mut lane| {
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
                let other_root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(8)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete(root, other_root)
            },
        )
        .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };

    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |session, canonical, evidence, output, control| {
                control.poll()?;
                let first_value = session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0));
                let second_value = session
                    .canonical_sequence(*output)
                    .and_then(|view| view.get(0));
                let evidence_value = session
                    .outcome_evidence_sequence(evidence)
                    .and_then(|view| view.get(0));
                if first_value == Some(7) && second_value == Some(8) && evidence_value == Some(9) {
                    Ok(CanonicalValidationDecision::accept("sealed-root"))
                } else {
                    Ok(CanonicalValidationDecision::reject(
                        ValidationRejection::Semantic,
                        72,
                    ))
                }
            },
        )
        .unwrap();
    let candidate = match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the sealed first root must reach central validation")
        }
    };
    assert_eq!(candidate.proof(), &"sealed-root");
    drop(candidate);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_outcome_evidence_sequence_charges_exact_completed_work() {
    let baseline = centrally_rejected_sequence_work(1);
    let measured = centrally_rejected_sequence_work(4);

    assert_eq!(measured.0.checked_sub(baseline.0), Some(3));
    assert_eq!(measured.1.checked_sub(baseline.1), Some(3));
}

#[test]
fn activation_plan_keeps_validation_work_on_its_semantic_dimension() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = split_dimension_operation(&account);
    let table = RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap();
    let activation_plan = CanonicalActivationPlan::new(
        table,
        1,
        keys.dimension,
        keys.other_dimension,
        keys.dimension,
        NonZeroU64::new(37).unwrap(),
    )
    .unwrap();
    let mut activation_plan = Some(activation_plan);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            move |preflight| {
                preflight.seal::<()>(
                    activation_plan
                        .take()
                        .expect("the sealed activation plan is consumed once"),
                )
            },
            |mut lane| {
                lane.control_checkpoint()
                    .unwrap()
                    .charge_observed(5)
                    .unwrap();
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(3).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&[1, 2, 3]),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, _control| {
                Ok(CanonicalValidationDecision::<(), _>::reject(
                    ValidationRejection::Semantic,
                    73,
                ))
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("semantic rejection must retain the evidence owner")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let materialization = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("materialization dimension remains queryable");
    let validation_work = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("validation-work dimension remains queryable");

    assert_eq!(materialization.attempted(), materialization.consumed());
    assert!(materialization.attempted() > validation_work.attempted());
    assert_eq!(
        (validation_work.attempted(), validation_work.consumed()),
        (3, 3)
    );
    assert_eq!(materialization.committed_canonical_output(), 0);
    assert_eq!(validation_work.committed_canonical_output(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_outcome_evidence_cancellation_preserves_the_completed_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome =
        activated
            .validate_centrally(
                |lane, _output| {
                    let evidence = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(2).unwrap(),
                        EvidenceLeafTag,
                        |writer| {
                            writer.push(9)?;
                            cancellation.cancel();
                            writer.push(10)
                        },
                    );
                    assert!(matches!(evidence, Err(StoreRuntimeError::Control(_))));
                    lane.abort::<EvidenceLeafTag, EvidenceLeafTag>()
                },
                |_session,
                 _canonical,
                 _evidence,
                 _output,
                 _control|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > {
                    unreachable!("cancelled evidence construction must skip central validation")
                },
            )
            .unwrap();
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("evidence construction cancellation must select minimal fallback")
        }
    };
    assert!(matches!(fallback.cause(), StoreRuntimeError::Control(_)));
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let ordinary = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("ordinary work evidence remains queryable");
    assert_eq!(ordinary.attempted() - ordinary.consumed(), 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_outcome_evidence_underwrite_preserves_the_completed_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome =
        activated
            .validate_centrally(
                |lane, _output| {
                    let evidence = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(2).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    );
                    assert!(matches!(evidence, Err(StoreRuntimeError::Build(_))));
                    lane.abort::<(), EvidenceLeafTag>()
                },
                |_session,
                 _canonical,
                 _evidence,
                 _output,
                 _control|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > {
                    unreachable!("incomplete evidence must skip central validation")
                },
            )
            .unwrap();
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("incomplete evidence must select minimal fallback")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let ordinary = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("ordinary work evidence remains queryable");
    assert_eq!(ordinary.attempted() - ordinary.consumed(), 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn foreign_canonical_root_is_contained_during_materialization_completion() {
    let factory = factory(2);
    let account_a = account(&factory);
    let account_b = account(&factory);
    let (operation_a, keys_a) = operation(&account_a);
    let (operation_b, keys_b) = operation(&account_b);

    let outcome_b = operation_b
        .into_session_outcome_owner(evidence_plan(keys_b, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys_b, 1)),
            |mut lane| {
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(2)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<_, (), _, _>(root, root)
            },
        )
        .unwrap();
    let activated_b = match outcome_b {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("second operation must activate")
        }
    };
    let foreign_root = *activated_b.output();

    let result = operation_a
        .into_session_outcome_owner(evidence_plan(keys_a, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys_a, 1)),
            |mut lane| {
                let own_root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(1)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<_, (), _, _>(foreign_root, own_root)
            },
        );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a foreign canonical root must be contained"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account_a.snapshot().poisoned());
    assert!(!account_b.snapshot().poisoned());
    drop(activated_b);
    assert_eq!(account_b.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_preserves_unused_table_capacity() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan_with_capacity(keys, 2, 1)),
            |mut lane| {
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

    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("unused table capacity must remain a legal activation")
        }
    };
    assert_eq!(activated.canonical_len(), 1);
    drop(activated);
}

#[test]
fn sealed_activation_rejects_materialization_beyond_the_exact_shape() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan_with_capacity(keys, 2, 1)),
            |mut lane| {
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
                let _extra = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(8)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<_, (), _, _>(root, root)
            },
        );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("materialization beyond the exact shape must not activate"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn sealed_activation_rejects_an_incomplete_exact_shape() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let result = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 2)),
            |mut lane| {
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
        Ok(_) => panic!("a partial canonical table must not activate"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn clean_central_validation_panic_prepares_minimal_fallback_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };

    let outcome =
        activated
            .validate_centrally(
                |lane, _output| {
                    let evidence = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            EvidenceLeafTag,
                            |writer| writer.push(9),
                        )
                        .unwrap();
                    lane.commit(keys.dimension, 1).unwrap();
                    lane.complete(evidence)
                },
                |_session,
                 _canonical,
                 _evidence,
                 _output,
                 _control|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > { panic!("recoverable central validator panic") },
            )
            .unwrap();
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
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
fn completed_central_validation_capture_drop_panic_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let validation_capture = PanicOnDrop;

    let result = activated.validate_centrally(
        |lane, _output| {
            let evidence = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(9),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(evidence)
        },
        move |_session, _canonical, _evidence, _output, _control| {
            let _capture = &validation_capture;
            Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
        },
    );

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a completed validator capture drop panic must not return a candidate"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn central_evidence_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let evidence_capture = PanicOnDrop;

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central validation")
        }
    };

    let result = activated.validate_centrally(
        move |_lane, _output| -> OutcomeEvidenceDecision<(), EvidenceLeafTag> {
            let _capture = &evidence_capture;
            panic!("contained evidence callback panic")
        },
        |_session,
         _canonical,
         _evidence,
         _output,
         _control|
         -> Result<CanonicalValidationDecision<(), ()>, StoreWorkError> {
            unreachable!("evidence callback panic must skip central validation")
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn central_evidence_builder_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central validation")
        }
    };

    let result = activated.validate_centrally(
        |lane, _output| -> OutcomeEvidenceDecision<(), EvidenceLeafTag> {
            let capture = PanicOnDrop;
            let cause = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    move |_writer| {
                        let _capture = &capture;
                        panic!("contained outcome-evidence sequence builder panic")
                    },
                )
                .unwrap_err();
            assert!(matches!(cause, StoreRuntimeError::BuilderPanicked));
            let _ = cause;
            lane.abort()
        },
        |_session,
         _canonical,
         _evidence,
         _output,
         _control|
         -> Result<CanonicalValidationDecision<(), ()>, StoreWorkError> {
            unreachable!("contract evidence fault must skip central validation")
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn activation_only_records_commitment_intent_without_running_the_joint_commit() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_commit_limit(&account, 0);
    let evidence_called = Cell::new(false);
    let validation_called = Cell::new(false);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central commitment")
        }
    };
    let result = activated
        .validate_centrally(
            |lane, _output| {
                evidence_called.set(true);
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |session, canonical, evidence, _output, _control| {
                validation_called.set(true);
                let canonical_value = session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0));
                let evidence_value = session
                    .outcome_evidence_sequence(evidence)
                    .and_then(|view| view.get(0));
                assert_eq!(canonical_value, Some(7));
                assert_eq!(evidence_value, Some(1));
                Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
            },
        )
        .unwrap();
    let candidate = match result {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("activation must not run the later joint-commit admission")
        }
    };
    assert!(evidence_called.get());
    assert!(validation_called.get());
    drop(candidate);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn trusted_evidence_slot_limit_returns_product_graph_fallback_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let validation_called = Cell::new(false);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central evidence construction")
        }
    };

    let result = activated
        .validate_centrally(
            |lane, _output| {
                let _root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                let cause = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(2),
                    )
                    .unwrap_err();
                assert!(matches!(
                    cause,
                    StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::OutcomeEvidence
                    }
                ));
                let _ = cause;
                lane.abort::<(), EvidenceLeafTag>()
            },
            |_session, _canonical, _evidence, _output, _control| {
                validation_called.set(true);
                Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
            },
        )
        .unwrap();
    let fallback = match result {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("trusted evidence construction failure must select minimal fallback")
        }
    };
    assert!(!validation_called.get());
    assert!(matches!(
        fallback.cause(),
        StoreRuntimeError::SlotLimit {
            destination: StoreDestination::OutcomeEvidence
        }
    ));
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn central_validation_rejection_can_validate_existing_rich_failure_evidence() {
    const TEXT: &str = "\u{5305}\u{5b50}";
    const EXPECTED_VALIDATION_WORK: u64 = 3 + 4 + 6 + 2 * 4;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 4))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let sequence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(3).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&[7, 8, 9]),
                    )
                    .unwrap();
                let bytes = lane
                    .try_retain_bytes(BytesPlan::new(4).unwrap(), EvidenceBytesTag, |writer| {
                        writer.write_bytes(&[10, 11, 12, 13])
                    })
                    .unwrap();
                let text = lane
                    .try_retain_utf8(
                        Utf8Plan::new(TEXT.len()).unwrap(),
                        EvidenceTextTag,
                        |writer| writer.write_str(TEXT),
                    )
                    .unwrap();
                let evidence = lane
                    .try_retain_records(RecordPlan::new(2, 4).unwrap(), EvidenceRootTag, |writer| {
                        writer.write_row(|row| {
                            row.write_sequence_ref(sequence)?;
                            row.write_bytes_ref(bytes)?;
                            row.write_utf8_ref(text)?;
                            row.write_scalar(42_u8)
                        })?;
                        writer.write_row(|row| {
                            row.write_scalar(1_u8)?;
                            row.write_scalar(2_u8)?;
                            row.write_scalar(3_u8)?;
                            row.write_scalar(4_u8)
                        })
                    })
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, _control| {
                Ok(CanonicalValidationDecision::<(), _>::reject(
                    ValidationRejection::Semantic,
                    72,
                ))
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("semantic rejection must retain rooted rich evidence")
        }
    };
    let validation_called = Cell::new(false);
    let validation = rejected
        .validate_rich_failure(|session, root| {
            validation_called.set(true);
            let mut sequence = None;
            let mut bytes = None;
            let mut text = None;
            let root_valid = session
                .visit_records(root, |row_index, row| match row_index {
                    0 => {
                        let sequence_valid = row
                            .sequence_ref::<u8, EvidenceLeafTag, _>(0, |value| {
                                sequence = Some(value);
                                true
                            })
                            .unwrap();
                        let bytes_valid = row
                            .bytes_ref::<EvidenceBytesTag, _>(1, |value| {
                                bytes = Some(value);
                                true
                            })
                            .unwrap();
                        let text_valid = row
                            .utf8_ref::<EvidenceTextTag, _>(2, |value| {
                                text = Some(value);
                                true
                            })
                            .unwrap();
                        let scalar_valid = row.scalar::<u8, _>(3, |value| value == 42).unwrap();
                        sequence_valid && bytes_valid && text_valid && scalar_valid
                    }
                    1 => {
                        row.scalar::<u8, _>(0, |value| value == 1).unwrap()
                            && row.scalar::<u8, _>(1, |value| value == 2).unwrap()
                            && row.scalar::<u8, _>(2, |value| value == 3).unwrap()
                            && row.scalar::<u8, _>(3, |value| value == 4).unwrap()
                    }
                    _ => false,
                })
                .unwrap();
            root_valid
                && session
                    .visit_sequence(
                        sequence.expect("record sequence reference"),
                        |index, value| value == [7, 8, 9][index],
                    )
                    .unwrap()
                && session
                    .visit_bytes(bytes.expect("record bytes reference"), |index, value| {
                        value == [10, 11, 12, 13][index]
                    })
                    .unwrap()
                && session
                    .visit_utf8_bytes(text.expect("record UTF-8 reference"), |index, value| {
                        value == TEXT.as_bytes()[index]
                    })
                    .unwrap()
        })
        .unwrap();
    let validated = match validation {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("valid central rejection evidence must remain rich")
        }
    };
    assert!(validation_called.get());
    assert_eq!(
        validated.cause(),
        &StoreCanonicalRejection::Requested(ValidationRejection::Semantic)
    );
    let (prepared, _cause) = validated.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(protected.attempted(), EXPECTED_VALIDATION_WORK);
    assert_eq!(protected.consumed(), EXPECTED_VALIDATION_WORK);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_evidence_validation_envelope_rejects_before_the_overflowing_builder() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_failure_work_limits(&account, 1, 1);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 2))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let overflowing_builder_called = Cell::new(false);
    let outcome =
        activated
            .validate_centrally(
                |lane, _output| {
                    let _first = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            EvidenceLeafTag,
                            |writer| writer.push(9),
                        )
                        .unwrap();
                    let overflow = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| {
                            overflowing_builder_called.set(true);
                            writer.push(10)
                        },
                    );
                    assert!(matches!(overflow, Err(StoreRuntimeError::Build(_))));
                    lane.abort::<(), EvidenceLeafTag>()
                },
                |_session,
                 _canonical,
                 _evidence,
                 _output,
                 _control|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > {
                    unreachable!("an envelope rejection must skip central validation")
                },
            )
            .unwrap();
    assert!(!overflowing_builder_called.get());
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the exhausted validation envelope must select minimal fallback")
        }
    };
    assert!(matches!(fallback.cause(), StoreRuntimeError::Build(_)));
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(protected.attempted(), 0);
    assert_eq!(protected.consumed(), 0);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_control_interruption_can_validate_existing_rich_failure_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, control| {
                cancellation.cancel();
                control.poll()?;
                Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
            },
        )
        .unwrap();
    let interrupted = match outcome {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("central cancellation must retain rooted rich evidence")
        }
    };
    assert!(interrupted.cause().control_cause().is_some());
    let validation_called = Cell::new(false);
    let validation = interrupted
        .validate_rich_failure(|session, root| {
            validation_called.set(true);
            session
                .visit_sequence(root, |index, value| index == 0 && value == 9)
                .unwrap()
        })
        .unwrap();
    let validated = match validation {
        RootedFailureValidation::Valid(validated) => validated,
        RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
            panic!("valid control-interruption evidence must remain rich")
        }
    };
    assert!(validation_called.get());
    assert!(validated.cause().control_cause().is_some());
    let (prepared, _cause) = validated.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_ledger_interruption_can_reject_existing_rich_failure_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, control| {
                assert!(control.charge_observed(u64::MAX).is_err());
                Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
            },
        )
        .unwrap();
    let interrupted = match outcome {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("central ledger rejection must retain rooted rich evidence")
        }
    };
    assert!(interrupted.cause().ledger_cause().is_some());
    let validation_called = Cell::new(false);
    let validation = interrupted
        .validate_rich_failure(|session, root| {
            validation_called.set(true);
            assert!(
                session
                    .visit_sequence(root, |index, value| index == 0 && value == 9)
                    .unwrap()
            );
            false
        })
        .unwrap();
    let rejected = match validation {
        RootedFailureValidation::Rejected(rejected) => rejected,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Fallback(_) => {
            panic!("the rich validator's semantic rejection must remain explicit")
        }
    };
    assert!(validation_called.get());
    assert!(rejected.cause().ledger_cause().is_some());
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn outcome_evidence_abort_uses_the_first_fault_without_caller_echo() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central evidence construction")
        }
    };

    let result =
        activated
            .validate_centrally(
                |lane, _output| {
                    let _root = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            EvidenceLeafTag,
                            |writer| writer.push(1),
                        )
                        .unwrap();
                    let first = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            EvidenceLeafTag,
                            |writer| writer.push(2),
                        )
                        .unwrap_err();
                    assert!(matches!(
                        first,
                        StoreRuntimeError::SlotLimit {
                            destination: StoreDestination::OutcomeEvidence
                        }
                    ));
                    assert!(matches!(
                        lane.commit(keys.dimension, 2),
                        Err(StoreRuntimeError::SlotLimit {
                            destination: StoreDestination::OutcomeEvidence
                        })
                    ));
                    lane.abort::<(), EvidenceLeafTag>()
                },
                |_session,
                 _canonical,
                 _evidence,
                 _output,
                 _control|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > { unreachable!("aborted evidence must skip central validation") },
            )
            .unwrap();
    let fallback = match result {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the first recoverable evidence fault must select minimal fallback")
        }
    };
    assert!(matches!(
        fallback.cause(),
        StoreRuntimeError::SlotLimit {
            destination: StoreDestination::OutcomeEvidence
        }
    ));
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn later_outcome_evidence_contract_fault_contains_an_earlier_slot_fault() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central evidence construction")
        }
    };

    let result = activated.validate_centrally(
        |lane, _output| {
            let _root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(1),
                )
                .unwrap();
            let first = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                EvidenceLeafTag,
                |writer| writer.push(2),
            );
            assert_slot_limit(first.map(|_| ()), StoreDestination::OutcomeEvidence);
            let later = lane.commit(foreign_dimension(), 1);
            assert_slot_limit(later, StoreDestination::OutcomeEvidence);
            lane.abort::<(), EvidenceLeafTag>()
        },
        |_session,
         _canonical,
         _evidence,
         _output,
         _control|
         -> Result<CanonicalValidationDecision<(), ValidationRejection>, StoreWorkError> {
            unreachable!("aborted evidence must skip central validation")
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn sticky_validation_work_failure_overrides_a_later_accept_decision() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must reach central validation")
        }
    };

    let result = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, control| {
                assert!(control.charge_observed(u64::MAX).is_err());
                Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
            },
        )
        .unwrap();
    let interrupted = match result {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("the sticky ledger failure must override the later semantic decision")
        }
    };
    assert!(interrupted.cause().ledger_cause().is_some());
    let (prepared, _cause) = interrupted.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
}

#[test]
fn central_validation_rejection_eagerly_cleans_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 1)),
            |mut lane| {
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
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization must activate")
        }
    };
    let outcome = activated
        .validate_centrally(
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(evidence)
            },
            |_session, _canonical, _evidence, _output, _control| {
                Ok(CanonicalValidationDecision::<(), _>::reject(
                    ValidationRejection::Semantic,
                    72,
                ))
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("semantic rejection must return a rooted rejected owner")
        }
    };
    assert_eq!(rejected.canonical_len(), 0);
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Requested(ValidationRejection::Semantic)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
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
}
