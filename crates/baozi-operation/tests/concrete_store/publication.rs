use super::*;

#[test]
fn final_cancellation_downgrades_validated_success_before_pure_handoff() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
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
    assembly
        .try_commit_outcome_evidence(keys.dimension, 6)
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some() && session.outcome_evidence_records(e).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("the candidate graph must validate before final cancellation"),
    };

    cancellation.cancel();
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::RichFailure(prepared) => prepared,
        _ => panic!("final cancellation must prepare rich failure evidence"),
    };
    let physical_before_handoff = account.snapshot();
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(account.snapshot(), physical_before_handoff);
    assert_eq!(
        store_failure_cause(&ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    assert_eq!(store_failure_cause(&ledger_evidence).code(), 1);
    assert_eq!(
        outcome_evidence
            .outcome_evidence()
            .root()
            .scalar::<u8>(0, 0),
        Some(7)
    );
    let dimension = ledger_evidence
        .dimension(keys.dimension)
        .expect("final cancellation evidence");
    assert_eq!(dimension.committed_canonical_output(), 0);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn validated_success_downgrade_preserves_evidence_and_publishes_exact_rich_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 3))
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
    assembly
        .try_commit_outcome_evidence(keys.dimension, 6)
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some()
            && session
                .outcome_evidence_records(e)
                .and_then(|records| records.scalar::<u8>(0, 0))
                == Some(7)
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("success candidate must validate before domain downgrade"),
    };

    let mut failure = validated.downgrade_to_rich_failure(73);
    assert_eq!(failure.evidence_root(), evidence);
    let detail = failure
        .try_retain_utf8(Utf8Plan::new(8).unwrap(), EvidenceTextTag, |writer| {
            writer.write_str("rejected")
        })
        .unwrap();
    let failure_root = failure
        .try_retain_records(
            RecordPlan::new(1, 2).unwrap(),
            EvidenceRecordLeafTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_record_ref(evidence)?;
                    row.write_utf8_ref(detail)
                })
            },
        )
        .unwrap();
    let before_rejected_commits = account.snapshot();
    assert!(matches!(
        failure.try_commit(foreign_dimension(), 11),
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));
    assert!(matches!(
        failure.try_commit(keys.other_dimension, 11),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension }))
            if dimension == keys.other_dimension
    ));
    assert_eq!(account.snapshot(), before_rejected_commits);
    failure.try_commit(keys.dimension, 4).unwrap();
    let validated = match failure.validate_rich_failure(failure_root, |session, failure_root| {
        let Some(root) = session.records(failure_root) else {
            return false;
        };
        let Some(preserved) = root.record_ref::<EvidenceRootTag>(0, 0) else {
            return false;
        };
        let Some(detail) = root.utf8_ref::<EvidenceTextTag>(0, 1) else {
            return false;
        };
        session
            .records(preserved)
            .and_then(|records| records.scalar::<u8>(0, 0))
            == Some(7)
            && session.utf8(detail) == Some("rejected")
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("augmented downgrade evidence must validate"),
    };
    let prepared = match validated.try_prepare(74, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated downgrade evidence must prepare rich failure")
        }
    };
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(store_failure_cause(&ledger_evidence).code(), 74);
    let evidence_view = outcome_evidence.outcome_evidence();
    let root = evidence_view.root();
    let preserved = root
        .record_ref::<EvidenceRootTag>(0, 0)
        .expect("preserved evidence reference");
    let detail = root
        .utf8_ref::<EvidenceTextTag>(0, 1)
        .expect("downgrade detail reference");
    assert_eq!(
        evidence_view
            .records(preserved)
            .and_then(|records| records.scalar::<u8>(0, 0)),
        Some(7)
    );
    assert_eq!(evidence_view.utf8(detail), Some("rejected"));
    let dimension = ledger_evidence
        .dimension(keys.dimension)
        .expect("downgrade ledger evidence");
    assert_eq!(dimension.committed_canonical_output(), 0);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
