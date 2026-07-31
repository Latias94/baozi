use super::*;

type RecordGraphCanonical =
    baozi_operation::PublishedCanonicalStore<baozi_operation::RecordBlock, CanonicalRootTag>;

struct PublishedRecordGraph {
    canonical: RecordGraphCanonical,
    canonical_sequence: CanonicalSequenceRef<u32, SequenceTag>,
    canonical_bytes: baozi_operation::CanonicalBytesRef<BytesTag>,
    canonical_child: baozi_operation::CanonicalRecordRef<RecordLeafTag>,
    _evidence: SourceEvidence,
}

struct PublishedSequenceGeneration {
    canonical: SourceCanonical,
    canonical_sequence: CanonicalSequenceRef<u32, SequenceTag>,
    canonical_bytes: baozi_operation::CanonicalBytesRef<BytesTag>,
    _evidence: SourceEvidence,
}

type DerivedTestOperation<'source> =
    baozi_operation::DerivedRetainedStoreOperation<'source, TestOwner, TestProgress, TestUnit>;

fn derived_operation<'source>(
    source: &'source SourceCanonical,
) -> (DerivedTestOperation<'source>, TestKeys) {
    derived_operation_with_control(source, OperationControl::default())
}

fn derived_operation_with_control<'source>(
    source: &'source SourceCanonical,
    control: OperationControl<TestProgress, TestUnit>,
) -> (DerivedTestOperation<'source>, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, 32, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    (
        TestOperation::try_start_derived(control, ledger_plan, source, start).unwrap(),
        keys,
    )
}

#[derive(Default)]
struct MidInheritanceCancellationState {
    active: std::sync::atomic::AtomicBool,
    active_polls: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Default)]
struct MidInheritanceCancellation(Arc<MidInheritanceCancellationState>);

impl MidInheritanceCancellation {
    fn arm(&self) {
        self.0
            .active_polls
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.0
            .active
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl baozi_operation::CancellationSignal for MidInheritanceCancellation {
    fn is_cancelled(&self) -> bool {
        if !self.0.active.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        self.0
            .active_polls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            >= 2
    }
}

fn publish_record_graph_source(account: &PhysicalStorageAccount) -> PublishedRecordGraph {
    let (operation, keys) = operation(account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 4, 1))
        .unwrap();
    let sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(16).unwrap(),
            SequenceTag,
            |writer| writer.extend_from_slice(&[41; 16]),
        )
        .unwrap();
    let bytes = assembly
        .try_retain_canonical_bytes(BytesPlan::new(2).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[42, 43])
        })
        .unwrap();
    let child = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 3).unwrap(), RecordLeafTag, |writer| {
            writer.write_row(|row| {
                row.write_sequence_ref(sequence)?;
                row.write_bytes_ref(bytes)?;
                row.write_scalar(44_u64)
            })
        })
        .unwrap();
    let root = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 2).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| {
                row.write_record_ref(child)?;
                row.write_scalar(45_u32)
            })
        })
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
        let Some(child) = root.record_ref::<RecordLeafTag>(0, 0) else {
            return false;
        };
        let Some(child) = session.canonical_records(child) else {
            return false;
        };
        let Some(sequence) = child.sequence_ref::<u32, SequenceTag>(0, 0) else {
            return false;
        };
        let Some(bytes) = child.bytes_ref::<BytesTag>(0, 1) else {
            return false;
        };
        session
            .canonical_sequence(sequence)
            .and_then(|view| view.get(0))
            == Some(41)
            && session.canonical_bytes(bytes) == Some(&[42, 43][..])
            && child.scalar::<u64>(0, 2) == Some(44)
            && root.scalar::<u32>(0, 1) == Some(45)
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("record graph source must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("record graph source must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedRecordGraph {
        canonical,
        canonical_sequence: sequence,
        canonical_bytes: bytes,
        canonical_child: child,
        _evidence: outcome_evidence,
    }
}

fn publish_rewritten_record_graph(
    account: &PhysicalStorageAccount,
    source: &PublishedRecordGraph,
) -> PublishedRecordGraph {
    let (operation, keys) = derived_operation(&source.canonical);
    let before_tables = account.snapshot();
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 3, 2), 4, 2).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let after_tables = account.snapshot();
    assert_eq!(physical_delta(before_tables, after_tables).1, 5);

    let sequence = assembly
        .try_share_canonical(source.canonical_sequence)
        .unwrap();
    let bytes = assembly
        .try_share_canonical(source.canonical_bytes)
        .unwrap();
    assert_eq!(account.snapshot(), after_tables);

    let child = assembly
        .try_rewrite_canonical_records_controlled(
            source.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            keys.dimension,
            RecordLeafTag,
        )
        .unwrap();
    let after_child = account.snapshot();
    assert_eq!(physical_delta(after_tables, after_child).1, 2);
    let root = assembly
        .try_rewrite_canonical_records_controlled(
            source.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let after_root = account.snapshot();
    assert_eq!(physical_delta(after_child, after_root).1, 2);

    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(2_u8)),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        let Some(child_ref) = root.record_ref::<RecordLeafTag>(0, 0) else {
            return false;
        };
        if child_ref != child {
            return false;
        }
        let Some(child) = session.canonical_records(child_ref) else {
            return false;
        };
        let Some(sequence_ref) = child.sequence_ref::<u32, SequenceTag>(0, 0) else {
            return false;
        };
        let Some(bytes_ref) = child.bytes_ref::<BytesTag>(0, 1) else {
            return false;
        };
        sequence_ref == sequence
            && bytes_ref == bytes
            && session
                .canonical_sequence(sequence_ref)
                .and_then(|view| view.get(0))
                == Some(41)
            && session.canonical_bytes(bytes_ref) == Some(&[42, 43][..])
            && child.scalar::<u64>(0, 2) == Some(44)
            && root.scalar::<u32>(0, 1) == Some(45)
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("rewritten record graph must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("rewritten record graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedRecordGraph {
        canonical,
        canonical_sequence: sequence,
        canonical_bytes: bytes,
        canonical_child: child,
        _evidence: outcome_evidence,
    }
}

fn publish_rewritten_sequence_generation(source: &PublishedSource) -> PublishedSequenceGeneration {
    let (operation, keys) = derived_operation(&source.canonical);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 3, 2), 3, 2).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let sequence = assembly
        .try_rewrite_canonical_sequence_controlled(
            source.canonical_sequence,
            SequencePlan::<u32>::new(2).unwrap(),
            keys.dimension,
            SequenceTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(2)?;
                writer.extend_from_slice(&[22, 23])?;
                Ok(())
            },
        )
        .unwrap();
    let bytes = assembly
        .try_share_canonical(source.canonical_bytes)
        .unwrap();
    let root = assembly
        .try_rewrite_canonical_records_controlled(
            source.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(6_u8)),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.sequence_ref::<u32, SequenceTag>(0, 0) == Some(sequence)
            && root.bytes_ref::<BytesTag>(0, 1) == Some(bytes)
            && session
                .canonical_sequence(sequence)
                .is_some_and(|view| view.as_slice() == [22, 23])
            && session.canonical_bytes(bytes) == Some(&[12, 13][..])
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("rewritten sequence generation must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("rewritten sequence generation must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedSequenceGeneration {
        canonical,
        canonical_sequence: sequence,
        canonical_bytes: bytes,
        _evidence: outcome_evidence,
    }
}

#[test]
fn canonical_record_rewrite_is_fresh_accounted_and_release_order_independent() {
    let factory = factory(1);
    let account = account(&factory);

    let source = publish_record_graph_source(&account);
    let rewritten = publish_rewritten_record_graph(&account, &source);
    assert!(
        rewritten
            .canonical
            .records(source.canonical_child)
            .is_none()
    );
    assert!(
        rewritten
            .canonical
            .records(source.canonical.root_ref())
            .is_none()
    );
    assert!(
        rewritten
            .canonical
            .sequence(source.canonical_sequence)
            .is_none()
    );
    assert_eq!(
        rewritten
            .canonical
            .sequence(rewritten.canonical_sequence)
            .unwrap()
            .get(0),
        Some(41)
    );
    assert_eq!(
        rewritten.canonical.bytes(rewritten.canonical_bytes),
        Some(&[42, 43][..])
    );
    let rewritten_root = rewritten.canonical.root();
    assert_eq!(
        rewritten_root.record_ref::<RecordLeafTag>(0, 0),
        Some(rewritten.canonical_child)
    );

    drop(source);
    let rewritten_child = rewritten
        .canonical
        .records(rewritten.canonical_child)
        .expect("rewritten child survives source-first release");
    assert_eq!(rewritten_child.scalar::<u64>(0, 2), Some(44));
    drop(rewritten);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);

    let source = publish_record_graph_source(&account);
    let rewritten = publish_rewritten_record_graph(&account, &source);
    drop(rewritten);
    assert_eq!(
        source
            .canonical
            .sequence(source.canonical_sequence)
            .unwrap()
            .get(0),
        Some(41)
    );
    assert_eq!(
        source.canonical.bytes(source.canonical_bytes),
        Some(&[42, 43][..])
    );
    assert_eq!(
        source
            .canonical
            .records(source.canonical_child)
            .unwrap()
            .scalar::<u64>(0, 2),
        Some(44)
    );
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_sequence_rewrite_is_typed_accounted_and_retargets_records() {
    let factory = factory(2);
    let account = account(&factory);
    let foreign_account = super::account(&factory);
    let source_a = publish_record_graph_source(&account);
    let source_b = publish_record_graph_source(&account);
    let foreign_source = publish_record_graph_source(&foreign_account);

    let (operation, keys) = derived_operation(&source_a.canonical);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 4, 2), 4, 3).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let before_invalid = account.snapshot();
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence(
            foreign_source.canonical_sequence,
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |writer| writer.push(90),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence(
            source_b.canonical_sequence,
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |writer| writer.push(91),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), before_invalid);

    let replacement = assembly
        .try_rewrite_canonical_sequence(
            source_a.canonical_sequence,
            SequencePlan::<u32>::new(10).unwrap(),
            SequenceTag,
            |writer| writer.extend_from_slice(&[99; 10]),
        )
        .unwrap();
    let after_replacement = account.snapshot();
    assert_eq!(physical_delta(before_invalid, after_replacement).1, 2);
    assert_eq!(assembly.canonical_len(), 1);
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence(
            source_a.canonical_sequence,
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |_writer| panic!("duplicate rewrite must reject before construction"),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        assembly.try_share_canonical(source_a.canonical_sequence),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence(
            replacement,
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |_writer| panic!("destination-local references are not source authority"),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(account.snapshot(), after_replacement);

    let bytes = assembly
        .try_share_canonical(source_a.canonical_bytes)
        .unwrap();
    let child = assembly
        .try_rewrite_canonical_records_with(
            source_a.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            RecordLeafTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(replacement)?;
                    row.write_bytes_ref(bytes)?;
                    row.write_scalar(144_u64)
                })
            },
        )
        .unwrap();
    let after_child = account.snapshot();
    assert!(matches!(
        assembly.try_rewrite_canonical_records_with(
            source_a.canonical_child,
            RecordPlan::new(2, 1).unwrap(),
            RecordLeafTag,
            |_writer| panic!("one source record can have only one replacement"),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(assembly.canonical_len(), 3);
    assert_eq!(account.snapshot(), after_child);
    let root = assembly
        .try_rewrite_canonical_records_controlled(
            source_a.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(4_u8)),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        let Some(child_ref) = root.record_ref::<RecordLeafTag>(0, 0) else {
            return false;
        };
        let Some(child_view) = session.canonical_records(child_ref) else {
            return false;
        };
        let Some(sequence_ref) = child_view.sequence_ref::<u32, SequenceTag>(0, 0) else {
            return false;
        };
        let Some(bytes_ref) = child_view.bytes_ref::<BytesTag>(0, 1) else {
            return false;
        };
        child_ref == child
            && sequence_ref == replacement
            && bytes_ref == bytes
            && session
                .canonical_sequence(sequence_ref)
                .and_then(|view| view.get(0))
                == Some(99)
            && session.canonical_bytes(bytes_ref) == Some(&[42, 43][..])
            && child_view.scalar::<u64>(0, 2) == Some(144)
            && root.scalar::<u32>(0, 1) == Some(45)
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("sequence replacement graph must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("sequence replacement graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    assert!(canonical.sequence(source_a.canonical_sequence).is_none());
    assert_eq!(canonical.sequence(replacement).unwrap().get(0), Some(99));
    assert_eq!(canonical.sequence(replacement).unwrap().len(), 10);
    assert_eq!(canonical.bytes(bytes), Some(&[42, 43][..]));
    assert_eq!(
        canonical.root().record_ref::<RecordLeafTag>(0, 0),
        Some(child)
    );
    assert_eq!(
        canonical.records(child).unwrap().scalar::<u64>(0, 2),
        Some(144)
    );

    drop(source_a);
    assert_eq!(canonical.sequence(replacement).unwrap().get(0), Some(99));
    drop(canonical);
    drop(outcome_evidence);
    drop(source_b);
    drop(foreign_source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn controlled_sequence_rewrite_preserves_typed_source_lineage() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let (operation, keys) = derived_operation(&source.canonical);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 3, 2), 2, 2).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let replacement = assembly
        .try_rewrite_canonical_sequence_controlled(
            source.canonical_sequence,
            SequencePlan::<u32>::new(2).unwrap(),
            keys.dimension,
            SequenceTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(1)?;
                writer.extend_from_slice(&[22, 23])?;
                Ok(())
            },
        )
        .unwrap();
    let root = assembly
        .try_rewrite_canonical_records_with_controlled(
            source.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
            |writer, control| {
                control.poll()?;
                writer.write_row(|row| {
                    row.write_sequence_ref(replacement)?;
                    row.write_scalar(24_u32)
                })?;
                Ok(())
            },
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(5_u8)),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.sequence_ref::<u32, SequenceTag>(0, 0) == Some(replacement)
            && root.scalar::<u32>(0, 1) == Some(24)
            && session
                .canonical_sequence(replacement)
                .is_some_and(|view| view.as_slice() == [22, 23])
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("controlled sequence replacement must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("controlled sequence replacement must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert!(canonical.sequence(source.canonical_sequence).is_none());
    assert_eq!(
        canonical.sequence(replacement).unwrap().as_slice(),
        &[22, 23]
    );
    assert_eq!(
        canonical.root().sequence_ref::<u32, SequenceTag>(0, 0),
        Some(replacement)
    );
    assert_eq!(canonical.root().scalar::<u32>(0, 1), Some(24));
    assert!(
        ledger_evidence
            .dimension(keys.dimension)
            .expect("controlled rewrite work evidence")
            .attempted()
            >= 1
    );
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn bulk_leaf_inheritance_carries_rewritten_leaves_across_generations() {
    let factory = factory(1);
    let account = account(&factory);
    let generation_one = publish_sequence_source(&account);
    let generation_two = publish_rewritten_sequence_generation(&generation_one);

    let (operation, keys) = derived_operation(&generation_two.canonical);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let before_inheritance = account.snapshot();
    assembly
        .try_inherit_all_canonical_leaves(keys.dimension)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_inheritance);

    assembly
        .try_inherit_all_canonical_leaves(keys.dimension)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_inheritance);

    let sequence = assembly
        .try_share_canonical(generation_two.canonical_sequence)
        .unwrap();
    let bytes = assembly
        .try_share_canonical(generation_two.canonical_bytes)
        .unwrap();
    assert_eq!(assembly.canonical_len(), 2);
    let root = assembly
        .try_rewrite_canonical_records_controlled(
            generation_two.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(7_u8)),
        )
        .unwrap();
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.sequence_ref::<u32, SequenceTag>(0, 0) == Some(sequence)
            && root.bytes_ref::<BytesTag>(0, 1) == Some(bytes)
            && session
                .canonical_sequence(sequence)
                .is_some_and(|view| view.as_slice() == [22, 23])
            && session.canonical_bytes(bytes) == Some(&[12, 13][..])
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("third-generation inherited graph must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("third-generation inherited graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical: generation_three,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);

    drop(generation_two);
    assert_eq!(
        generation_three.sequence(sequence).unwrap().as_slice(),
        &[22, 23]
    );
    assert_eq!(generation_three.bytes(bytes), Some(&[12, 13][..]));
    drop(generation_one);
    assert_eq!(
        generation_three.sequence(sequence).unwrap().as_slice(),
        &[22, 23]
    );
    drop(generation_three);
    drop(outcome_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn bulk_leaf_inheritance_rolls_back_a_cancelled_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let cancellation = MidInheritanceCancellation::default();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = derived_operation_with_control(&source.canonical, control);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 2, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let existing = assembly
        .try_share_canonical(source.canonical_sequence)
        .unwrap();
    let before_inheritance = account.snapshot();
    cancellation.arm();

    assert!(matches!(
        assembly.try_inherit_all_canonical_leaves(keys.dimension),
        Err(StoreRuntimeError::Control(_))
    ));
    assert_eq!(assembly.canonical_len(), 1);
    assert!(source.canonical.sequence(existing).is_none());
    assert_eq!(account.snapshot(), before_inheritance);

    drop(assembly);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_source_binding_clears_a_cancelled_partial_fill() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let cancellation = MidInheritanceCancellation::default();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = derived_operation_with_control(&source.canonical, control);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    let before_binding = account.snapshot();
    cancellation.arm();

    assert!(matches!(
        assembly.try_bind_canonical_source(keys.dimension),
        Err(StoreRuntimeError::Control(_))
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), before_binding);

    drop(assembly);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn bulk_leaf_inheritance_rolls_back_slot_limit_and_remains_writable() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let (operation, keys) = derived_operation(&source.canonical);
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let before_inheritance = account.snapshot();

    assert!(matches!(
        assembly.try_inherit_all_canonical_leaves(keys.dimension),
        Err(StoreRuntimeError::SlotLimit {
            destination: baozi_operation::StoreDestination::Canonical
        })
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), before_inheritance);

    let bytes = assembly
        .try_share_canonical(source.canonical_bytes)
        .expect("ordinary bulk rollback must clear its source mapping suffix");
    assert_eq!(assembly.canonical_len(), 1);
    assert!(source.canonical.bytes(bytes).is_none());
    assert_eq!(account.snapshot(), before_inheritance);

    drop(assembly);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn same_account_share_is_zero_physical_growth_and_stale_refs_fail() {
    let factory = factory(1);
    let source_account = account(&factory);
    let source = publish_sequence_source(&source_account);

    let (share_operation, keys) = derived_operation(&source.canonical);
    let share_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 3, 1).unwrap(),
    );
    let mut assembly = share_operation
        .try_publication_assembly(share_plan)
        .unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let physical_before_share = source_account.snapshot();
    let canonical_sequence = assembly
        .try_share_canonical(source.canonical_sequence)
        .unwrap();
    let duplicate_canonical_sequence = assembly
        .try_share_canonical(source.canonical_sequence)
        .unwrap();
    assert_eq!(duplicate_canonical_sequence, canonical_sequence);
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(source_account.snapshot(), physical_before_share);
    assert!(matches!(
        assembly.try_rewrite_canonical_sequence(
            source.canonical_sequence,
            SequencePlan::<u32>::new(2).unwrap(),
            SequenceTag,
            |_writer| panic!("an inherited source cannot also be rewritten"),
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(source_account.snapshot(), physical_before_share);
    let canonical_bytes = assembly
        .try_share_canonical(source.canonical_bytes)
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

    drop(shared_canonical);
    drop(shared_evidence);
    drop(source);
    assert_eq!(source_account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn recoverable_open_failure_preserves_exact_canonical_source_authority() {
    let factory = factory(1);
    let account = account(&factory);
    let source_a = publish_sequence_source(&account);
    let source_b = publish_sequence_source(&account);
    let (operation, keys) = derived_operation(&source_a.canonical);
    let invalid_writer = RetainedStoreWriterPlan::new(
        foreign_phase(),
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(1, 2).unwrap(),
    );
    let invalid_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(invalid_writer, 1, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
    );
    let error = match operation.try_publication_assembly(invalid_plan) {
        Ok(_) => panic!("foreign writer phase must fail before publication allocation"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        StoreRuntimeError::ForeignPhaseDomain { phase } if *phase == foreign_phase()
    ));

    let recovered = error
        .try_into_operation()
        .expect("publication preflight failure must preserve the derived operation");
    let valid_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 1, 2), 1, 0).unwrap(),
    );
    let mut assembly = recovered.try_publication_assembly(valid_plan).unwrap();
    assembly.try_bind_canonical_source(keys.dimension).unwrap();
    let before_wrong_source = account.snapshot();
    assert!(matches!(
        assembly.try_share_canonical(source_b.canonical_sequence),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), before_wrong_source);
    let _shared = assembly
        .try_share_canonical(source_a.canonical_sequence)
        .expect("recovered operation must retain its exact source authority");
    drop(assembly.prepare_minimal_fallback().unwrap().handoff());

    drop(source_a);
    drop(source_b);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
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
    let (challenged_operation, challenged_keys) = derived_operation(&source_a.canonical);
    let mut challenged = challenged_operation
        .try_publication_assembly(target_plan(challenged_keys))
        .unwrap();
    challenged
        .try_bind_canonical_source(challenged_keys.dimension)
        .unwrap();
    let physical_before_invalid_shares = account.snapshot();
    assert!(matches!(
        challenged.try_share_canonical(source_b.canonical_sequence),
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
        .try_share_canonical(source_a.canonical_sequence)
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

    let (control_operation, control_keys) = derived_operation(&source_a.canonical);
    let mut control = control_operation
        .try_publication_assembly(target_plan(control_keys))
        .unwrap();
    control
        .try_bind_canonical_source(control_keys.dimension)
        .unwrap();
    let control_canonical = control
        .try_share_canonical(source_a.canonical_sequence)
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

#[test]
fn record_rewrite_rejects_invalid_sources_mappings_and_plans_before_mutation() {
    let factory = factory(2);
    let foreign_account = account(&factory);
    let account = account(&factory);
    let source_a = publish_record_graph_source(&account);
    let source_b = publish_record_graph_source(&account);
    let foreign_source = publish_record_graph_source(&foreign_account);

    let target_plan = |keys| {
        RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(writer_plan(keys, 3, 2), 4, 2).unwrap(),
            RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1).unwrap(),
        )
    };
    let (challenged_operation, challenged_keys) = derived_operation(&source_a.canonical);
    let mut challenged = challenged_operation
        .try_publication_assembly(target_plan(challenged_keys))
        .unwrap();
    challenged
        .try_bind_canonical_source(challenged_keys.dimension)
        .unwrap();
    let before_invalid = account.snapshot();
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            foreign_source.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            source_b.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            source_a.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::Build(_))
    ));
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            source_a.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            challenged_keys.dimension,
            CanonicalRootTag,
        ),
        Err(StoreRuntimeError::Build(_))
    ));
    assert_eq!(challenged.canonical_len(), 0);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        before_invalid.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        before_invalid.live_retained_allocations()
    );

    let sequence = challenged
        .try_share_canonical(source_a.canonical_sequence)
        .unwrap();
    let bytes = challenged
        .try_share_canonical(source_a.canonical_bytes)
        .unwrap();
    let after_shares = account.snapshot();
    assert_eq!(
        after_shares.live_retained_bytes(),
        before_invalid.live_retained_bytes()
    );
    assert_eq!(
        after_shares.live_retained_allocations(),
        before_invalid.live_retained_allocations()
    );
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            source_a.canonical_child,
            RecordPlan::new(1, 2).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::Build(_))
    ));
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            source_a.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            challenged_keys.dimension,
            CanonicalRootTag,
        ),
        Err(StoreRuntimeError::Build(_))
    ));
    assert_eq!(challenged.canonical_len(), 2);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        after_shares.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        after_shares.live_retained_allocations()
    );

    let child = challenged
        .try_rewrite_canonical_records_controlled(
            source_a.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        )
        .unwrap();
    let after_child = account.snapshot();
    assert!(matches!(
        challenged.try_rewrite_canonical_records_controlled(
            child,
            RecordPlan::new(1, 3).unwrap(),
            challenged_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    assert_eq!(challenged.canonical_len(), 3);
    assert_eq!(account.snapshot(), after_child);
    let root = challenged
        .try_rewrite_canonical_records_controlled(
            source_a.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            challenged_keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let evidence = challenged
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(3_u8)),
        )
        .unwrap();
    let challenged = match challenged.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        let Some(child) = root.record_ref::<RecordLeafTag>(0, 0) else {
            return false;
        };
        let Some(child) = session.canonical_records(child) else {
            return false;
        };
        let Some(sequence) = child.sequence_ref::<u32, SequenceTag>(0, 0) else {
            return false;
        };
        let Some(bytes) = child.bytes_ref::<BytesTag>(0, 1) else {
            return false;
        };
        session.canonical_sequence(sequence).is_some()
            && session.canonical_bytes(bytes).is_some()
            && session.outcome_evidence_records(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("challenged rewrite graph must validate"),
    };
    let challenged = match challenged.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("challenged rewrite graph must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = challenged.handoff();
    assert_eq!(canonical.sequence(sequence).unwrap().get(0), Some(41));
    assert_eq!(canonical.bytes(bytes), Some(&[42, 43][..]));
    drop(canonical);
    drop(outcome_evidence);
    let challenged_work = ledger_evidence
        .dimension(challenged_keys.dimension)
        .expect("challenged rewrite work evidence");
    drop(ledger_evidence);

    let (control_operation, control_keys) = derived_operation(&source_a.canonical);
    let mut control = control_operation
        .try_publication_assembly(target_plan(control_keys))
        .unwrap();
    control
        .try_bind_canonical_source(control_keys.dimension)
        .unwrap();
    let control_sequence = control
        .try_share_canonical(source_a.canonical_sequence)
        .unwrap();
    let control_bytes = control
        .try_share_canonical(source_a.canonical_bytes)
        .unwrap();
    let control_child = control
        .try_rewrite_canonical_records_controlled(
            source_a.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            control_keys.dimension,
            RecordLeafTag,
        )
        .unwrap();
    let control_root = control
        .try_rewrite_canonical_records_controlled(
            source_a.canonical.root_ref(),
            RecordPlan::new(1, 2).unwrap(),
            control_keys.dimension,
            CanonicalRootTag,
        )
        .unwrap();
    let control_evidence = control
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(3_u8)),
        )
        .unwrap();
    let control =
        match control.validate_success(control_root, control_evidence, |session, root, evidence| {
            let Some(root) = session.canonical_records(root) else {
                return false;
            };
            root.record_ref::<RecordLeafTag>(0, 0) == Some(control_child)
                && session.canonical_records(control_child).is_some()
                && session.canonical_sequence(control_sequence).is_some()
                && session.canonical_bytes(control_bytes).is_some()
                && session.outcome_evidence_records(evidence).is_some()
        }) {
            StoreSuccessValidation::Valid(validated) => validated,
            _ => panic!("control rewrite graph must validate"),
        };
    let control = match control.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("control rewrite graph must prepare success"),
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
        .expect("control rewrite work evidence");
    assert!(challenged_work.attempted() > control_work.attempted());
    assert!(challenged_work.consumed() > control_work.consumed());
    drop(ledger_evidence);

    let (capacity_operation, capacity_keys) = derived_operation(&source_a.canonical);
    let capacity_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(capacity_keys, 1, 2), 3, 0).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(capacity_keys, 1, 2), 1, 0).unwrap(),
    );
    let mut capacity = capacity_operation
        .try_publication_assembly(capacity_plan)
        .unwrap();
    capacity
        .try_bind_canonical_source(capacity_keys.dimension)
        .unwrap();
    let _sequence = capacity
        .try_share_canonical(source_a.canonical_sequence)
        .unwrap();
    let _bytes = capacity
        .try_share_canonical(source_a.canonical_bytes)
        .unwrap();
    let before_capacity_rejection = account.snapshot();
    assert!(matches!(
        capacity.try_rewrite_canonical_records_controlled(
            source_a.canonical_child,
            RecordPlan::new(1, 3).unwrap(),
            capacity_keys.dimension,
            RecordLeafTag,
        ),
        Err(StoreRuntimeError::SlotLimit {
            destination: baozi_operation::StoreDestination::Canonical
        })
    ));
    assert_eq!(capacity.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_capacity_rejection);
    assert!(matches!(
        capacity.try_share_canonical(source_b.canonical_sequence),
        Err(StoreRuntimeError::InvalidSourceSlot)
    ));
    let duplicate = capacity
        .try_share_canonical(source_a.canonical_sequence)
        .expect("capacity rejection must leave the bound assembly writable");
    assert_eq!(duplicate, _sequence);
    assert_eq!(capacity.canonical_len(), 2);
    assert_eq!(account.snapshot(), before_capacity_rejection);
    drop(capacity.prepare_minimal_fallback().unwrap().handoff());

    drop(source_a);
    drop(source_b);
    drop(foreign_source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
}
