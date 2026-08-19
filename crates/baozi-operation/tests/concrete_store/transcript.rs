use std::cell::Cell;

use super::*;
use baozi_operation::{
    OutcomeEvidenceSequenceRef, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedStoreFallbackParts, ScalarSequence, SequenceBatchItem, SequenceBatchPlan,
    StoreBuildError, StoreExecutionError, StoreTranscriptFailureValidation,
    StoreTranscriptPlanError, StoreTranscriptStartCause, StoreTranscriptSuccessValidation,
};

type TranscriptCanonicalSource = PublishedCanonicalStore<ScalarSequence<u32>, CanonicalRootTag>;
type TranscriptEvidenceSource = PublishedOutcomeEvidenceStore<ScalarSequence<u8>, EvidenceRootTag>;
type DerivedTranscriptOperation = DerivedRetainedStoreOperation<TestOwner, TestProgress, TestUnit>;

const DERIVED_TRANSCRIPT_MAX_FAILURE_EVIDENCE_MEMBERS: usize = 3;

struct TranscriptSource {
    canonical: TranscriptCanonicalSource,
    canonical_root: CanonicalSequenceRef<u32, CanonicalRootTag>,
    evidence: TranscriptEvidenceSource,
    evidence_root: OutcomeEvidenceSequenceRef<u8, EvidenceRootTag>,
}

fn transcript_writer(keys: TestKeys) -> RetainedStoreWriterPlan {
    writer_plan(keys, 8, 4)
}

fn derived_transcript_operation(
    source: &TranscriptCanonicalSource,
) -> (DerivedTranscriptOperation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::new(
        keys.failure_terminal_dimension,
        DERIVED_TRANSCRIPT_MAX_FAILURE_EVIDENCE_MEMBERS,
        u64::MAX,
        u64::MAX,
    )
    .unwrap();
    let policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work.ordinary_attempted_work_limit(),
        failure_terminal_work.ordinary_consumed_work_limit(),
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, policy),
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
        TestOperation::try_start_derived(OperationControl::default(), ledger_plan, source, start)
            .unwrap(),
        keys,
    )
}

fn publish_transcript_source(account: &PhysicalStorageAccount) -> TranscriptSource {
    let (operation, keys) = operation(account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(41))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(7))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0))
                    == Some(41)
                    && session
                        .outcome_evidence_sequence(evidence)
                        .and_then(|view| view.get(0))
                        == Some(7)
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("transcript source must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("transcript source must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(ledger_evidence.header().active_reservations(), 0);
    drop(ledger_evidence);
    TranscriptSource {
        canonical,
        canonical_root,
        evidence: outcome_evidence,
        evidence_root,
    }
}

#[test]
fn exact_success_transcript_prepares_zero_live_operation_and_handoffs_purely() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let canonical_commit = transcript.plan_canonical_commit(keys.dimension, 5).unwrap();
    let evidence_commit = transcript
        .plan_outcome_evidence_commit(keys.dimension, 6)
        .unwrap();
    assert_eq!(transcript.instruction_count(), 4);

    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(11))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(13))
        .unwrap();
    execution
        .try_execute_canonical_commit(&canonical_commit)
        .unwrap();
    execution
        .try_execute_outcome_evidence_commit(&evidence_commit)
        .unwrap();
    assert!(execution.primary_is_complete());
    assert_eq!(execution.primary_cursor(), 4);

    let wrong_mode =
        match execution.try_validate_rich_failure(evidence_root, |_session, _evidence| true) {
            Ok(_) => panic!("success-capable transcript must reject rich-failure validation"),
            Err(error) => error,
        };
    assert!(matches!(
        wrong_mode.cause(),
        StoreExecutionError::Runtime(StoreRuntimeError::WrongOutcomeMode)
    ));
    let execution = wrong_mode.into_state();
    assert!(execution.primary_is_complete());

    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0))
                    == Some(11)
                    && session
                        .outcome_evidence_sequence(evidence)
                        .and_then(|view| view.get(0))
                        == Some(13)
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("exact transcript graph must validate"),
    };
    assert!(!validated.has_failure_continuation());
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("validated transcript must prepare success"),
    };
    let physical_before_handoff = account.snapshot();
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(account.snapshot(), physical_before_handoff);
    assert_eq!(canonical.sequence(canonical_root).unwrap().get(0), Some(11));
    assert_eq!(
        outcome_evidence.sequence(evidence_root).unwrap().get(0),
        Some(13)
    );
    let header = ledger_evidence.header();
    assert_eq!(header.current_live_bytes(), 0);
    assert_eq!(header.active_reservations(), 0);
    assert_eq!(header.current_evidence_bytes(), 0);
    let dimension = ledger_evidence.dimension(keys.dimension).unwrap();
    assert_eq!(dimension.current_live_bytes(), 0);
    assert_eq!(dimension.pending_publication(), 0);
    assert_eq!(dimension.committed_canonical_output(), 5);
    assert_eq!(dimension.published_outcome_evidence(), 7);
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn ticket_rejection_preserves_primary_cursor_for_exact_retry() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();

    let (foreign_operation, foreign_keys) = super::operation(&account);
    let mut foreign_transcript = foreign_operation
        .try_begin_success_transcript(
            transcript_writer(foreign_keys),
            transcript_writer(foreign_keys),
        )
        .unwrap();
    let foreign_ticket = foreign_transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let _foreign_evidence_ticket = foreign_transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let foreign_execution = foreign_transcript.try_open().unwrap();
    let before_rejections = account.snapshot();
    let rejected_callback_called = Cell::new(false);
    assert!(matches!(
        execution.try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| {
            rejected_callback_called.set(true);
            writer.push(3)
        }),
        Err(StoreExecutionError::OutOfOrder {
            expected: 0,
            actual: 1
        })
    ));
    assert!(!rejected_callback_called.get());
    assert_eq!(execution.primary_cursor(), 0);
    assert!(matches!(
        execution.try_execute_canonical_sequence(&foreign_ticket, |writer| writer.push(5)),
        Err(StoreExecutionError::ForeignTicket)
    ));
    assert_eq!(execution.primary_cursor(), 0);
    assert_eq!(account.snapshot(), before_rejections);

    let _ = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(7))
        .unwrap();
    assert_eq!(execution.primary_cursor(), 1);
    assert!(matches!(
        execution.try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(9)),
        Err(StoreExecutionError::OutOfOrder {
            expected: 1,
            actual: 0
        })
    ));
    assert_eq!(execution.primary_cursor(), 1);
    let _ = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(11))
        .unwrap();
    assert!(execution.primary_is_complete());
    assert!(matches!(
        execution.try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(13)),
        Err(StoreExecutionError::SegmentComplete)
    ));
    assert_eq!(execution.primary_cursor(), 2);

    drop(execution.prepare_minimal_fallback().unwrap().handoff());
    drop(
        foreign_execution
            .prepare_minimal_fallback()
            .unwrap()
            .handoff(),
    );
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn ordinary_build_failure_leaves_instruction_retryable() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let before_failure = account.snapshot();
    let expected = StoreBuildError::domain(73).unwrap();
    assert!(matches!(
        execution.try_execute_canonical_sequence(&canonical_ticket, |writer| {
            writer.push(99)?;
            Err(expected)
        }),
        Err(StoreExecutionError::Runtime(StoreRuntimeError::Build(cause)))
            if cause == expected
    ));
    assert_eq!(execution.primary_cursor(), 0);
    let after_failure = account.snapshot();
    assert_eq!(
        after_failure.live_retained_bytes(),
        before_failure.live_retained_bytes()
    );
    assert_eq!(
        after_failure.live_retained_allocations(),
        before_failure.live_retained_allocations()
    );

    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(17))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(19))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session.canonical_sequence(canonical).is_some()
                    && session.outcome_evidence_sequence(evidence).is_some()
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("retried transcript must validate"),
    };
    drop(validated.prepare_minimal_fallback().unwrap().handoff());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn semantic_rejection_uses_the_precompiled_tail_and_seals_after_protected_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    transcript.begin_failure_continuation().unwrap();
    let detail_ticket = transcript
        .plan_outcome_evidence_utf8(8, EvidenceTextTag)
        .unwrap();
    let failure_root_ticket = transcript
        .plan_outcome_evidence_records(1, 2, EvidenceRecordLeafTag)
        .unwrap();
    let _failure_commit = transcript
        .plan_outcome_evidence_commit(keys.dimension, 3)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(23))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(29))
        .unwrap();
    assert!(execution.primary_is_complete());
    let mut failure = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |_session, _canonical, _evidence| false,
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must enter the planned failure tail"),
    };
    assert_eq!(failure.cursor(), 2);
    assert_eq!(failure.evidence_root(), evidence_root);
    assert!(!failure.is_complete());

    let incomplete = match failure.try_validate_rich_failure(evidence_root, |session, evidence| {
        session.sequence(evidence).is_some()
    }) {
        Ok(_) => panic!("incomplete failure tail must not validate"),
        Err(error) => error,
    };
    assert!(matches!(
        incomplete.cause(),
        StoreExecutionError::OutOfOrder {
            expected: 5,
            actual: 2
        }
    ));
    failure = incomplete.into_state();
    assert_eq!(failure.cursor(), 2);
    let rejected_callback_called = Cell::new(false);
    assert!(matches!(
        failure.try_execute_records(&failure_root_ticket, |_writer| {
            rejected_callback_called.set(true);
            Ok(())
        }),
        Err(StoreExecutionError::OutOfOrder {
            expected: 2,
            actual: 3
        })
    ));
    assert!(!rejected_callback_called.get());
    assert_eq!(failure.cursor(), 2);

    let before_tail_failure = account.snapshot();
    let expected_tail_error = StoreBuildError::domain(74).unwrap();
    assert!(matches!(
        failure.try_execute_utf8(&detail_ticket, |writer| {
            writer.write_str("temp")?;
            Err(expected_tail_error)
        }),
        Err(StoreExecutionError::Runtime(StoreRuntimeError::Build(cause)))
            if cause == expected_tail_error
    ));
    assert_eq!(failure.cursor(), 2);
    let after_tail_failure = account.snapshot();
    assert_eq!(
        after_tail_failure.live_retained_bytes(),
        before_tail_failure.live_retained_bytes()
    );
    assert_eq!(
        after_tail_failure.live_retained_allocations(),
        before_tail_failure.live_retained_allocations()
    );

    let rejected_retry_called = Cell::new(false);
    assert!(matches!(
        failure.try_execute_utf8(&detail_ticket, |_writer| {
            rejected_retry_called.set(true);
            Ok(())
        }),
        Err(StoreExecutionError::Runtime(
            StoreRuntimeError::FallbackOnly
        ))
    ));
    assert!(!rejected_retry_called.get());

    let PublishedStoreFallbackParts {
        ledger_evidence, ..
    } = failure.prepare_minimal_fallback().unwrap().handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(ledger_evidence.header().active_reservations(), 0);
    assert_eq!(
        store_failure_cause(&ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert_eq!(store_failure_cause(&ledger_evidence).code(), 0x4654_5753);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn validated_success_downgrade_reuses_the_same_failure_tail() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    transcript.begin_failure_continuation().unwrap();
    let failure_root_ticket = transcript
        .plan_outcome_evidence_records(1, 1, EvidenceRecordLeafTag)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(31))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(37))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session.canonical_sequence(canonical).is_some()
                    && session.outcome_evidence_sequence(evidence).is_some()
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("success must validate before explicit downgrade"),
    };
    assert!(validated.has_failure_continuation());
    let mut failure = validated.downgrade_to_rich_failure(91);
    assert_eq!(failure.cursor(), 2);
    assert_eq!(failure.evidence_root(), evidence_root);
    let failure_root = failure
        .try_execute_records(&failure_root_ticket, |writer| {
            writer.write_row(|row| row.write_sequence_ref(evidence_root))
        })
        .unwrap();
    assert!(failure.is_complete());
    let validated = match failure
        .try_validate_rich_failure(failure_root, |session, root| {
            let Some(records) = session.records(root) else {
                return false;
            };
            let Some(preserved) = records.sequence_ref::<u8, EvidenceRootTag>(0, 0) else {
                return false;
            };
            session.sequence(preserved).and_then(|view| view.get(0)) == Some(37)
        })
        .unwrap()
    {
        StoreTranscriptFailureValidation::Valid(validated) => validated,
        _ => panic!("explicit downgrade tail must validate"),
    };
    let prepared = match validated.try_prepare(92, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated explicit downgrade must remain rich")
        }
    };
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(store_failure_cause(&ledger_evidence).code(), 91);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn rich_failure_mode_guard_preserves_executor_for_correct_validation() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_transcript_source(&account);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_rich_failure_transcript(transcript_writer(keys))
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(43))
        .unwrap();
    assert!(execution.primary_is_complete());

    let wrong_mode = match execution.try_validate_success(
        source.canonical_root,
        evidence_root,
        |_session, _canonical, _evidence| true,
    ) {
        Ok(_) => panic!("rich-failure-only transcript must reject success validation"),
        Err(error) => error,
    };
    assert!(matches!(
        wrong_mode.cause(),
        StoreExecutionError::Runtime(StoreRuntimeError::WrongOutcomeMode)
    ));
    let execution = wrong_mode.into_state();
    assert!(execution.primary_is_complete());
    assert_eq!(execution.primary_cursor(), 1);

    let validated = match execution
        .try_validate_rich_failure(evidence_root, |session, evidence| {
            session.sequence(evidence).and_then(|view| view.get(0)) == Some(43)
        })
        .unwrap()
    {
        StoreTranscriptFailureValidation::Valid(validated) => validated,
        _ => panic!("rich-failure-only transcript must validate through failure mode"),
    };
    let prepared = match validated.try_prepare(93, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated rich failure must remain rich")
        }
    };
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(
        outcome_evidence
            .outcome_evidence()
            .sequence(evidence_root)
            .unwrap()
            .get(0),
        Some(43)
    );
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    drop(outcome_evidence);
    drop(ledger_evidence);
    drop(source.canonical);
    drop(source.evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn incomplete_primary_validation_preserves_executor_until_completion() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_transcript_source(&account);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(47))
        .unwrap();

    let incomplete = match execution.try_validate_success(
        canonical_root,
        source.evidence_root,
        |_session, _canonical, _evidence| true,
    ) {
        Ok(_) => panic!("incomplete primary must not enter semantic validation"),
        Err(error) => error,
    };
    assert!(matches!(
        incomplete.cause(),
        StoreExecutionError::OutOfOrder {
            expected: 2,
            actual: 1
        }
    ));
    let mut execution = incomplete.into_state();
    assert_eq!(execution.primary_cursor(), 1);
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(53))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session
                    .canonical_sequence(canonical)
                    .and_then(|view| view.get(0))
                    == Some(47)
                    && session
                        .outcome_evidence_sequence(evidence)
                        .and_then(|view| view.get(0))
                        == Some(53)
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("completed primary must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("completed primary must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    drop(source.canonical);
    drop(source.evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn same_account_shares_preserve_physical_storage_across_primary_and_failure_tail() {
    let factory = factory(1);
    let account = account(&factory);
    let primary_source = publish_transcript_source(&account);
    let failure_source = publish_transcript_source(&account);
    let (operation, keys) = derived_transcript_operation(&primary_source.canonical);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence_share(&primary_source.canonical, primary_source.canonical_root)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence_share(
            &primary_source.evidence,
            primary_source.evidence_root,
        )
        .unwrap();
    transcript.begin_failure_continuation().unwrap();
    let failure_ticket = transcript
        .plan_outcome_evidence_sequence_share(
            &failure_source.evidence,
            failure_source.evidence_root,
        )
        .unwrap();
    let failure_root_ticket = transcript
        .plan_outcome_evidence_records(1, 2, EvidenceRecordLeafTag)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let after_open = account.snapshot();
    let canonical_root = execution
        .try_execute_canonical_sequence_share(&primary_source.canonical, &canonical_ticket)
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence_share(&evidence_ticket)
        .unwrap();
    assert_eq!(execution.primary_cursor(), 2);
    assert_eq!(account.snapshot(), after_open);
    let mut failure = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |_session, _canonical, _evidence| false,
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must expose the planned share tail"),
    };
    let shared_failure_leaf = failure.try_execute_sequence_share(&failure_ticket).unwrap();
    assert_eq!(account.snapshot(), after_open);
    let failure_root = failure
        .try_execute_records(&failure_root_ticket, |writer| {
            writer.write_row(|row| {
                row.write_sequence_ref(evidence_root)?;
                row.write_sequence_ref(shared_failure_leaf)
            })
        })
        .unwrap();
    assert!(failure.is_complete());
    let validated = match failure
        .try_validate_rich_failure(failure_root, |session, root| {
            let Some(records) = session.records(root) else {
                return false;
            };
            let Some(preserved) = records.sequence_ref::<u8, EvidenceRootTag>(0, 0) else {
                return false;
            };
            let Some(shared) = records.sequence_ref::<u8, EvidenceRootTag>(0, 1) else {
                return false;
            };
            session.sequence(preserved).and_then(|view| view.get(0)) == Some(7)
                && session.sequence(shared).and_then(|view| view.get(0)) == Some(7)
        })
        .unwrap()
    {
        StoreTranscriptFailureValidation::Valid(validated) => validated,
        _ => panic!("shared failure evidence must validate"),
    };
    let prepared = match validated.try_prepare(103, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated shared evidence must remain rich")
        }
    };
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    let view = outcome_evidence.outcome_evidence();
    let records = view.records(failure_root).unwrap();
    let published_shared = records.sequence_ref::<u8, EvidenceRootTag>(0, 1).unwrap();
    assert_eq!(view.sequence(published_shared).unwrap().get(0), Some(7));
    drop(outcome_evidence);
    drop(ledger_evidence);
    drop(primary_source.canonical);
    drop(primary_source.evidence);
    drop(failure_source.canonical);
    drop(failure_source.evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn duplicate_canonical_share_ticket_is_an_ordinary_runtime_rejection() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_transcript_source(&account);
    let sibling = publish_transcript_source(&account);
    let (operation, keys) = derived_transcript_operation(&source.canonical);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let first = transcript
        .plan_canonical_sequence_share(&source.canonical, source.canonical_root)
        .unwrap();
    let duplicate = transcript
        .plan_canonical_sequence_share(&source.canonical, source.canonical_root)
        .unwrap();
    let _evidence = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let before_wrong_source = account.snapshot();
    assert!(matches!(
        execution.try_execute_canonical_sequence_share(&sibling.canonical, &first),
        Err(StoreExecutionError::Runtime(
            StoreRuntimeError::InvalidSourceSlot
        ))
    ));
    assert_eq!(execution.primary_cursor(), 0);
    assert_eq!(account.snapshot(), before_wrong_source);
    let _ = execution
        .try_execute_canonical_sequence_share(&source.canonical, &first)
        .unwrap();
    let before_duplicate = account.snapshot();
    assert!(matches!(
        execution.try_execute_canonical_sequence_share(&source.canonical, &duplicate),
        Err(StoreExecutionError::Runtime(
            StoreRuntimeError::InvalidSourceSlot
        ))
    ));
    assert_eq!(account.snapshot(), before_duplicate);

    drop(execution.prepare_minimal_fallback().unwrap().handoff());
    drop(source.canonical);
    drop(source.evidence);
    drop(sibling.canonical);
    drop(sibling.evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sequence_batch_counts_many_blocks_as_one_instruction() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let first = [2_u16, 3];
    let second = [5_u16, 7, 11];
    let items = [
        SequenceBatchItem::new(&first).unwrap(),
        SequenceBatchItem::new(&second).unwrap(),
    ];
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let batch_ticket = transcript
        .plan_canonical_sequence_batch(SequenceBatchPlan::new(&items).unwrap(), CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    assert_eq!(transcript.instruction_count(), 2);

    let mut execution = transcript.try_open().unwrap();
    let range = execution
        .try_execute_canonical_sequence_batch(&batch_ticket)
        .unwrap();
    assert_eq!(range.len(), 2);
    assert_eq!(execution.primary_cursor(), 1);
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(13))
        .unwrap();
    let canonical_root = range.get(0).unwrap();
    let validated = match execution
        .try_validate_success(canonical_root, evidence_root, |session, _, evidence| {
            let Some(first_ref) = range.get(0) else {
                return false;
            };
            let Some(second_ref) = range.get(1) else {
                return false;
            };
            session
                .canonical_sequence(first_ref)
                .is_some_and(|view| view.as_slice() == first)
                && session
                    .canonical_sequence(second_ref)
                    .is_some_and(|view| view.as_slice() == second)
                && session
                    .outcome_evidence_sequence(evidence)
                    .and_then(|view| view.get(0))
                    == Some(13)
        })
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("batch-backed graph must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("batch-backed graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(
        canonical
            .sequence(range.get(1).unwrap())
            .unwrap()
            .as_slice(),
        second
    );
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_and_validator_panics_enter_terminal_containment() {
    let factory = factory(2);
    let builder_account = account(&factory);
    let (operation, keys) = operation(&builder_account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let _evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    assert!(matches!(
        execution.try_execute_canonical_sequence(
            &canonical_ticket,
            |_writer| -> Result<(), StoreBuildError> {
                panic!("contained transcript builder panic")
            },
        ),
        Err(StoreExecutionError::Contained {
            failed_instruction: 0,
            cause: StoreRuntimeError::BuilderPanicked,
        })
    ));
    assert!(matches!(
        execution.try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(17)),
        Err(StoreExecutionError::Abandoned {
            failed_instruction: 0
        })
    ));
    assert!(matches!(
        execution.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));

    let validator_account = account(&factory);
    let (operation, keys) = super::operation(&validator_account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(19))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(23))
        .unwrap();
    let fallback = match execution
        .try_validate_success(canonical_root, evidence_root, |_session, _, _| {
            panic!("contained transcript validator panic")
        })
        .unwrap()
    {
        StoreTranscriptSuccessValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("validator panic must expose only terminal fallback"),
    };
    let prepared = fallback.prepare_minimal_fallback().unwrap();
    let PublishedStoreFallbackParts {
        ledger_evidence, ..
    } = prepared.handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    drop(ledger_evidence);
    assert_eq!(validator_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(validator_account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn writer_and_commit_keys_are_qualified_before_they_become_irreversible() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let limits = RetainedStoreBatchLimits::new(8, 4).unwrap();
    let invalid_phase_writer =
        RetainedStoreWriterPlan::new(foreign_phase(), keys.dimension, keys.dimension, limits);
    let start_error = match operation
        .try_begin_success_transcript(invalid_phase_writer, transcript_writer(keys))
    {
        Ok(_) => panic!("foreign writer phase must fail before transcript identity is issued"),
        Err(error) => error,
    };
    assert!(matches!(
        start_error.cause(),
        StoreTranscriptStartCause::Runtime(StoreRuntimeError::ForeignPhaseDomain { phase })
            if *phase == foreign_phase()
    ));
    let operation = start_error.into_operation();

    let unknown_dimension_writer = RetainedStoreWriterPlan::new(
        keys.phase,
        keys.other_dimension,
        keys.other_dimension,
        limits,
    );
    let start_error = match operation
        .try_begin_success_transcript(unknown_dimension_writer, transcript_writer(keys))
    {
        Ok(_) => panic!("unknown writer dimension must fail before transcript identity is issued"),
        Err(error) => error,
    };
    assert!(matches!(
        start_error.cause(),
        StoreTranscriptStartCause::Runtime(StoreRuntimeError::InvalidPublicationPlan)
    ));
    let operation = start_error.into_operation();

    let invalid_evidence_writer =
        RetainedStoreWriterPlan::new(foreign_phase(), keys.dimension, keys.dimension, limits);
    let start_error = match operation.try_begin_rich_failure_transcript(invalid_evidence_writer) {
        Ok(_) => {
            panic!("foreign evidence writer must fail before transcript identity is issued")
        }
        Err(error) => error,
    };
    assert!(matches!(
        start_error.cause(),
        StoreTranscriptStartCause::Runtime(StoreRuntimeError::ForeignPhaseDomain { phase })
            if *phase == foreign_phase()
    ));
    let operation = start_error.into_operation();

    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    assert!(matches!(
        transcript.plan_canonical_commit(foreign_dimension(), 1),
        Err(StoreTranscriptPlanError::Runtime(
            StoreRuntimeError::ForeignDimensionDomain { dimension }
        )) if dimension == foreign_dimension()
    ));
    assert_eq!(transcript.instruction_count(), 0);
    assert!(matches!(
        transcript.plan_outcome_evidence_commit(keys.other_dimension, 1),
        Err(StoreTranscriptPlanError::Runtime(StoreRuntimeError::Ledger(
            LedgerCause::UnknownDimension { dimension }
        ))) if dimension == keys.other_dimension
    ));
    assert_eq!(transcript.instruction_count(), 0);

    let commit_ticket = transcript.plan_canonical_commit(keys.dimension, 3).unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    execution
        .try_execute_canonical_commit(&commit_ticket)
        .unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(29))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(31))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session.canonical_sequence(canonical).is_some()
                    && session.outcome_evidence_sequence(evidence).is_some()
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("planner must remain usable after rejected keys"),
    };
    drop(validated.prepare_minimal_fallback().unwrap().handoff());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn share_planning_rejects_foreign_evidence_accounts_and_wrong_canonical_source_slots() {
    let factory = factory(2);
    let source_account = account(&factory);
    let foreign_account = account(&factory);
    let source = publish_transcript_source(&source_account);
    let foreign_source = publish_transcript_source(&foreign_account);

    let (foreign_operation, foreign_keys) = operation(&foreign_account);
    let mut foreign_transcript = foreign_operation
        .try_begin_success_transcript(
            transcript_writer(foreign_keys),
            transcript_writer(foreign_keys),
        )
        .unwrap();
    assert!(matches!(
        foreign_transcript
            .plan_outcome_evidence_sequence_share(&source.evidence, source.evidence_root,),
        Err(StoreTranscriptPlanError::Runtime(
            StoreRuntimeError::ForeignAccount
        ))
    ));
    drop(
        foreign_transcript
            .prepare_minimal_fallback(101)
            .unwrap()
            .handoff(),
    );

    let other_source = publish_transcript_source(&source_account);
    let (operation, keys) = derived_transcript_operation(&source.canonical);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    assert!(matches!(
        transcript
            .plan_canonical_sequence_share(&other_source.canonical, other_source.canonical_root,),
        Err(StoreTranscriptPlanError::Runtime(
            StoreRuntimeError::InvalidSourceSlot
        ))
    ));
    assert_eq!(transcript.instruction_count(), 0);
    assert!(matches!(
        transcript.plan_canonical_sequence_share(
            &foreign_source.canonical,
            foreign_source.canonical_root,
        ),
        Err(StoreTranscriptPlanError::Runtime(
            StoreRuntimeError::InvalidSourceSlot
        ))
    ));
    assert_eq!(transcript.instruction_count(), 0);
    drop(transcript.prepare_minimal_fallback(102).unwrap().handoff());

    drop(source.canonical);
    drop(source.evidence);
    drop(other_source.canonical);
    drop(other_source.evidence);
    drop(foreign_source.canonical);
    drop(foreign_source.evidence);
    assert_eq!(source_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(source_account.snapshot().live_retained_allocations(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn incomplete_derived_open_preserves_exact_source_authority_and_existing_tickets() {
    let factory = factory(1);
    let account = account(&factory);
    let source_a = publish_transcript_source(&account);
    let source_b = publish_transcript_source(&account);
    let (operation, keys) = derived_transcript_operation(&source_a.canonical);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence_share(&source_a.canonical, source_a.canonical_root)
        .unwrap();

    let open_error = match transcript.try_open() {
        Ok(_) => panic!("success publication without evidence must not open"),
        Err(error) => error,
    };
    let mut recovery = open_error
        .into_disposition()
        .expect("incomplete derived transcript must preserve retry authority");
    assert!(recovery.is_retryable());
    assert!(matches!(
        recovery.plan_canonical_sequence_share(&source_b.canonical, source_b.canonical_root),
        Err(StoreTranscriptPlanError::Runtime(
            StoreRuntimeError::InvalidSourceSlot
        ))
    ));
    let evidence_ticket = recovery
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();

    let mut execution = recovery.retry_open().unwrap();
    let _shared = execution
        .try_execute_canonical_sequence_share(&source_a.canonical, &canonical_ticket)
        .expect("recovered transcript must retain its exact source authority");
    let _evidence = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(17))
        .unwrap();
    drop(execution.prepare_minimal_fallback().unwrap().handoff());

    drop(source_a.canonical);
    drop(source_a.evidence);
    drop(source_b.canonical);
    drop(source_b.evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn incomplete_open_returns_the_same_transcript_for_repair() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();

    let open_error = match transcript.try_open() {
        Ok(_) => panic!("success publication without evidence must not open"),
        Err(error) => error,
    };
    assert!(matches!(
        open_error.cause(),
        StoreRuntimeError::InvalidPublicationPlan
    ));
    let mut recovery = open_error
        .into_disposition()
        .expect("structural plan rejection must preserve retry authority");
    assert!(recovery.is_retryable());
    let evidence_ticket = recovery
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();

    let mut execution = recovery.retry_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(37))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(41))
        .unwrap();
    let validated = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |session, canonical, evidence| {
                session.canonical_sequence(canonical).is_some()
                    && session.outcome_evidence_sequence(evidence).is_some()
            },
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Valid(validated) => validated,
        _ => panic!("repaired transcript must validate"),
    };
    drop(validated.prepare_minimal_fallback().unwrap().handoff());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn failure_tail_builder_panic_latches_containment() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_success_transcript(transcript_writer(keys), transcript_writer(keys))
        .unwrap();
    let canonical_ticket = transcript
        .plan_canonical_sequence::<u32, _>(1, CanonicalRootTag)
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    transcript.begin_failure_continuation().unwrap();
    let failure_ticket = transcript
        .plan_outcome_evidence_utf8(4, EvidenceTextTag)
        .unwrap();

    let mut execution = transcript.try_open().unwrap();
    let canonical_root = execution
        .try_execute_canonical_sequence(&canonical_ticket, |writer| writer.push(43))
        .unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(47))
        .unwrap();
    let mut failure = match execution
        .try_validate_success(
            canonical_root,
            evidence_root,
            |_session, _canonical, _evidence| false,
        )
        .unwrap()
    {
        StoreTranscriptSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must enter the failure tail"),
    };

    assert!(matches!(
        failure.try_execute_utf8(&failure_ticket, |_writer| -> Result<(), StoreBuildError> {
            panic!("contained failure-tail builder panic")
        },),
        Err(StoreExecutionError::Contained {
            failed_instruction: 2,
            cause: StoreRuntimeError::BuilderPanicked,
        })
    ));
    assert!(matches!(
        failure.try_execute_utf8(&failure_ticket, |writer| writer.write_str("retry")),
        Err(StoreExecutionError::Abandoned {
            failed_instruction: 2
        })
    ));
    assert!(matches!(
        failure.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));
}

#[test]
fn failure_validator_panic_downgrades_to_minimal_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut transcript = operation
        .try_begin_rich_failure_transcript(transcript_writer(keys))
        .unwrap();
    let evidence_ticket = transcript
        .plan_outcome_evidence_sequence::<u8, _>(1, EvidenceRootTag)
        .unwrap();
    let mut execution = transcript.try_open().unwrap();
    let evidence_root = execution
        .try_execute_outcome_evidence_sequence(&evidence_ticket, |writer| writer.push(53))
        .unwrap();

    let fallback = match execution
        .try_validate_rich_failure(evidence_root, |_session, _evidence| {
            panic!("contained failure validator panic")
        })
        .unwrap()
    {
        StoreTranscriptFailureValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("failure validator panic must expose only terminal fallback"),
    };
    let prepared = fallback.prepare_minimal_fallback().unwrap();
    let PublishedStoreFallbackParts {
        ledger_evidence, ..
    } = prepared.handoff();
    assert_eq!(ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(ledger_evidence.header().active_reservations(), 0);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
