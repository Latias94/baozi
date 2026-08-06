use super::*;

fn one_unit_rooted_failure_protected_work() -> (u64, u64) {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
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
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("the one-unit baseline must retain a rooted failure")
        }
    };
    let (prepared, _cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    (protected.attempted(), protected.consumed())
}

macro_rules! assert_exact_failure_construction_work {
    ($method:ident, $plan:expr, $tag:expr, $build:expr, $expected:expr) => {{
        let baseline = one_unit_rooted_failure_protected_work();
        let factory = factory(1);
        let account = account(&factory);
        let (state, keys) = rejected_session_with_one_evidence_slot(&account);
        let outcome = state
            .build_failure_evidence(|lane| {
                let root = lane.$method($plan, $tag, $build).unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(root)
            })
            .unwrap();
        let rooted = match outcome {
            FailureEvidenceOutcome::Rooted(rooted) => rooted,
            FailureEvidenceOutcome::MinimalFallback(_) => {
                panic!("valid evidence construction must remain rooted")
            }
        };
        let (prepared, _cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let protected = published
            .ledger_evidence
            .dimension(keys.failure_terminal_dimension)
            .expect("protected failure-work evidence remains queryable");
        let expected_delta = $expected - 1;
        assert_eq!(protected.attempted() - baseline.0, expected_delta);
        assert_eq!(protected.consumed() - baseline.1, expected_delta);
    }};
}

fn rejected_session_with_evidence_slots(
    account: &PhysicalStorageAccount,
    blocks: usize,
) -> (
    baozi_operation::__canonical_activation::NeverActivatedStoreSession<
        TestOwner,
        TestProgress,
        TestUnit,
        PreflightRejection,
    >,
    TestKeys,
) {
    let (operation, keys) = operation(account);
    rejected_session_from_operation(operation, keys, blocks)
}

fn protected_failure_work(
    evidence: &StoreLedgerEvidence,
    dimension: StoreDimensionKey,
) -> (u64, u64) {
    let protected = evidence
        .dimension(dimension)
        .expect("protected failure-work evidence remains queryable");
    (protected.attempted(), protected.consumed())
}

macro_rules! assert_exact_failure_validation_work {
    (
        $blocks:expr,
        |$lane:ident, $dimension:ident| $build:block,
        $validate:expr,
        $expected:expr
    ) => {{
        let baseline = {
            let factory = factory(1);
            let account = account(&factory);
            let (state, keys) = rejected_session_with_evidence_slots(&account, $blocks);
            let outcome = state
                .build_failure_evidence(|$lane| {
                    let $dimension = keys.dimension;
                    $build
                })
                .unwrap();
            let rooted = match outcome {
                FailureEvidenceOutcome::Rooted(rooted) => rooted,
                FailureEvidenceOutcome::MinimalFallback(_) => {
                    panic!("the baseline evidence graph must remain rooted")
                }
            };
            let (prepared, _cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
            let published = prepared.handoff();
            let work =
                protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
            drop(published);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
            work
        };

        let measured = {
            let factory = factory(1);
            let account = account(&factory);
            let (state, keys) = rejected_session_with_evidence_slots(&account, $blocks);
            let outcome = state
                .build_failure_evidence(|$lane| {
                    let $dimension = keys.dimension;
                    $build
                })
                .unwrap();
            let rooted = match outcome {
                FailureEvidenceOutcome::Rooted(rooted) => rooted,
                FailureEvidenceOutcome::MinimalFallback(_) => {
                    panic!("the measured evidence graph must remain rooted")
                }
            };
            let validation = rooted.validate_rich_failure($validate).unwrap();
            let validated = match validation {
                RootedFailureValidation::Valid(validated) => validated,
                RootedFailureValidation::Rejected(_) | RootedFailureValidation::Fallback(_) => {
                    panic!("the measured evidence graph must validate")
                }
            };
            let (prepared, _cause) = validated
                .try_prepare(final_progress())
                .unwrap()
                .into_parts();
            let prepared = match prepared {
                PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
                PreparedStoreFailureOutcome::MinimalFallback(_) => {
                    panic!("validated evidence must prepare a rich failure")
                }
            };
            let published = prepared.handoff();
            let work =
                protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
            drop(published);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
            work
        };

        let expected = u64::try_from($expected).unwrap();
        assert_eq!(measured.0.checked_sub(baseline.0).unwrap(), expected);
        assert_eq!(measured.1.checked_sub(baseline.1).unwrap(), expected);
    }};
}

#[test]
fn ignored_failure_evidence_fault_plus_complete_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_one_evidence_slot(&account);

    let result = state.build_failure_evidence(|lane| {
        let root = lane
            .try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                EvidenceLeafTag,
                |writer| writer.push(1),
            )
            .unwrap();
        let ignored = lane.try_retain_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |writer| writer.push(2),
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
                    EvidenceLeafTag,
                    move |_writer| {
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
fn failed_failure_evidence_builder_records_attempted_without_consumed_work() {
    const FAILED_WORK: u64 = 7;

    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);

    let outcome = state
        .build_failure_evidence(|lane| {
            let expected = StoreBuildError::domain(73).unwrap();
            let cause = lane
                .try_retain_records(
                    RecordPlan::new(1, usize::try_from(FAILED_WORK).unwrap()).unwrap(),
                    EvidenceLeafTag,
                    |writer| {
                        writer.write_row(|row| {
                            row.write_scalar(1_u8)?;
                            Err(expected)
                        })
                    },
                )
                .unwrap_err();
            assert_eq!(cause, StoreRuntimeError::Build(expected));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => panic!("failed evidence construction must downgrade"),
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted() - protected.consumed(),
        FAILED_WORK,
        "a failed builder must not fabricate completed work"
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_error_before_first_write_attempts_the_full_sealed_plan() {
    const PLANNED_WORK: u64 = 7;

    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let expected = StoreBuildError::domain(74).unwrap();
            let cause = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(usize::try_from(PLANNED_WORK).unwrap()).unwrap(),
                    EvidenceLeafTag,
                    |_writer| Err(expected),
                )
                .unwrap_err();
            assert_eq!(cause, StoreRuntimeError::Build(expected));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => panic!("failed construction must downgrade"),
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted() - protected.consumed(),
        PLANNED_WORK,
        "member admission records the complete plan before the builder runs"
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn bytes_utf8_and_records_charge_exact_completed_construction_work() {
    assert_exact_failure_construction_work!(
        try_retain_bytes,
        BytesPlan::new(3).unwrap(),
        EvidenceBytesTag,
        |writer| writer.write_bytes(b"abc"),
        3
    );
    assert_exact_failure_construction_work!(
        try_retain_utf8,
        Utf8Plan::new("\u{5305}\u{5b50}".len()).unwrap(),
        EvidenceTextTag,
        |writer| writer.write_str("\u{5305}\u{5b50}"),
        u64::try_from("\u{5305}\u{5b50}".len()).unwrap()
    );
    assert_exact_failure_construction_work!(
        try_retain_records,
        RecordPlan::new(2, 3).unwrap(),
        EvidenceRecordLeafTag,
        |writer| {
            writer.write_row(|row| {
                row.write_scalar(1_u8)?;
                row.write_scalar(2_u16)?;
                row.write_scalar(3_u32)
            })?;
            writer.write_row(|row| {
                row.write_scalar(4_u8)?;
                row.write_scalar(5_u16)?;
                row.write_scalar(6_u32)
            })
        },
        6
    );
}

#[test]
fn failure_validation_charges_each_binary_byte_exactly() {
    const PAYLOAD: &[u8] = b"baozi";

    assert_exact_failure_validation_work!(
        1,
        |lane, dimension| {
            let root = lane
                .try_retain_bytes(
                    BytesPlan::new(PAYLOAD.len()).unwrap(),
                    EvidenceBytesTag,
                    |writer| writer.write_bytes(PAYLOAD),
                )
                .unwrap();
            lane.commit(dimension, 1).unwrap();
            lane.complete(root)
        },
        |session, root| {
            session
                .visit_bytes(root, |index, value| value == PAYLOAD[index])
                .unwrap()
        },
        PAYLOAD.len()
    );
}

#[test]
fn failure_validation_charges_encoded_utf8_bytes_not_scalar_count() {
    const TEXT: &str = "A\u{e9}\u{1f980}";

    assert_eq!(TEXT.len(), 7);
    assert_eq!(TEXT.chars().count(), 3);
    assert_exact_failure_validation_work!(
        1,
        |lane, dimension| {
            let root = lane
                .try_retain_utf8(
                    Utf8Plan::new(TEXT.len()).unwrap(),
                    EvidenceTextTag,
                    |writer| writer.write_str(TEXT),
                )
                .unwrap();
            lane.commit(dimension, 1).unwrap();
            lane.complete(root)
        },
        |session, root| {
            session
                .visit_utf8_bytes(root, |index, value| value == TEXT.as_bytes()[index])
                .unwrap()
        },
        TEXT.len()
    );
}

#[test]
fn failure_utf8_metering_preserves_a_scalar_across_the_chunk_boundary() {
    let mut text = "a".repeat(255);
    text.push('\u{1f980}');
    text.push('z');
    assert_eq!(text.len(), 260);

    assert_exact_failure_validation_work!(
        1,
        |lane, dimension| {
            let root = lane
                .try_retain_utf8(
                    Utf8Plan::new(text.len()).unwrap(),
                    EvidenceTextTag,
                    |writer| writer.write_str(&text),
                )
                .unwrap();
            lane.commit(dimension, 1).unwrap();
            lane.complete(root)
        },
        |session, root| {
            session
                .visit_utf8_bytes(root, |index, value| value == text.as_bytes()[index])
                .unwrap()
        },
        text.len()
    );
}

#[test]
fn failure_validation_cancellation_preserves_the_completed_prefix() {
    const PLANNED_WORK: u64 = 3;

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (state, keys) = rejected_session_with_control(&account, control);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(usize::try_from(PLANNED_WORK).unwrap()).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.extend_from_slice(&[7, 8, 9]),
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
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |index, value| {
                    assert_eq!(value, 7 + u8::try_from(index).unwrap());
                    if index == 0 {
                        cancellation.cancel();
                    }
                    true
                })
                .unwrap_or(false)
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("mid-validation cancellation must select minimal fallback")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted() - protected.consumed(),
        PLANNED_WORK - 1,
        "validation attempts the full plan and consumes only the completed prefix"
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn failure_validation_charges_record_cells_and_every_typed_reference() {
    const ROOT_ROWS: usize = 2;
    const ROOT_FIELDS: usize = 5;
    const LEAF_UNITS: usize = 4;
    const EXPECTED_VALIDATION_UNITS: usize = ROOT_ROWS * ROOT_FIELDS + LEAF_UNITS;

    assert_exact_failure_validation_work!(
        5,
        |lane, dimension| {
            let sequence = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(7),
                )
                .unwrap();
            let bytes = lane
                .try_retain_bytes(BytesPlan::new(1).unwrap(), EvidenceBytesTag, |writer| {
                    writer.write_bytes(&[8])
                })
                .unwrap();
            let text = lane
                .try_retain_utf8(Utf8Plan::new(1).unwrap(), EvidenceTextTag, |writer| {
                    writer.write_str("x")
                })
                .unwrap();
            let records = lane
                .try_retain_records(
                    RecordPlan::new(1, 1).unwrap(),
                    EvidenceRecordLeafTag,
                    |writer| writer.write_row(|row| row.write_scalar(9_u16)),
                )
                .unwrap();
            let root = lane
                .try_retain_records(
                    RecordPlan::new(ROOT_ROWS, ROOT_FIELDS).unwrap(),
                    EvidenceRootTag,
                    |writer| {
                        writer.write_row(|row| {
                            row.write_scalar(42_u32)?;
                            row.write_sequence_ref(sequence)?;
                            row.write_bytes_ref(bytes)?;
                            row.write_utf8_ref(text)?;
                            row.write_record_ref(records)
                        })?;
                        writer.write_row(|row| {
                            row.write_scalar(43_u32)?;
                            row.write_sequence_ref(sequence)?;
                            row.write_bytes_ref(bytes)?;
                            row.write_utf8_ref(text)?;
                            row.write_record_ref(records)
                        })
                    },
                )
                .unwrap();
            lane.commit(dimension, 1).unwrap();
            lane.complete(root)
        },
        |session, root| {
            let mut sequence = None;
            let mut bytes = None;
            let mut text = None;
            let mut records = None;
            let root_valid = session
                .visit_records(root, |row_index, row| {
                    assert_eq!(row.row(), row_index);
                    assert_eq!(row.fields(), ROOT_FIELDS);
                    let scalar_valid = row
                        .scalar::<u32, _>(0, |value| {
                            value == 42 + u32::try_from(row_index).unwrap()
                        })
                        .unwrap();
                    let sequence_valid = row
                        .sequence_ref::<u8, EvidenceLeafTag, _>(1, |value| {
                            sequence = Some(value);
                            true
                        })
                        .unwrap();
                    let bytes_valid = row
                        .bytes_ref::<EvidenceBytesTag, _>(2, |value| {
                            bytes = Some(value);
                            true
                        })
                        .unwrap();
                    let text_valid = row
                        .utf8_ref::<EvidenceTextTag, _>(3, |value| {
                            text = Some(value);
                            true
                        })
                        .unwrap();
                    let records_valid = row
                        .record_ref::<EvidenceRecordLeafTag, _>(4, |value| {
                            records = Some(value);
                            true
                        })
                        .unwrap();
                    scalar_valid && sequence_valid && bytes_valid && text_valid && records_valid
                })
                .unwrap();
            root_valid
                && session
                    .visit_sequence(
                        sequence.expect("record sequence reference"),
                        |_index, value| value == 7,
                    )
                    .unwrap()
                && session
                    .visit_bytes(bytes.expect("record bytes reference"), |_index, value| {
                        value == 8
                    })
                    .unwrap()
                && session
                    .visit_utf8_bytes(text.expect("record UTF-8 reference"), |_index, value| {
                        value == b'x'
                    })
                    .unwrap()
                && session
                    .visit_records(
                        records.expect("record child reference"),
                        |_row_index, row| row.scalar::<u16, _>(0, |value| value == 9).unwrap(),
                    )
                    .unwrap()
        },
        EXPECTED_VALIDATION_UNITS
    );
}

#[test]
fn repeated_record_edges_validate_the_child_once_without_recharging_it() {
    assert_exact_failure_validation_work!(
        2,
        |lane, dimension| {
            let child = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(7),
                )
                .unwrap();
            let root = lane
                .try_retain_records(RecordPlan::new(1, 2).unwrap(), EvidenceRootTag, |writer| {
                    writer.write_row(|row| {
                        row.write_sequence_ref(child)?;
                        row.write_sequence_ref(child)
                    })
                })
                .unwrap();
            lane.commit(dimension, 1).unwrap();
            lane.complete(root)
        },
        |session, root| {
            let mut first = None;
            let mut repeated = None;
            let root_valid = session
                .visit_records(root, |_row_index, row| {
                    let first_valid = row
                        .sequence_ref::<u8, EvidenceLeafTag, _>(0, |value| {
                            first = Some(value);
                            true
                        })
                        .unwrap();
                    let repeated_valid = row
                        .sequence_ref::<u8, EvidenceLeafTag, _>(1, |value| {
                            repeated = Some(value);
                            true
                        })
                        .unwrap();
                    first_valid && repeated_valid
                })
                .unwrap();
            let mut inspected = 0;
            let first_valid = session
                .visit_sequence(
                    first.expect("the first edge yields a child capability"),
                    |_, value| {
                        inspected += 1;
                        value == 7
                    },
                )
                .unwrap();
            let repeated_valid = session
                .visit_sequence(
                    repeated.expect("the repeated edge yields a child capability"),
                    |_, _| {
                        inspected += 1;
                        true
                    },
                )
                .unwrap();
            assert_eq!(
                inspected, 1,
                "a completed DAG member is not charged or inspected twice"
            );
            root_valid && first_valid && repeated_valid
        },
        3
    );
}

#[test]
fn semantic_rejection_stops_repeated_child_validation() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_evidence_slots(&account, 2);
    let outcome = state
        .build_failure_evidence(|lane| {
            let child = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(2).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.extend_from_slice(&[7, 8]),
                )
                .unwrap();
            let root = lane
                .try_retain_records(RecordPlan::new(1, 2).unwrap(), EvidenceRootTag, |writer| {
                    writer.write_row(|row| {
                        row.write_sequence_ref(child)?;
                        row.write_sequence_ref(child)
                    })
                })
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
        .validate_rich_failure(|session, root| {
            let mut first = None;
            let mut repeated = None;
            session
                .visit_records(root, |_row_index, row| {
                    let first_valid = row
                        .sequence_ref::<u8, EvidenceLeafTag, _>(0, |value| {
                            first = Some(value);
                            true
                        })
                        .unwrap();
                    let repeated_valid = row
                        .sequence_ref::<u8, EvidenceLeafTag, _>(1, |value| {
                            repeated = Some(value);
                            true
                        })
                        .unwrap();
                    first_valid && repeated_valid
                })
                .unwrap();
            assert!(
                !session
                    .visit_sequence(first.expect("first child capability"), |index, _| index
                        != 0)
                    .unwrap()
            );
            let mut repeated_inspections = 0;
            assert!(
                !session
                    .visit_sequence(repeated.expect("repeated child capability"), |_, _| {
                        repeated_inspections += 1;
                        true
                    })
                    .unwrap()
            );
            assert_eq!(repeated_inspections, 0);
            true
        })
        .unwrap();
    let rejected = match validation {
        RootedFailureValidation::Rejected(rejected) => rejected,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Fallback(_) => {
            panic!("a semantic rejection remains requested after a repeated edge")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let protected =
        protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
    assert_eq!(protected.0 - protected.1, 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn an_unreachable_local_member_forces_product_graph_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_evidence_slots(&account, 2);
    let outcome = state
        .build_failure_evidence(|lane| {
            let _unreachable = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(99),
                )
                .unwrap();
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceRootTag,
                    |writer| writer.push(7),
                )
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("the unreachable member is rejected by validation, not construction")
        }
    };
    let validation = rooted
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |_index, value| value == 7)
                .unwrap()
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("an unreachable local member cannot receive a closure proof")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn foreign_record_reference_forces_product_graph_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (source_state, source_keys) = rejected_session_with_one_evidence_slot(&account);
    let mut foreign = None;
    let source_outcome = source_state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(7),
                )
                .unwrap();
            foreign = Some(root);
            lane.commit(source_keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let source = match source_outcome {
        FailureEvidenceOutcome::Rooted(source) => source,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("the source evidence must remain live for the cross-set probe")
        }
    };

    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_records(RecordPlan::new(1, 1).unwrap(), EvidenceRootTag, |writer| {
                    writer.write_row(|row| {
                        row.write_sequence_ref(
                            foreign.expect("the source callback records its typed slot"),
                        )
                    })
                })
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("the foreign edge is detected during validation")
        }
    };
    let validation = rooted
        .validate_rich_failure(|session, root| {
            session
                .visit_records(root, |_row_index, row| {
                    let ignored = row.sequence_ref::<u8, EvidenceLeafTag, _>(0, |_slot| true);
                    assert!(ignored.is_err());
                    true
                })
                .unwrap_or(false)
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("a cross-set edge cannot become valid or an ordinary semantic rejection")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);

    let (source_prepared, _cause) = source.prepare_minimal_fallback().unwrap().into_parts();
    drop(source_prepared.handoff());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn accepted_record_row_with_an_unvisited_field_forces_product_graph_fallback() {
    let baseline = {
        let factory = factory(1);
        let account = account(&factory);
        let (state, keys) = rejected_session_with_evidence_slots(&account, 1);
        let outcome = state
            .build_failure_evidence(|lane| {
                let root = lane
                    .try_retain_records(RecordPlan::new(1, 2).unwrap(), EvidenceRootTag, |writer| {
                        writer.write_row(|row| {
                            row.write_scalar(1_u8)?;
                            row.write_scalar(2_u8)
                        })
                    })
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(root)
            })
            .unwrap();
        let rooted = match outcome {
            FailureEvidenceOutcome::Rooted(rooted) => rooted,
            FailureEvidenceOutcome::MinimalFallback(_) => {
                panic!("the baseline record evidence must remain rooted")
            }
        };
        let (prepared, _cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let work =
            protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
        drop(published);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        work
    };

    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_evidence_slots(&account, 1);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_records(RecordPlan::new(1, 2).unwrap(), EvidenceRootTag, |writer| {
                    writer.write_row(|row| {
                        row.write_scalar(1_u8)?;
                        row.write_scalar(2_u8)
                    })
                })
                .unwrap();
            lane.commit(keys.dimension, 1).unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("the measured record evidence must remain rooted")
        }
    };
    let validation = rooted
        .validate_rich_failure(|session, root| {
            let cause = session
                .visit_records(root, |_row_index, row| {
                    row.scalar::<u8, _>(0, |value| value == 1).unwrap()
                })
                .unwrap_err();
            assert!(matches!(cause, StoreRuntimeError::Build(_)));
            true
        })
        .unwrap();
    let fallback = match validation {
        RootedFailureValidation::Fallback(fallback) => fallback,
        RootedFailureValidation::Valid(_) | RootedFailureValidation::Rejected(_) => {
            panic!("an accepted row with an unvisited field must fall back")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let measured =
        protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
    assert_eq!(measured.0.checked_sub(baseline.0).unwrap(), 2);
    assert_eq!(measured.1.checked_sub(baseline.1).unwrap(), 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
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
                    EvidenceLeafTag,
                    |writer| writer.push(1),
                )
                .unwrap();
            let ignored = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(),
                EvidenceLeafTag,
                |writer| writer.push(2),
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
                    SequencePlan::<u8>::new(
                        usize::try_from(TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT).unwrap(),
                    )
                    .unwrap(),
                    EvidenceLeafTag,
                    |writer| {
                        writer.extend_from_slice(&vec![
                            7;
                            usize::try_from(
                                TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT
                            )
                            .unwrap()
                        ])
                    },
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
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |_index, value| value == 7)
                .unwrap()
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
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted(),
        protected.consumed(),
        "successful construction and validation must complete every admitted work unit"
    );
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
                SequencePlan::<u8>::new(
                    usize::try_from(TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT + 1).unwrap(),
                )
                .unwrap(),
                EvidenceLeafTag,
                |_writer| unreachable!("the one-short build must be rejected first"),
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
fn one_short_failure_validation_budget_rejects_unvalidatable_evidence() {
    let factory = factory(1);
    let account = account(&factory);
    let (state, _keys) = rejected_session_with_failure_work_limits(&account, 2, 1);
    let outcome = state
        .build_failure_evidence(|lane| {
            let error = lane.try_retain_sequence(
                SequencePlan::<u8>::new(2).unwrap(),
                EvidenceLeafTag,
                |_writer| unreachable!("validation admission must reject before construction"),
            );
            assert!(matches!(error, Err(StoreRuntimeError::Build(_))));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => {
            panic!("evidence without enough validation work must downgrade")
        }
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
                    EvidenceLeafTag,
                    |writer| writer.push(7),
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
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |_index, value| value == 7)
                .unwrap()
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
                EvidenceLeafTag,
                |_writer| unreachable!("cancellation must be polled before construction"),
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
fn failure_construction_polls_between_completed_units() {
    const PLANNED_WORK: u64 = 2;

    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (state, keys) = rejected_session_with_control(&account, control);

    let outcome = state
        .build_failure_evidence(|lane| {
            let result = lane.try_retain_sequence(
                SequencePlan::<u8>::new(usize::try_from(PLANNED_WORK).unwrap()).unwrap(),
                EvidenceLeafTag,
                |writer| {
                    writer.push(7)?;
                    cancellation.cancel();
                    writer.push(8)
                },
            );
            assert!(matches!(result, Err(StoreRuntimeError::Control(_))));
            lane.abort::<(), ()>()
        })
        .unwrap();
    let fallback = match outcome {
        FailureEvidenceOutcome::MinimalFallback(fallback) => fallback,
        FailureEvidenceOutcome::Rooted(_) => {
            panic!("mid-construction cancellation must discard rich evidence")
        }
    };
    let (prepared, _cause) = fallback.into_parts();
    let published = prepared.handoff();
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted() - protected.consumed(),
        1,
        "the complete plan is attempted while only the completed prefix is consumed"
    );
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
                    EvidenceLeafTag,
                    |writer| writer.push(7),
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

    let result = rooted.validate_rich_failure(move |_session, _root| {
        let _capture = &capture;
        panic!("contained rich-failure validation panic")
    });

    assert!(result.is_err());
    assert!(account.snapshot().poisoned());
}

#[test]
fn clean_rich_failure_validation_panic_prepares_fallback_without_poisoning() {
    const PLANNED_WORK: usize = 7;

    let baseline = {
        let factory = factory(1);
        let account = account(&factory);
        let (state, keys) = rejected_session_with_one_evidence_slot(&account);
        let outcome = state
            .build_failure_evidence(|lane| {
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(PLANNED_WORK).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.extend_from_slice(&[7; PLANNED_WORK]),
                    )
                    .unwrap();
                lane.commit(keys.dimension, 1).unwrap();
                lane.complete(root)
            })
            .unwrap();
        let rooted = match outcome {
            FailureEvidenceOutcome::Rooted(rooted) => rooted,
            FailureEvidenceOutcome::MinimalFallback(_) => {
                panic!("the panic baseline must remain rooted")
            }
        };
        let (prepared, _cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
        let published = prepared.handoff();
        let work =
            protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
        drop(published);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        work
    };

    let factory = factory(1);
    let account = account(&factory);
    let (state, keys) = rejected_session_with_one_evidence_slot(&account);
    let outcome = state
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(PLANNED_WORK).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.extend_from_slice(&[7; PLANNED_WORK]),
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
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |index, value| -> bool {
                    assert_eq!(index, 0);
                    assert_eq!(value, 7);
                    panic!("recoverable rich-failure validator panic")
                })
                .unwrap()
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
    let measured =
        protected_failure_work(&published.ledger_evidence, keys.failure_terminal_dimension);
    assert_eq!(
        measured.0.checked_sub(baseline.0).unwrap(),
        u64::try_from(PLANNED_WORK).unwrap(),
        "the complete sealed validation plan is attempted before the first unit"
    );
    assert_eq!(
        measured.1.checked_sub(baseline.1).unwrap(),
        0,
        "a panic in the first validation unit cannot fabricate completed work"
    );
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
                    SequencePlan::<u8>::new(3).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.extend_from_slice(&[7, 8, 9]),
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
        .validate_rich_failure(|session, root| {
            session
                .visit_sequence(root, |_index, value| {
                    assert_eq!(value, 7);
                    false
                })
                .unwrap()
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
    let protected = published
        .ledger_evidence
        .dimension(keys.failure_terminal_dimension)
        .expect("protected failure-work evidence remains queryable");
    assert_eq!(
        protected.attempted() - protected.consumed(),
        2,
        "semantic rejection consumes only the inspected prefix"
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
