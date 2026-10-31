use super::*;

#[test]
fn sequence_bytes_utf8_and_records_round_trip_through_typed_roots() {
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
    operation.try_complete_phase(keys.phase).unwrap();
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 4, 1))
        .unwrap();

    let sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(3).unwrap(),
            SequenceTag,
            |writer| writer.extend_from_slice(&[1, 2, 3]),
        )
        .unwrap();
    let bytes = assembly
        .try_retain_canonical_bytes(BytesPlan::new(2).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[4, 5])
        })
        .unwrap();
    let text = assembly
        .try_retain_canonical_utf8(Utf8Plan::new(5).unwrap(), TextTag, |writer| {
            writer.write_str("baozi")
        })
        .unwrap();
    let canonical_root = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 4).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| {
                row.write_scalar(9_u32)?;
                row.write_sequence_ref(sequence)?;
                row.write_bytes_ref(bytes)?;
                row.write_utf8_ref(text)
            })
        })
        .unwrap();
    let evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(7_u8)),
        )
        .unwrap();

    let before_rejected_commits = account.snapshot();
    assert!(matches!(
        assembly.try_commit_canonical(foreign_dimension(), 13),
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));
    assert!(matches!(
        assembly.try_commit_outcome_evidence(foreign_dimension(), 13),
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));
    assert_eq!(account.snapshot(), before_rejected_commits);

    assembly.try_commit_canonical(keys.dimension, 5).unwrap();
    assembly
        .try_commit_outcome_evidence(keys.dimension, 6)
        .unwrap();
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            let Some(root) = session.canonical_records(canonical_root) else {
                return false;
            };
            if root.scalar::<u32>(0, 0) != Some(9) {
                return false;
            }
            let Some(sequence): Option<CanonicalSequenceRef<u32, SequenceTag>> =
                root.sequence_ref(0, 1)
            else {
                return false;
            };
            let Some(bytes) = root.bytes_ref::<BytesTag>(0, 2) else {
                return false;
            };
            let Some(text) = root.utf8_ref::<TextTag>(0, 3) else {
                return false;
            };
            session
                .canonical_sequence(sequence)
                .map(|view| view.as_slice())
                == Some(&[1, 2, 3][..])
                && session.canonical_bytes(bytes) == Some(&[4, 5][..])
                && session.canonical_utf8(text) == Some("baozi")
                && session
                    .outcome_evidence_records(evidence_root)
                    .and_then(|view| view.scalar::<u8>(0, 0))
                    == Some(7)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("complete typed graph must validate"),
    };

    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("validated success must prepare as success"),
    };
    drop(producer);
    assert!(
        producer_unloaded.upgrade().is_none(),
        "prepared success must not retain executable producer state"
    );
    let physical_before_handoff = account.snapshot();
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(account.snapshot(), physical_before_handoff);
    let root = canonical.root();
    assert_eq!(root.scalar::<u32>(0, 0), Some(9));
    let sequence = root
        .sequence_ref::<u32, SequenceTag>(0, 1)
        .expect("typed sequence reference");
    assert_eq!(canonical.sequence(sequence).unwrap().as_slice(), &[1, 2, 3]);
    assert_eq!(outcome_evidence.root().scalar::<u8>(0, 0), Some(7));
    drop(canonical);
    drop(outcome_evidence);

    let header = ledger_evidence.header();
    assert_eq!(header.schema_revision(), 1);
    assert_eq!(header.terminal_cause(), StoreTerminalCause::Success);
    assert_eq!(
        header.last_completed_phase(),
        Some(keys.phase.semantic_id())
    );
    assert_eq!(header.dimension_count(), 2);
    assert_eq!(header.phase_count(), 1);
    assert_eq!(header.current_live_bytes(), 0);
    assert_eq!(header.active_reservations(), 0);
    assert_eq!(header.current_evidence_bytes(), 0);
    assert_semantic_and_ordinal_evidence_queries(&ledger_evidence, keys);
    let snapshot = ledger_evidence
        .dimension(keys.dimension)
        .expect("published dimension evidence");
    assert_eq!(snapshot.committed_canonical_output(), 5);
    assert_eq!(snapshot.published_outcome_evidence(), 7);
    let phase = ledger_evidence
        .phase(keys.phase)
        .expect("published phase evidence");
    assert_eq!(phase.phase(), keys.phase.semantic_id());
    assert_eq!(phase.current_live_bytes(), 0);
    assert!(matches!(
        ledger_evidence.dimension(foreign_dimension()),
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));
    assert!(matches!(
        ledger_evidence.dimension(keys.other_dimension),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension }))
            if dimension == keys.other_dimension
    ));
    assert!(matches!(
        ledger_evidence.phase(foreign_phase()),
        Err(StoreRuntimeError::ForeignPhaseDomain { phase }) if phase == foreign_phase()
    ));
    assert!(matches!(
        ledger_evidence.phase(keys.other_phase),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))
    ));
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sequence_bytes_and_utf8_roots_round_trip_through_every_terminal_owner() {
    let factory = factory(1);
    let account = account(&factory);

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(publication_plan(keys, 1, 1))
            .unwrap();
        let canonical = assembly
            .try_retain_canonical_sequence(
                SequencePlan::<u16>::new(2).unwrap(),
                SequenceTag,
                |writer| writer.extend_from_slice(&[1, 2]),
            )
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_sequence(
                SequencePlan::<u8>::new(2).unwrap(),
                EvidenceLeafTag,
                |writer| writer.extend_from_slice(&[3, 4]),
            )
            .unwrap();
        let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
            session.canonical_sequence(c).is_some()
                && session.outcome_evidence_sequence(e).is_some()
        }) {
            StoreSuccessValidation::Valid(validated) => validated,
            _ => panic!("sequence roots must validate"),
        };
        let prepared = match validated.try_prepare(final_progress()).unwrap() {
            PreparedStoreSuccessOutcome::Success(prepared) => prepared,
            _ => panic!("sequence roots must prepare success"),
        };
        let published = prepared.handoff();
        assert_eq!(published.canonical.root().as_slice(), &[1, 2]);
        assert_eq!(published.outcome_evidence.root().as_slice(), &[3, 4]);
        drop(published);
    }

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(publication_plan(keys, 1, 1))
            .unwrap();
        let canonical = assembly
            .try_retain_canonical_bytes(BytesPlan::new(2).unwrap(), BytesTag, |writer| {
                writer.write_bytes(&[5, 6])
            })
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_bytes(
                BytesPlan::new(3).unwrap(),
                EvidenceBytesTag,
                |writer| writer.write_bytes(&[7, 8, 9]),
            )
            .unwrap();
        let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
            session.canonical_bytes(c).is_some() && session.outcome_evidence_bytes(e).is_some()
        }) {
            StoreSuccessValidation::Valid(validated) => validated,
            _ => panic!("byte roots must validate"),
        };
        let prepared = match validated.try_prepare(final_progress()).unwrap() {
            PreparedStoreSuccessOutcome::Success(prepared) => prepared,
            _ => panic!("byte roots must prepare success"),
        };
        let published = prepared.handoff();
        assert_eq!(published.canonical.root(), &[5, 6]);
        assert_eq!(published.outcome_evidence.root(), &[7, 8, 9]);
        drop(published);
    }

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(publication_plan(keys, 1, 1))
            .unwrap();
        let canonical = assembly
            .try_retain_canonical_utf8(Utf8Plan::new(5).unwrap(), TextTag, |writer| {
                writer.write_str("baozi")
            })
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_utf8(
                Utf8Plan::new(2).unwrap(),
                EvidenceTextTag,
                |writer| writer.write_str("ok"),
            )
            .unwrap();
        let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
            session.canonical_utf8(c).is_some() && session.outcome_evidence_utf8(e).is_some()
        }) {
            StoreSuccessValidation::Valid(validated) => validated,
            _ => panic!("UTF-8 roots must validate"),
        };
        let prepared = match validated.try_prepare(final_progress()).unwrap() {
            PreparedStoreSuccessOutcome::Success(prepared) => prepared,
            _ => panic!("UTF-8 roots must prepare success"),
        };
        let published = prepared.handoff();
        assert_eq!(published.canonical.root(), "baozi");
        assert_eq!(published.outcome_evidence.root(), "ok");
        drop(published);
    }

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(rich_failure_plan(keys, 1))
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_sequence(
                SequencePlan::<u32>::new(2).unwrap(),
                EvidenceLeafTag,
                |writer| writer.extend_from_slice(&[10, 11]),
            )
            .unwrap();
        let validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
            session.sequence(evidence).is_some()
        }) {
            StoreFailureValidation::Valid(validated) => validated,
            _ => panic!("rich-failure sequence root must validate"),
        };
        let prepared = match validated.try_prepare(41, final_progress()).unwrap() {
            PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
            PreparedStoreFailureOutcome::MinimalFallback(_) => {
                panic!("rich-failure sequence root must remain available")
            }
        };
        let published = prepared.handoff();
        assert_eq!(
            published
                .outcome_evidence
                .outcome_evidence()
                .root()
                .as_slice(),
            &[10, 11]
        );
        drop(published);
    }

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(rich_failure_plan(keys, 1))
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_bytes(
                BytesPlan::new(2).unwrap(),
                EvidenceBytesTag,
                |writer| writer.write_bytes(&[12, 13]),
            )
            .unwrap();
        let validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
            session.bytes(evidence).is_some()
        }) {
            StoreFailureValidation::Valid(validated) => validated,
            _ => panic!("rich-failure byte root must validate"),
        };
        let prepared = match validated.try_prepare(42, final_progress()).unwrap() {
            PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
            PreparedStoreFailureOutcome::MinimalFallback(_) => {
                panic!("rich-failure byte root must remain available")
            }
        };
        let published = prepared.handoff();
        assert_eq!(
            published.outcome_evidence.outcome_evidence().root(),
            &[12, 13]
        );
        drop(published);
    }

    {
        let (operation, keys) = operation(&account);
        let mut assembly = operation
            .try_publication_assembly(rich_failure_plan(keys, 1))
            .unwrap();
        let evidence = assembly
            .try_retain_outcome_evidence_utf8(
                Utf8Plan::new(5).unwrap(),
                EvidenceTextTag,
                |writer| writer.write_str("error"),
            )
            .unwrap();
        let validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
            session.utf8(evidence).is_some()
        }) {
            StoreFailureValidation::Valid(validated) => validated,
            _ => panic!("rich-failure UTF-8 root must validate"),
        };
        let prepared = match validated.try_prepare(43, final_progress()).unwrap() {
            PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
            PreparedStoreFailureOutcome::MinimalFallback(_) => {
                panic!("rich-failure UTF-8 root must remain available")
            }
        };
        let published = prepared.handoff();
        assert_eq!(
            published.outcome_evidence.outcome_evidence().root(),
            "error"
        );
        drop(published);
    }

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn record_reference_projection_requires_exact_block_kind_and_tag() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 5, 5))
        .unwrap();

    let sequence = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let bytes = assembly
        .try_retain_canonical_bytes(BytesPlan::new(1).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[2])
        })
        .unwrap();
    let text = assembly
        .try_retain_canonical_utf8(Utf8Plan::new(1).unwrap(), TextTag, |writer| {
            writer.write_str("c")
        })
        .unwrap();
    let records = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 1).unwrap(), RecordLeafTag, |writer| {
            writer.write_row(|row| row.write_scalar(3_u8))
        })
        .unwrap();
    let canonical_root = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 4).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| {
                row.write_sequence_ref(sequence)?;
                row.write_bytes_ref(bytes)?;
                row.write_utf8_ref(text)?;
                row.write_record_ref(records)
            })
        })
        .unwrap();

    let evidence_sequence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |writer| writer.push(4),
        )
        .unwrap();
    let evidence_bytes = assembly
        .try_retain_outcome_evidence_bytes(BytesPlan::new(1).unwrap(), EvidenceBytesTag, |writer| {
            writer.write_bytes(&[5])
        })
        .unwrap();
    let evidence_text = assembly
        .try_retain_outcome_evidence_utf8(Utf8Plan::new(1).unwrap(), EvidenceTextTag, |writer| {
            writer.write_str("e")
        })
        .unwrap();
    let evidence_records = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRecordLeafTag,
            |writer| writer.write_row(|row| row.write_scalar(6_u8)),
        )
        .unwrap();
    let evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 4).unwrap(),
            EvidenceRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(evidence_sequence)?;
                    row.write_bytes_ref(evidence_bytes)?;
                    row.write_utf8_ref(evidence_text)?;
                    row.write_record_ref(evidence_records)
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
            assert!(
                canonical_root
                    .sequence_ref::<u8, SequenceTag>(0, 0)
                    .is_some()
            );
            assert!(canonical_root.sequence_ref::<u8, WrongTag>(0, 0).is_none());
            assert!(canonical_root.bytes_ref::<SequenceTag>(0, 0).is_none());
            assert!(canonical_root.utf8_ref::<SequenceTag>(0, 0).is_none());
            assert!(canonical_root.record_ref::<SequenceTag>(0, 0).is_none());
            assert!(canonical_root.bytes_ref::<BytesTag>(0, 1).is_some());
            assert!(canonical_root.bytes_ref::<WrongTag>(0, 1).is_none());
            assert!(canonical_root.sequence_ref::<u8, BytesTag>(0, 1).is_none());
            assert!(canonical_root.utf8_ref::<BytesTag>(0, 1).is_none());
            assert!(canonical_root.record_ref::<BytesTag>(0, 1).is_none());
            assert!(canonical_root.utf8_ref::<TextTag>(0, 2).is_some());
            assert!(canonical_root.utf8_ref::<WrongTag>(0, 2).is_none());
            assert!(canonical_root.sequence_ref::<u8, TextTag>(0, 2).is_none());
            assert!(canonical_root.bytes_ref::<TextTag>(0, 2).is_none());
            assert!(canonical_root.record_ref::<TextTag>(0, 2).is_none());
            assert!(canonical_root.record_ref::<RecordLeafTag>(0, 3).is_some());
            assert!(canonical_root.record_ref::<WrongTag>(0, 3).is_none());
            assert!(
                canonical_root
                    .sequence_ref::<u8, RecordLeafTag>(0, 3)
                    .is_none()
            );
            assert!(canonical_root.bytes_ref::<RecordLeafTag>(0, 3).is_none());
            assert!(canonical_root.utf8_ref::<RecordLeafTag>(0, 3).is_none());

            let Some(evidence_root) = session.outcome_evidence_records(evidence_root) else {
                return false;
            };
            assert!(
                evidence_root
                    .sequence_ref::<u8, EvidenceLeafTag>(0, 0)
                    .is_some()
            );
            assert!(evidence_root.sequence_ref::<u8, WrongTag>(0, 0).is_none());
            assert!(evidence_root.bytes_ref::<EvidenceLeafTag>(0, 0).is_none());
            assert!(evidence_root.utf8_ref::<EvidenceLeafTag>(0, 0).is_none());
            assert!(evidence_root.record_ref::<EvidenceLeafTag>(0, 0).is_none());
            assert!(evidence_root.bytes_ref::<EvidenceBytesTag>(0, 1).is_some());
            assert!(evidence_root.bytes_ref::<WrongTag>(0, 1).is_none());
            assert!(
                evidence_root
                    .sequence_ref::<u8, EvidenceBytesTag>(0, 1)
                    .is_none()
            );
            assert!(evidence_root.utf8_ref::<EvidenceBytesTag>(0, 1).is_none());
            assert!(evidence_root.record_ref::<EvidenceBytesTag>(0, 1).is_none());
            assert!(evidence_root.utf8_ref::<EvidenceTextTag>(0, 2).is_some());
            assert!(evidence_root.utf8_ref::<WrongTag>(0, 2).is_none());
            assert!(
                evidence_root
                    .sequence_ref::<u8, EvidenceTextTag>(0, 2)
                    .is_none()
            );
            assert!(evidence_root.bytes_ref::<EvidenceTextTag>(0, 2).is_none());
            assert!(evidence_root.record_ref::<EvidenceTextTag>(0, 2).is_none());
            assert!(
                evidence_root
                    .record_ref::<EvidenceRecordLeafTag>(0, 3)
                    .is_some()
            );
            assert!(evidence_root.record_ref::<WrongTag>(0, 3).is_none());
            assert!(
                evidence_root
                    .sequence_ref::<u8, EvidenceRecordLeafTag>(0, 3)
                    .is_none()
            );
            assert!(
                evidence_root
                    .bytes_ref::<EvidenceRecordLeafTag>(0, 3)
                    .is_none()
            );
            assert!(
                evidence_root
                    .utf8_ref::<EvidenceRecordLeafTag>(0, 3)
                    .is_none()
            );

            session.canonical_sequence(sequence).is_some()
                && session.canonical_bytes(bytes).is_some()
                && session.canonical_utf8(text).is_some()
                && session.canonical_records(records).is_some()
                && session
                    .outcome_evidence_sequence(evidence_sequence)
                    .is_some()
                && session.outcome_evidence_bytes(evidence_bytes).is_some()
                && session.outcome_evidence_utf8(evidence_text).is_some()
                && session.outcome_evidence_records(evidence_records).is_some()
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("exact kind and tag references must validate"),
    };

    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("tag-safe graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    let canonical_root = canonical.root();
    assert!(
        canonical_root
            .sequence_ref::<u8, SequenceTag>(0, 0)
            .is_some()
    );
    assert!(canonical_root.sequence_ref::<u8, WrongTag>(0, 0).is_none());
    assert!(canonical_root.bytes_ref::<SequenceTag>(0, 0).is_none());
    assert!(canonical_root.utf8_ref::<SequenceTag>(0, 0).is_none());
    assert!(canonical_root.record_ref::<SequenceTag>(0, 0).is_none());
    assert!(canonical_root.bytes_ref::<BytesTag>(0, 1).is_some());
    assert!(canonical_root.bytes_ref::<WrongTag>(0, 1).is_none());
    assert!(canonical_root.sequence_ref::<u8, BytesTag>(0, 1).is_none());
    assert!(canonical_root.utf8_ref::<BytesTag>(0, 1).is_none());
    assert!(canonical_root.record_ref::<BytesTag>(0, 1).is_none());
    assert!(canonical_root.utf8_ref::<TextTag>(0, 2).is_some());
    assert!(canonical_root.utf8_ref::<WrongTag>(0, 2).is_none());
    assert!(canonical_root.sequence_ref::<u8, TextTag>(0, 2).is_none());
    assert!(canonical_root.bytes_ref::<TextTag>(0, 2).is_none());
    assert!(canonical_root.record_ref::<TextTag>(0, 2).is_none());
    assert!(canonical_root.record_ref::<RecordLeafTag>(0, 3).is_some());
    assert!(canonical_root.record_ref::<WrongTag>(0, 3).is_none());
    assert!(
        canonical_root
            .sequence_ref::<u8, RecordLeafTag>(0, 3)
            .is_none()
    );
    assert!(canonical_root.bytes_ref::<RecordLeafTag>(0, 3).is_none());
    assert!(canonical_root.utf8_ref::<RecordLeafTag>(0, 3).is_none());
    let evidence_root = outcome_evidence.root();
    assert!(
        evidence_root
            .sequence_ref::<u8, EvidenceLeafTag>(0, 0)
            .is_some()
    );
    assert!(evidence_root.sequence_ref::<u8, WrongTag>(0, 0).is_none());
    assert!(evidence_root.bytes_ref::<EvidenceLeafTag>(0, 0).is_none());
    assert!(evidence_root.utf8_ref::<EvidenceLeafTag>(0, 0).is_none());
    assert!(evidence_root.record_ref::<EvidenceLeafTag>(0, 0).is_none());
    assert!(evidence_root.bytes_ref::<EvidenceBytesTag>(0, 1).is_some());
    assert!(evidence_root.bytes_ref::<WrongTag>(0, 1).is_none());
    assert!(
        evidence_root
            .sequence_ref::<u8, EvidenceBytesTag>(0, 1)
            .is_none()
    );
    assert!(evidence_root.utf8_ref::<EvidenceBytesTag>(0, 1).is_none());
    assert!(evidence_root.record_ref::<EvidenceBytesTag>(0, 1).is_none());
    assert!(evidence_root.utf8_ref::<EvidenceTextTag>(0, 2).is_some());
    assert!(evidence_root.utf8_ref::<WrongTag>(0, 2).is_none());
    assert!(
        evidence_root
            .sequence_ref::<u8, EvidenceTextTag>(0, 2)
            .is_none()
    );
    assert!(evidence_root.bytes_ref::<EvidenceTextTag>(0, 2).is_none());
    assert!(evidence_root.record_ref::<EvidenceTextTag>(0, 2).is_none());
    assert!(
        evidence_root
            .record_ref::<EvidenceRecordLeafTag>(0, 3)
            .is_some()
    );
    assert!(evidence_root.record_ref::<WrongTag>(0, 3).is_none());
    assert!(
        evidence_root
            .sequence_ref::<u8, EvidenceRecordLeafTag>(0, 3)
            .is_none()
    );
    assert!(
        evidence_root
            .bytes_ref::<EvidenceRecordLeafTag>(0, 3)
            .is_none()
    );
    assert!(
        evidence_root
            .utf8_ref::<EvidenceRecordLeafTag>(0, 3)
            .is_none()
    );
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn every_supported_scalar_kind_round_trips_without_cross_kind_projection() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 11, 1))
        .unwrap();

    macro_rules! retain_scalar {
        ($type:ty, $value:expr, $tag:expr) => {
            assembly
                .try_retain_canonical_sequence(
                    SequencePlan::<$type>::new(1).unwrap(),
                    $tag,
                    |writer| writer.push($value),
                )
                .unwrap()
        };
    }

    let u8_slot = retain_scalar!(u8, 1_u8, ScalarTag::<1>);
    let i8_slot = retain_scalar!(i8, -2_i8, ScalarTag::<2>);
    let u16_slot = retain_scalar!(u16, 3_u16, ScalarTag::<3>);
    let i16_slot = retain_scalar!(i16, -4_i16, ScalarTag::<4>);
    let u32_slot = retain_scalar!(u32, 5_u32, ScalarTag::<5>);
    let i32_slot = retain_scalar!(i32, -6_i32, ScalarTag::<6>);
    let u64_slot = retain_scalar!(u64, 7_u64, ScalarTag::<7>);
    let i64_slot = retain_scalar!(i64, -8_i64, ScalarTag::<8>);
    let f32_slot = retain_scalar!(f32, 1.5_f32, ScalarTag::<9>);
    let f64_slot = retain_scalar!(f64, -2.5_f64, ScalarTag::<10>);
    let root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, 10).unwrap(),
            CanonicalRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(u8_slot)?;
                    row.write_sequence_ref(i8_slot)?;
                    row.write_sequence_ref(u16_slot)?;
                    row.write_sequence_ref(i16_slot)?;
                    row.write_sequence_ref(u32_slot)?;
                    row.write_sequence_ref(i32_slot)?;
                    row.write_sequence_ref(u64_slot)?;
                    row.write_sequence_ref(i64_slot)?;
                    row.write_sequence_ref(f32_slot)?;
                    row.write_sequence_ref(f64_slot)
                })
            },
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(1_u8)),
        )
        .unwrap();

    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        assert!(root.sequence_ref::<f32, ScalarTag<1>>(0, 0).is_none());
        macro_rules! visit_scalar {
            ($field:expr, $type:ty, $tag:ty, $expected:expr) => {{
                let Some(slot) = root.sequence_ref::<$type, $tag>(0, $field) else {
                    return false;
                };
                if session
                    .canonical_sequence(slot)
                    .and_then(|view| view.get(0))
                    != Some($expected)
                {
                    return false;
                }
            }};
        }
        visit_scalar!(0, u8, ScalarTag<1>, 1_u8);
        visit_scalar!(1, i8, ScalarTag<2>, -2_i8);
        visit_scalar!(2, u16, ScalarTag<3>, 3_u16);
        visit_scalar!(3, i16, ScalarTag<4>, -4_i16);
        visit_scalar!(4, u32, ScalarTag<5>, 5_u32);
        visit_scalar!(5, i32, ScalarTag<6>, -6_i32);
        visit_scalar!(6, u64, ScalarTag<7>, 7_u64);
        visit_scalar!(7, i64, ScalarTag<8>, -8_i64);
        visit_scalar!(8, f32, ScalarTag<9>, 1.5_f32);
        visit_scalar!(9, f64, ScalarTag<10>, -2.5_f64);
        session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("all scalar kinds must validate"),
    };

    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("scalar inventory must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(outcome_evidence);
    drop(ledger_evidence);
    let root = canonical.root();
    macro_rules! assert_scalar {
        ($field:expr, $type:ty, $tag:ty, $expected:expr) => {{
            let slot = root
                .sequence_ref::<$type, $tag>(0, $field)
                .expect("typed scalar slot");
            assert_eq!(
                canonical.sequence(slot).and_then(|view| view.get(0)),
                Some($expected)
            );
        }};
    }
    assert_scalar!(0, u8, ScalarTag<1>, 1_u8);
    assert_scalar!(1, i8, ScalarTag<2>, -2_i8);
    assert_scalar!(2, u16, ScalarTag<3>, 3_u16);
    assert_scalar!(3, i16, ScalarTag<4>, -4_i16);
    assert_scalar!(4, u32, ScalarTag<5>, 5_u32);
    assert_scalar!(5, i32, ScalarTag<6>, -6_i32);
    assert_scalar!(6, u64, ScalarTag<7>, 7_u64);
    assert_scalar!(7, i64, ScalarTag<8>, -8_i64);
    assert_scalar!(8, f32, ScalarTag<9>, 1.5_f32);
    assert_scalar!(9, f64, ScalarTag<10>, -2.5_f64);
    drop(canonical);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn exact_plans_reject_empty_excess_and_wrong_record_kinds() {
    assert!(matches!(
        SequencePlan::<u8>::new(0),
        Err(baozi_operation::StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        BytesPlan::new(0),
        Err(baozi_operation::StorePlanError::EmptyPayload)
    ));
    assert!(RecordPlan::new(1, 39).is_ok());
    assert!(RecordPlan::new(1, 64).is_ok());
    assert!(matches!(
        RecordPlan::new(1, 65),
        Err(baozi_operation::StorePlanError::InvalidRecordFieldCount {
            fields: 65,
            limit: 64,
        })
    ));

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 2, 1))
        .unwrap();
    let error = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.extend_from_slice(&[1, 2])
        })
        .unwrap_err();
    assert!(matches!(
        error,
        StoreRuntimeError::Build(cause)
            if cause.class() == StoreBuildFailureClass::CapacityExceeded
    ));

    let sequence = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let root = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 1).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| row.write_sequence_ref(sequence))
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(1_u8)),
        )
        .unwrap();
    let fallback = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        assert!(root.sequence_ref::<f32, SequenceTag>(0, 0).is_none());
        let Some(sequence) = root.sequence_ref::<u8, SequenceTag>(0, 0) else {
            return false;
        };
        session.canonical_sequence(sequence).is_some()
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated.prepare_minimal_fallback().unwrap(),
        _ => panic!("kind-safe graph must validate"),
    };
    drop(fallback.handoff());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn physical_snapshots_follow_capacity_envelopes_for_sequence_bytes_and_utf8() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 3, 1))
        .unwrap();

    let before_sequence = account.snapshot();
    let _sequence = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(3).unwrap(), SequenceTag, |writer| {
            writer.extend_from_slice(&[1, 2, 3])
        })
        .unwrap();
    let after_sequence = account.snapshot();

    let _bytes = assembly
        .try_retain_canonical_bytes(BytesPlan::new(4).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[1, 2, 3, 4])
        })
        .unwrap();
    let after_bytes = account.snapshot();

    let _text = assembly
        .try_retain_canonical_utf8(Utf8Plan::new(5).unwrap(), TextTag, |writer| {
            writer.write_str("baozi")
        })
        .unwrap();
    let after_utf8 = account.snapshot();

    let sequence_step = physical_delta(before_sequence, after_sequence);
    let bytes_step = physical_delta(after_sequence, after_bytes);
    let utf8_step = physical_delta(after_bytes, after_utf8);

    assert_eq!(sequence_step, bytes_step);
    assert_eq!(utf8_step.0, bytes_step.0 + 4);
    for (_, allocations) in [sequence_step, bytes_step, utf8_step] {
        assert_eq!(allocations, 2);
    }

    drop(assembly.prepare_minimal_fallback().unwrap().handoff());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
