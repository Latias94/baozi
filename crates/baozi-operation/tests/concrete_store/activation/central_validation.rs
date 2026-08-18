use super::*;

fn inspect_one_byte_pair<'session, 'storage, 'control, 'operation>(
    session: &mut CanonicalValidationSession<
        'session,
        'storage,
        'control,
        'operation,
        TestOwner,
        TestProgress,
        TestUnit,
    >,
    canonical: CanonicalValidationMember<'session, ScalarSequence<u8>, SequenceTag>,
    evidence: OutcomeEvidenceValidationMember<'session, ScalarSequence<u8>, EvidenceLeafTag>,
    expected_canonical: u8,
    expected_evidence: u8,
) -> Result<bool, StoreWorkError> {
    let mut canonical_value = None;
    let canonical_valid = session.visit_canonical_sequence(canonical, 1, |_index, value| {
        canonical_value = Some(value);
        true
    })?;
    let mut evidence_value = None;
    let evidence_valid =
        session.visit_outcome_evidence_sequence(evidence, 1, |_index, value| {
            evidence_value = Some(value);
            true
        })?;
    Ok(canonical_valid
        && evidence_valid
        && canonical_value == Some(expected_canonical)
        && evidence_value == Some(expected_evidence))
}

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

fn validation_completion_limited_operation(
    account: &PhysicalStorageAccount,
) -> (TestOperation, TestKeys) {
    const VALIDATION_ATTEMPTED_LIMIT: u64 = 2;
    const VALIDATION_CONSUMED_LIMIT: u64 = 1;

    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.failure_terminal_dimension,
        TEST_MAX_FAILURE_EVIDENCE_MEMBERS,
        TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT,
        TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT,
        u64::MAX,
        u64::MAX - 1,
    )
    .unwrap();
    let other_policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work
            .ordinary_attempted_work_limit()
            .checked_sub(VALIDATION_ATTEMPTED_LIMIT)
            .expect("validation work fits the ordinary attempted budget"),
        failure_terminal_work
            .ordinary_consumed_work_limit()
            .checked_sub(VALIDATION_CONSUMED_LIMIT)
            .expect("validation work fits the ordinary consumed budget"),
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let validation_policy = DimensionPolicy::reservable_bytes(
        VALIDATION_ATTEMPTED_LIMIT,
        VALIDATION_CONSUMED_LIMIT,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, other_policy),
            (keys.other_dimension, validation_policy),
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
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_terminal_work,
    );
    (
        TestOperation::try_start(OperationControl::default(), ledger_plan, account, start).unwrap(),
        keys,
    )
}

fn scoped_activation_shape_with_validation_work(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
    validation_work: NonZeroU64,
) -> TestScopedActivation {
    let TestScopedActivation {
        policy, failure, ..
    } = scoped_activation_shape(keys, canonical_blocks, evidence_blocks);
    TestScopedActivation {
        policy,
        failure,
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(canonical_blocks, canonical_blocks).unwrap(),
            canonical_blocks,
            validation_work,
        )
        .unwrap(),
    }
}

fn scoped_activation_shape_with_capacity(
    canonical_keys: TestKeys,
    canonical_capacity: usize,
    expected_materialized_blocks: usize,
    evidence_keys: TestKeys,
    evidence_blocks: usize,
) -> TestScopedActivation {
    TestScopedActivation {
        policy: CanonicalActivationPolicy::new(
            writer_plan(canonical_keys, canonical_capacity + 1, 2),
            writer_plan(evidence_keys, evidence_blocks + 1, 2),
            CanonicalMaterializationRole::new(canonical_keys.dimension, canonical_keys.dimension),
            CanonicalValidationRole::new(
                canonical_keys.phase,
                canonical_keys.dimension,
                canonical_keys.dimension,
            ),
            canonical_keys.dimension,
            evidence_keys.dimension,
        ),
        failure: CanonicalFailureRequirements::new(
            CanonicalTableShape::new(evidence_blocks, evidence_blocks).unwrap(),
        ),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(canonical_capacity, canonical_capacity).unwrap(),
            expected_materialized_blocks,
            NonZeroU64::MIN,
        )
        .unwrap(),
    }
}

fn centrally_rejected_sequence_work(elements: usize) -> (u64, u64) {
    let values = (0..elements)
        .map(|value| u8::try_from(value).expect("the test sequence fits in u8"))
        .collect::<Vec<_>>();
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(elements).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&values),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            73,
                        ))
                    },
                )
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

const CANONICAL_VALIDATION_STACK_BUDGET: usize = 2 * 1024 * 1024;

fn run_with_canonical_validation_stack_budget(test: fn()) {
    let result = std::thread::Builder::new()
        .name("canonical-validation-stack-budget".into())
        .stack_size(CANONICAL_VALIDATION_STACK_BUDGET)
        .spawn(test)
        .expect("the canonical validation stack-budget thread must start")
        .join();
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn matching_activation_can_only_publish_through_central_validation() {
    run_with_canonical_validation_stack_budget(
        matching_activation_can_only_publish_through_central_validation_body,
    );
}

fn matching_activation_can_only_publish_through_central_validation_body() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        if inspect_one_byte_pair(session, canonical, evidence, 7, 9)? {
                            Ok(CanonicalValidationDecision::accept("validated"))
                        } else {
                            Ok(
                                CanonicalValidationDecision::<&str, ValidationRejection>::reject(
                                    ValidationRejection::Semantic,
                                    71,
                                ),
                            )
                        }
                    },
                )
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
fn canonical_shape_contract_cannot_be_overwritten_by_requested_rejection() {
    run_with_canonical_validation_stack_budget(
        canonical_shape_contract_cannot_be_overwritten_by_requested_rejection_body,
    );
}

fn canonical_shape_contract_cannot_be_overwritten_by_requested_rejection_body() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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

    let result = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(inspect_one_byte_pair(session, canonical, evidence, 7, 9)?);
                        assert!(!session.visit_canonical_sequence(
                            canonical,
                            2,
                            |_index, _value| true
                        )?);
                        Ok(
                            CanonicalValidationDecision::<(), ValidationRejection>::reject(
                                ValidationRejection::Semantic,
                                72,
                            ),
                        )
                    },
                )
            },
        )
        .unwrap();

    let rejected = match result {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a canonical shape contract fault must remain a rich rejection")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Runtime(StoreRuntimeError::CanonicalValidationContractRejected,)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_contract_precedes_later_control_interruption() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session,
                     canonical,
                     evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        assert!(!session.visit_canonical_sequence(
                            canonical,
                            2,
                            |_index, _value| true,
                        )?);
                        cancellation.cancel();
                        let _ = session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, _value| true,
                        )?;
                        unreachable!("the cancelled visit must interrupt validation")
                    },
                )
            },
        )
        .unwrap();

    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the first ProductGraph cause must survive later cancellation")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Runtime(StoreRuntimeError::CanonicalValidationContractRejected,)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn control_interruption_precedes_later_canonical_contract_failure() {
    run_with_canonical_validation_stack_budget(
        control_interruption_precedes_later_canonical_contract_failure_body,
    );
}

fn control_interruption_precedes_later_canonical_contract_failure_body() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        cancellation.cancel();
                        let control_cause = session
                            .visit_canonical_sequence(canonical, 1, |_index, _value| true)
                            .unwrap_err();
                        assert!(!session.visit_outcome_evidence_sequence(
                            evidence,
                            2,
                            |_index, _value| true,
                        )?);
                        Err::<CanonicalValidationDecision<(), ValidationRejection>, _>(
                            control_cause,
                        )
                    },
                )
            },
        )
        .unwrap();

    let interrupted = match outcome {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("the first Control cause must survive a later contract fault")
        }
    };
    assert!(interrupted.cause().control_cause().is_some());
    let (prepared, _cause) = interrupted.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn completed_member_repeat_still_polls_control_without_recharging() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
    let repeated_inspector_called = Cell::new(false);

    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(inspect_one_byte_pair(session, canonical, evidence, 7, 9)?);
                        cancellation.cancel();
                        let cause = session
                            .visit_canonical_sequence(canonical, 1, |_index, _value| {
                                repeated_inspector_called.set(true);
                                true
                            })
                            .unwrap_err();
                        assert!(cause.control_cause().is_some());
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            74,
                        ))
                    },
                )
            },
        )
        .unwrap();

    let interrupted = match outcome {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("a completed member repeat must observe cancellation")
        }
    };
    assert!(!repeated_inspector_called.get());
    assert!(interrupted.cause().control_cause().is_some());
    let (prepared, _cause) = interrupted.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_validation_cancellation_is_bounded_and_preserves_the_completed_prefix() {
    const CANONICAL_UNITS: usize = 257;
    const EVIDENCE_UNITS: usize = 1;
    const COMPLETED_PREFIX: usize = 256;

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let values = vec![7; CANONICAL_UNITS];

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(
            keys,
            1,
            1,
            NonZeroU64::new(u64::try_from(CANONICAL_UNITS + EVIDENCE_UNITS).unwrap()).unwrap(),
        ),
        |mut lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(CANONICAL_UNITS).unwrap(),
                    SequenceTag,
                    |writer| writer.extend_from_slice(&values),
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
    let inspected = Cell::new(0usize);

    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(EVIDENCE_UNITS).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, _evidence, &mut ()| {
                        let cause = session
                            .visit_canonical_sequence(canonical, CANONICAL_UNITS, |index, value| {
                                assert_eq!(value, 7);
                                inspected.set(inspected.get() + 1);
                                if index == 0 {
                                    cancellation.cancel();
                                }
                                true
                            })
                            .expect_err("cancellation must be observed at the bounded checkpoint");
                        Err::<CanonicalValidationDecision<(), ValidationRejection>, _>(cause)
                    },
                )
            },
        )
        .unwrap();

    let interrupted = match outcome {
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => interrupted,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_) => {
            panic!("bounded canonical validation must preserve the Control interruption")
        }
    };
    assert_eq!(inspected.get(), COMPLETED_PREFIX);
    let (prepared, _cause) = interrupted.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let ordinary = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("ordinary validation work remains queryable");
    assert_eq!(
        ordinary.attempted() - ordinary.consumed(),
        u64::try_from(CANONICAL_UNITS + EVIDENCE_UNITS - COMPLETED_PREFIX).unwrap()
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_record_inspector_false_is_a_semantic_rejection() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_records(RecordPlan::new(1, 1).unwrap(), CanonicalRootTag, |writer| {
                    writer.write_row(|row| row.write_scalar(7_u8))
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        let canonical_valid = session.visit_canonical_records(
                            canonical,
                            1,
                            1,
                            |_row_index, row| {
                                assert_eq!(row.scalar::<u8, _>(0, |value| value)?, Some(7));
                                Ok(false)
                            },
                        )?;
                        assert!(!canonical_valid);
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_index, value| value == 9,
                        )?);
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            75,
                        ))
                    },
                )
            },
        )
        .unwrap();

    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a fully visited canonical record may reject semantically")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Requested(ValidationRejection::Semantic)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn outcome_evidence_record_inspector_false_is_a_semantic_rejection() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_records(
                        RecordPlan::new(1, 1).unwrap(),
                        EvidenceRecordLeafTag,
                        |writer| writer.write_row(|row| row.write_scalar(9_u8)),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(session.visit_canonical_sequence(
                            canonical,
                            1,
                            |_index, value| value == 7,
                        )?);
                        let evidence_valid = session.visit_outcome_evidence_records(
                            evidence,
                            1,
                            1,
                            |_row_index, row| {
                                assert_eq!(row.scalar::<u8, _>(0, |value| value)?, Some(9));
                                Ok(false)
                            },
                        )?;
                        assert!(!evidence_valid);
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            76,
                        ))
                    },
                )
            },
        )
        .unwrap();

    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a fully visited evidence record may reject semantically")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Requested(ValidationRejection::Semantic)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn control_interruption_before_validator_panic_preserves_one_terminal_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session,
                     canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        cancellation.cancel();
                        let cause = session
                            .visit_canonical_sequence(canonical, 1, |_index, _value| true)
                            .unwrap_err();
                        assert!(cause.control_cause().is_some());
                        panic!("panic after the first controlled interruption")
                    },
                )
            },
        )
        .unwrap();

    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a validator panic must retain minimal fallback authority")
        }
    };
    assert!(matches!(fallback.cause(), StoreRuntimeError::Control(_)));
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
fn ledger_interruption_before_validator_panic_preserves_one_terminal_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = validation_completion_limited_operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_dimensions(
            keys,
            1,
            1,
            keys.dimension,
            keys.other_dimension,
            keys.dimension,
            keys.other_dimension,
        ),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session,
                     canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        let cause = session
                            .visit_canonical_sequence(canonical, 1, |_index, _value| true)
                            .unwrap_err();
                        assert!(cause.ledger_cause().is_some());
                        panic!("panic after the first ledger interruption")
                    },
                )
            },
        )
        .unwrap();

    let published = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            assert!(matches!(fallback.cause(), StoreRuntimeError::Ledger(_)));
            fallback.into_parts().0.handoff()
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            assert!(interrupted.cause().ledger_cause().is_some());
            interrupted
                .prepare_minimal_fallback()
                .unwrap()
                .into_parts()
                .0
                .handoff()
        }
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_) => {
            panic!("a validator panic must preserve the first Ledger cause")
        }
    };
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn output_slot_outside_the_root_graph_cannot_form_a_candidate() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 2, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
                .unwrap();
            let other_root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(8)?;
                    Ok(())
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, other_root)
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        if inspect_one_byte_pair(session, canonical, evidence, 7, 9)? {
                            Ok(CanonicalValidationDecision::accept("sealed-root"))
                        } else {
                            Ok(CanonicalValidationDecision::reject(
                                ValidationRejection::Semantic,
                                72,
                            ))
                        }
                    },
                )
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a slot outside the sealed root graph must not form a candidate")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Runtime(StoreRuntimeError::CanonicalValidationContractRejected,)
    );
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert!(!account.snapshot().poisoned());
    drop(published);
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
fn activation_policy_keeps_validation_work_on_its_semantic_dimension() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = split_dimension_operation(&account);
    let activation = scoped_activation_shape_with_dimensions(
        keys,
        1,
        1,
        keys.dimension,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
    );

    let outcome = plan_scoped_activation!(operation, activation, |mut lane| {
        let root = lane
            .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                writer.push(7)?;
                Ok(())
            })
            .unwrap();
        lane.complete::<_, (), _, _>(root, root)
    },)
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(3).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&[1, 2, 3]),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            73,
                        ))
                    },
                )
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
    assert!(materialization.attempted() > 0);
    assert_eq!(
        (validation_work.attempted(), validation_work.consumed()),
        (4, 3)
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session,
                     _canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        unreachable!("cancelled evidence construction must skip central validation")
                    },
                )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane.try_retain_sequence(
                    SequencePlan::<u8>::new(2).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(9),
                );
                assert!(matches!(evidence, Err(StoreRuntimeError::Build(_))));
                lane.abort::<(), EvidenceLeafTag>()
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session,
                     _canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        unreachable!("incomplete evidence must skip central validation")
                    },
                )
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

    let outcome_b = plan_scoped_activation!(
        operation_b,
        scoped_activation_shape(keys_b, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(2)?;
                    Ok(())
                })
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

    let result = plan_scoped_activation!(
        operation_a,
        scoped_activation_shape(keys_a, 1, 1),
        |mut lane| {
            let own_root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(1)?;
                    Ok(())
                })
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_capacity(keys, 2, 1, keys, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_capacity(keys, 2, 1, keys, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
                .unwrap();
            let _extra = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(8)?;
                    Ok(())
                })
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

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 2, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
    struct PanicOnDropScratch;

    impl Drop for PanicOnDropScratch {
        fn drop(&mut self) {
            panic!("validation scratch destructor panic during validator unwind")
        }
    }

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(PanicOnDropScratch),
                    |_session,
                     _canonical,
                     _evidence,
                     _scratch|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > { panic!("recoverable central validator panic") },
                )
            },
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
fn scratch_drop_panic_still_drops_the_validation_proof_before_fallback() {
    struct PanicOnDropScratch;

    impl Drop for PanicOnDropScratch {
        fn drop(&mut self) {
            panic!("validation scratch destructor panic")
        }
    }

    struct ProofDropMarker<'a>(&'a Cell<bool>);

    impl Drop for ProofDropMarker<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
    let proof_dropped = Cell::new(false);

    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(PanicOnDropScratch),
                    |_session, _canonical, _evidence, _scratch| {
                        Ok(
                            CanonicalValidationDecision::<_, ValidationRejection>::accept(
                                ProofDropMarker(&proof_dropped),
                            ),
                        )
                    },
                )
            },
        )
        .unwrap();
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a scratch destructor panic must select minimal fallback")
        }
    };
    assert!(proof_dropped.get());
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(cause, StoreRuntimeError::Build(_)));
    drop(prepared.handoff());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_validation_panic_flushes_the_nonzero_completed_prefix() {
    const VALIDATION_UNITS: usize = 4;
    const EVIDENCE_CONSTRUCTION_UNITS: u64 = 1;
    const COMPLETED_PREFIX: usize = 2;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = split_dimension_operation(&account);
    let TestScopedActivation {
        policy, failure, ..
    } = scoped_activation_shape_with_dimensions(
        keys,
        1,
        1,
        keys.dimension,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
    );
    let activation = TestScopedActivation {
        policy,
        failure,
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(1, 1).unwrap(),
            1,
            NonZeroU64::new(u64::try_from(VALIDATION_UNITS).unwrap()).unwrap(),
        )
        .unwrap(),
    };
    let values = [7; VALIDATION_UNITS];
    let outcome = plan_scoped_activation!(operation, activation, |mut lane| {
        let root = lane
            .try_retain_sequence(
                SequencePlan::<u8>::new(VALIDATION_UNITS).unwrap(),
                SequenceTag,
                |writer| writer.extend_from_slice(&values),
            )
            .unwrap();
        lane.complete::<_, (), _, _>(root, root)
    },)
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let inspected = Cell::new(0usize);

    let outcome =
        activated
            .validate_centrally(
                CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                |lane, _output| {
                    let evidence = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            EvidenceLeafTag,
                            |writer| writer.push(9),
                        )
                        .unwrap();
                    lane.complete(evidence)
                },
                |planning, _output| {
                    planning.validate_with(
                        |_| Ok(()),
                        |session, canonical, _evidence, &mut ()| {
                            session
                    .visit_canonical_sequence(canonical, VALIDATION_UNITS, |index, value| {
                        if index == COMPLETED_PREFIX {
                            panic!("panic after the completed canonical-validation prefix");
                        }
                        assert_eq!(value, 7);
                        inspected.set(inspected.get() + 1);
                        true
                    })
                    .map(|accepted| {
                        if accepted {
                            CanonicalValidationDecision::accept(())
                        } else {
                            CanonicalValidationDecision::reject(ValidationRejection::Semantic, 73)
                        }
                    })
                        },
                    )
                },
            )
            .unwrap();
    let fallback = match outcome {
        CanonicalCentralValidationOutcome::Fallback(fallback) => fallback,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("a validator panic after completed work must select minimal fallback")
        }
    };
    assert_eq!(inspected.get(), COMPLETED_PREFIX);
    assert!(!account.snapshot().poisoned());
    let (prepared, cause) = fallback.into_parts();
    assert!(matches!(cause, StoreRuntimeError::Build(_)));
    let published = prepared.handoff();
    let validation = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("validation-work evidence remains queryable");
    assert_eq!(
        (validation.attempted(), validation.consumed()),
        (
            EVIDENCE_CONSTRUCTION_UNITS + u64::try_from(VALIDATION_UNITS).unwrap(),
            EVIDENCE_CONSTRUCTION_UNITS + u64::try_from(COMPLETED_PREFIX).unwrap(),
        ),
        "batch drop must flush the exact completed prefix without refunding it"
    );
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
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
        |lane, _output| {
            let evidence = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(9),
                )
                .unwrap();
            lane.complete(evidence)
        },
        move |planning, _output| {
            planning.validate_with(
                |_| Ok(()),
                |session, canonical, evidence, &mut ()| {
                    let _capture = &validation_capture;
                    let valid = inspect_one_byte_pair(session, canonical, evidence, 7, 9)?;
                    assert!(valid);
                    Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                },
            )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
        move |_lane, _output| -> OutcomeEvidenceDecision<(), EvidenceLeafTag> {
            let _capture = &evidence_capture;
            panic!("contained evidence callback panic")
        },
        |planning, _output| {
            planning.validate_with(
                |_| Ok(()),
                |_session,
                 _canonical,
                 _evidence,
                 &mut ()|
                 -> Result<CanonicalValidationDecision<(), ()>, StoreWorkError> {
                    unreachable!("evidence callback panic must skip central validation")
                },
            )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
        |planning, _output| {
            planning.validate_with(
                |_| Ok(()),
                |_session,
                 _canonical,
                 _evidence,
                 &mut ()|
                 -> Result<CanonicalValidationDecision<(), ()>, StoreWorkError> {
                    unreachable!("contract evidence fault must skip central validation")
                },
            )
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn candidate_reports_exact_member_counts_without_running_the_joint_commit() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_commit_limit(&account, 0);
    let evidence_called = Cell::new(false);
    let validation_called = Cell::new(false);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        validation_called.set(true);
                        assert!(inspect_one_byte_pair(session, canonical, evidence, 7, 1)?);
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
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
    assert_eq!(candidate.canonical_member_count(), 1);
    assert_eq!(candidate.outcome_evidence_member_count(), 1);
    drop(candidate);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn candidate_proof_drop_panic_poison_contains_the_physical_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
    let result = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(inspect_one_byte_pair(session, canonical, evidence, 7, 1)?);
                        Ok(CanonicalValidationDecision::<
                            PanicOnDrop,
                            ValidationRejection,
                        >::accept(PanicOnDrop))
                    },
                )
            },
        )
        .unwrap();
    let candidate = match result {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("valid central validation must produce a candidate")
        }
    };

    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(candidate)));
    assert!(dropped.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn candidate_output_drop_panic_poison_contains_the_physical_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap(),),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, PanicOnDrop)
        },
    )
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(state) => state,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => panic!("materialization must activate"),
    };
    let result = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(inspect_one_byte_pair(session, canonical, evidence, 7, 1)?);
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
            },
        )
        .unwrap();
    let candidate = match result {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("valid central validation must produce a candidate")
        }
    };

    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(candidate)));
    assert!(dropped.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn trusted_evidence_slot_limit_returns_product_graph_fallback_without_poisoning() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let validation_called = Cell::new(false);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        validation_called.set(true);
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 4),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            72,
                        ))
                    },
                )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 2),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session,
                     _canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        unreachable!("an envelope rejection must skip central validation")
                    },
                )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, _evidence, &mut ()| {
                        cancellation.cancel();
                        let _ =
                            session
                                .visit_canonical_sequence(canonical, 1, |_index, _value| true)?;
                        Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                    },
                )
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape_with_validation_work(
            keys,
            1,
            1,
            NonZeroU64::new(u64::MAX).unwrap(),
        ),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
    let validation_called = Cell::new(false);
    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session,
                     _canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        validation_called.set(true);
                        unreachable!("a rejected plan-bound envelope must skip the validator")
                    },
                )
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
    assert!(!validation_called.get());
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

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
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
                lane.abort::<(), EvidenceLeafTag>()
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session,
                     _canonical,
                     _evidence,
                     &mut ()|
                     -> Result<
                        CanonicalValidationDecision<(), ValidationRejection>,
                        StoreWorkError,
                    > {
                        unreachable!("aborted evidence must skip central validation")
                    },
                )
            },
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
fn completing_outcome_evidence_after_an_earlier_slot_fault_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
        |lane, _output| {
            let root = lane
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
            lane.complete(root)
        },
        |planning, _output| {
            planning.validate_with(
                |_| Ok(()),
                |_session,
                 _canonical,
                 _evidence,
                 &mut ()|
                 -> Result<
                    CanonicalValidationDecision<(), ValidationRejection>,
                    StoreWorkError,
                > { unreachable!("aborted evidence must skip central validation") },
            )
        },
    );

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn incomplete_validation_work_cannot_form_a_candidate() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
        |lane, _output| {
            let evidence = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(1),
                )
                .unwrap();
            lane.complete(evidence)
        },
        |planning, _output| {
            planning.validate_with(
                |_| Ok(()),
                |_session, _canonical, _evidence, &mut ()| {
                    Ok(CanonicalValidationDecision::<(), ValidationRejection>::accept(()))
                },
            )
        },
    );
    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("an incomplete validation permit must not form a candidate"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert!(account.snapshot().poisoned());
}

#[test]
fn central_validation_rejection_eagerly_cleans_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
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
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            72,
                        ))
                    },
                )
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
