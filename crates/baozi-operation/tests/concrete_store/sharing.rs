use super::*;

#[test]
fn same_account_share_is_zero_physical_growth_and_foreign_or_stale_refs_fail() {
    let factory = factory(2);
    let source_account = account(&factory);
    let foreign_account = account(&factory);
    let source = publish_sequence_source(&source_account);

    let (share_operation, keys) = operation(&source_account);
    let share_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
    );
    let mut assembly = share_operation
        .try_publication_assembly(share_plan)
        .unwrap();
    let physical_before_share = source_account.snapshot();
    let canonical_sequence = assembly
        .try_share_canonical(&source.canonical, source.canonical_sequence)
        .unwrap();
    let canonical_bytes = assembly
        .try_share_canonical(&source.canonical, source.canonical_bytes)
        .unwrap();
    let evidence_sequence = assembly
        .try_share_outcome_evidence(&source.evidence, source.evidence_sequence)
        .unwrap();
    let evidence_text = assembly
        .try_share_outcome_evidence(&source.evidence, source.evidence_text)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(assembly.outcome_evidence_len(), 2);
    assert_eq!(source_account.snapshot(), physical_before_share);

    let canonical_root = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 2).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| {
                row.write_sequence_ref(canonical_sequence)?;
                row.write_bytes_ref(canonical_bytes)
            })
        })
        .unwrap();
    let evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 2).unwrap(),
            EvidenceRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(evidence_sequence)?;
                    row.write_utf8_ref(evidence_text)
                })
            },
        )
        .unwrap();
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            let Some(canonical_root) = session.canonical_records(canonical_root) else {
                return false;
            };
            let Some(canonical_sequence) = canonical_root.sequence_ref::<u32, SequenceTag>(0, 0)
            else {
                return false;
            };
            let Some(canonical_bytes) = canonical_root.bytes_ref::<BytesTag>(0, 1) else {
                return false;
            };
            let Some(evidence_root) = session.outcome_evidence_records(evidence_root) else {
                return false;
            };
            let Some(evidence_sequence) = evidence_root.sequence_ref::<u8, EvidenceLeafTag>(0, 0)
            else {
                return false;
            };
            let Some(evidence_text) = evidence_root.utf8_ref::<EvidenceTextTag>(0, 1) else {
                return false;
            };
            session
                .canonical_sequence(canonical_sequence)
                .and_then(|view| view.get(0))
                == Some(11)
                && session.canonical_bytes(canonical_bytes) == Some(&[12, 13][..])
                && session
                    .outcome_evidence_sequence(evidence_sequence)
                    .and_then(|view| view.get(0))
                    == Some(7)
                && session.outcome_evidence_utf8(evidence_text) == Some("ok")
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("shared graph must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("shared graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical: shared_canonical,
        outcome_evidence: shared_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    assert!(
        shared_canonical
            .sequence(source.canonical_sequence)
            .is_none()
    );
    assert!(shared_canonical.bytes(source.canonical_bytes).is_none());
    assert_eq!(
        shared_canonical
            .sequence(canonical_sequence)
            .unwrap()
            .get(0),
        Some(11)
    );
    assert_eq!(shared_canonical.bytes(canonical_bytes), Some(&[12, 13][..]));
    assert!(shared_evidence.sequence(source.evidence_sequence).is_none());
    assert!(shared_evidence.utf8(source.evidence_text).is_none());
    assert_eq!(
        shared_evidence.sequence(evidence_sequence).unwrap().get(0),
        Some(7)
    );
    assert_eq!(shared_evidence.utf8(evidence_text), Some("ok"));

    let (stale_operation, keys) = operation(&source_account);
    let mut stale_root = stale_operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let _local_canonical = stale_root
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |writer| writer.push(12),
        )
        .unwrap();
    let local_evidence = stale_root
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(8_u8)),
        )
        .unwrap();
    let fallback_only = match stale_root.validate_success(
        source.canonical_sequence,
        local_evidence,
        |_session, _, _| panic!("invalid root must reject before the validator callback"),
    ) {
        StoreSuccessValidation::FallbackOnly(fallback_only) => fallback_only,
        _ => panic!("a stale root must transition directly to fallback-only"),
    };
    drop(fallback_only.prepare_minimal_fallback().unwrap().handoff());

    let (operation, foreign_keys) = operation(&foreign_account);
    let mut foreign = operation
        .try_publication_assembly(RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(writer_plan(foreign_keys, 1, 2), 1, 0).unwrap(),
            RetainedStoreBlockTablePlan::new(writer_plan(foreign_keys, 1, 2), 1, 0).unwrap(),
        ))
        .unwrap();
    assert!(matches!(
        foreign.try_share_canonical(&source.canonical, source.canonical_sequence),
        Err(StoreRuntimeError::ForeignAccount)
    ));
    drop(foreign.prepare_minimal_fallback().unwrap().handoff());
    drop(shared_canonical);
    drop(shared_evidence);
    drop(source);
    assert_eq!(source_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn same_account_owner_ref_mismatch_rejects_before_mutation_or_work_charge() {
    let factory = factory(1);
    let account = account(&factory);
    let source_a = publish_sequence_source(&account);
    let source_b = publish_sequence_source(&account);

    let target_plan = |keys| {
        RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
            RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
        )
    };
    let (challenged_operation, challenged_keys) = operation(&account);
    let mut challenged = challenged_operation
        .try_publication_assembly(target_plan(challenged_keys))
        .unwrap();
    let physical_before_invalid_shares = account.snapshot();
    assert!(matches!(
        challenged.try_share_canonical(&source_a.canonical, source_b.canonical_sequence),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        challenged.try_share_outcome_evidence(&source_a.evidence, source_b.evidence_sequence),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(challenged.canonical_len(), 0);
    assert_eq!(challenged.outcome_evidence_len(), 0);
    assert_eq!(account.snapshot(), physical_before_invalid_shares);

    let challenged_canonical = challenged
        .try_share_canonical(&source_a.canonical, source_a.canonical_sequence)
        .unwrap();
    let challenged_evidence = challenged
        .try_share_outcome_evidence(&source_a.evidence, source_a.evidence_sequence)
        .unwrap();
    let challenged = match challenged.validate_success(
        challenged_canonical,
        challenged_evidence,
        |session, canonical, evidence| {
            session
                .canonical_sequence(canonical)
                .and_then(|view| view.get(0))
                == Some(11)
                && session
                    .outcome_evidence_sequence(evidence)
                    .and_then(|view| view.get(0))
                    == Some(7)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("legal shares after rejected mismatches must validate"),
    };
    let challenged = match challenged.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("challenged legal graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = challenged.handoff();
    drop(canonical);
    drop(outcome_evidence);
    let challenged_work = ledger_evidence
        .dimension(challenged_keys.dimension)
        .expect("challenged work snapshot");
    drop(ledger_evidence);

    let (control_operation, control_keys) = operation(&account);
    let mut control = control_operation
        .try_publication_assembly(target_plan(control_keys))
        .unwrap();
    let control_canonical = control
        .try_share_canonical(&source_a.canonical, source_a.canonical_sequence)
        .unwrap();
    let control_evidence = control
        .try_share_outcome_evidence(&source_a.evidence, source_a.evidence_sequence)
        .unwrap();
    let control = match control.validate_success(
        control_canonical,
        control_evidence,
        |session, canonical, evidence| {
            session.canonical_sequence(canonical).is_some()
                && session.outcome_evidence_sequence(evidence).is_some()
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("control shared graph must validate"),
    };
    let control = match control.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("control shared graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = control.handoff();
    drop(canonical);
    drop(outcome_evidence);
    let control_work = ledger_evidence
        .dimension(control_keys.dimension)
        .expect("control work snapshot");
    assert_eq!(challenged_work.attempted(), control_work.attempted());
    assert_eq!(challenged_work.consumed(), control_work.consumed());
    drop(ledger_evidence);

    drop(source_a);
    drop(source_b);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
