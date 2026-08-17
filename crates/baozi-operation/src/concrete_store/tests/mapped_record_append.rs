use super::*;

use crate::ledger::LedgerLimits;

type TestDerivedOperation = DerivedRetainedStoreOperation<TestOwner, TestProgress, TestUnit>;

struct MappedPrefixSource {
    canonical: PublishedCanonicalStore<RecordBlock, TestRootTag>,
    sequence: CanonicalSequenceRef<u8, TestTag>,
    bytes: CanonicalBytesRef<TestTag>,
}

fn test_derived_operation_with_work_limits(
    source: &PublishedCanonicalStore<RecordBlock, TestRootTag>,
    attempted_limit: u64,
    consumed_limit: u64,
) -> (TestDerivedOperation, TestKeys) {
    let (domain, keys) = test_vocabulary();
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::new(
        keys.failure_terminal_dimension,
        1,
        u64::MAX,
        u64::MAX,
    )
    .expect("test failure-terminal work plan");
    let ordinary_attempted_limit = failure_terminal_work.ordinary_attempted_work_limit();
    let ordinary_consumed_limit = failure_terminal_work.ordinary_consumed_work_limit();
    let admitted_attempted_limit = attempted_limit.min(ordinary_attempted_limit / 2);
    let admitted_consumed_limit = consumed_limit.min(ordinary_consumed_limit / 2);
    let storage_policy = DimensionPolicy::reservable_bytes(
        ordinary_attempted_limit - admitted_attempted_limit,
        ordinary_consumed_limit - admitted_consumed_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let work_policy = DimensionPolicy::reservable_bytes(
        admitted_attempted_limit,
        admitted_consumed_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let ledger_limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, storage_policy),
            (keys.work_dimension, work_policy),
            (
                keys.failure_terminal_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )
    .expect("test ledger limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, ledger_limits, &[keys.phase])
        .expect("test store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 32, 8),
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
            .expect("test derived operation"),
        keys,
    )
}

fn publish_mapped_prefix_source(account: &PhysicalStorageAccount) -> MappedPrefixSource {
    let (operation, keys) = test_operation(account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("source publication assembly");
    let sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("source sequence plan"),
            TestTag,
            |writer| writer.push(7),
        )
        .expect("source sequence");
    let bytes = assembly
        .try_retain_canonical_bytes(
            BytesPlan::new(2).expect("source bytes plan"),
            TestTag,
            |writer| writer.write_bytes(&[8, 9]),
        )
        .expect("source bytes");
    let root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(2, 2).expect("source record plan"),
            TestRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(sequence)?;
                    row.write_scalar(11_u32)
                })?;
                writer.write_row(|row| {
                    row.write_bytes_ref(bytes)?;
                    row.write_scalar(12_u32)
                })
            },
        )
        .expect("source records");
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("source evidence plan"),
            TestTag,
            |writer| writer.push(1),
        )
        .expect("source evidence");
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.sequence_ref::<u8, TestTag>(0, 0) == Some(sequence)
            && root.scalar::<u32>(0, 1) == Some(11)
            && root.bytes_ref::<TestTag>(1, 0) == Some(bytes)
            && root.scalar::<u32>(1, 1) == Some(12)
            && session.canonical_sequence(sequence).is_some()
            && session.canonical_bytes(bytes) == Some(&[8, 9][..])
            && session.outcome_evidence_sequence(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("source graph must validate"),
    };
    let prepared = match validated
        .try_prepare(test_final_progress())
        .expect("source preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("source graph must prepare as success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(outcome_evidence);
    drop(ledger_evidence);
    MappedPrefixSource {
        canonical,
        sequence,
        bytes,
    }
}

fn assert_live_storage_unchanged(
    before: crate::PhysicalStorageAccountSnapshot,
    after: crate::PhysicalStorageAccountSnapshot,
) {
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
}

#[test]
fn mapped_prefix_retargets_historical_references_and_appends_exact_row() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) =
        test_derived_operation_with_work_limits(&source.canonical, u64::MAX, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("mapped-prefix assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let root = assembly
        .try_rewrite_canonical_records_with_mapped_prefix_controlled(
            &source.canonical,
            source.canonical.root_ref(),
            RecordPlan::new(2, 2).expect("source plan"),
            RecordPlan::new(3, 2).expect("destination plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, control| {
                assert_eq!(writer.rows(), 2);
                control.poll()?;
                writer.write_row(|row| {
                    row.write_scalar(13_u32)?;
                    row.write_scalar(14_u32)
                })?;
                Ok(())
            },
        )
        .expect("mapped-prefix records");
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(2),
        )
        .expect("destination evidence");
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        let Some(root) = session.canonical_records(root) else {
            return false;
        };
        root.rows() == 3
            && root.fields() == 2
            && root.sequence_ref::<u8, TestTag>(0, 0) == Some(sequence)
            && root.scalar::<u32>(0, 1) == Some(11)
            && root.bytes_ref::<TestTag>(1, 0) == Some(bytes)
            && root.scalar::<u32>(1, 1) == Some(12)
            && root.scalar::<u32>(2, 0) == Some(13)
            && root.scalar::<u32>(2, 1) == Some(14)
            && session.canonical_sequence(sequence).is_some()
            && session.canonical_bytes(bytes) == Some(&[8, 9][..])
            && session.outcome_evidence_sequence(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("mapped-prefix graph must validate"),
    };
    let prepared = match validated
        .try_prepare(test_final_progress())
        .expect("mapped-prefix preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("mapped-prefix graph must prepare as success"),
    };
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.work_dimension)
        .expect("mapped-prefix work evidence");
    assert_eq!((work.attempted(), work.consumed()), (4, 4));
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mapped_prefix_rejects_source_shape_and_destination_shape_before_mutation() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) =
        test_derived_operation_with_work_limits(&source.canonical, u64::MAX, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("shape-rejection assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let _sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let _bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let before = account.snapshot();

    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(1, 2).expect("wrong source plan"),
                RecordPlan::new(2, 2).expect("destination plan"),
                keys.work_dimension,
                TestRootTag,
                |_writer, _control| panic!("source mismatch must reject before append"),
            )
            .expect_err("wrong source shape must reject"),
        StoreRuntimeError::Build(StoreBuildError::capacity_envelope_violated())
    );
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());

    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(2, 2).expect("source plan"),
                RecordPlan::new(3, 1).expect("wrong destination fields"),
                keys.work_dimension,
                TestRootTag,
                |_writer, _control| panic!("field mismatch must reject before append"),
            )
            .expect_err("destination field mismatch must reject"),
        StoreRuntimeError::Build(StoreBuildError::invalid_record_width())
    );
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());

    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(2, 2).expect("source plan"),
                RecordPlan::new(1, 2).expect("undersized destination plan"),
                keys.work_dimension,
                TestRootTag,
                |_writer, _control| panic!("short destination must reject before append"),
            )
            .expect_err("undersized destination must reject"),
        StoreRuntimeError::Build(StoreBuildError::capacity_exceeded())
    );
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());

    drop(
        assembly
            .prepare_minimal_fallback()
            .expect("shape-rejection fallback")
            .handoff(),
    );
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mapped_prefix_build_failure_rolls_back_and_leaves_source_rewritable() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) =
        test_derived_operation_with_work_limits(&source.canonical, u64::MAX, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("build-failure assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let before = account.snapshot();

    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(2, 2).expect("source plan"),
                RecordPlan::new(3, 2).expect("destination plan"),
                keys.work_dimension,
                TestRootTag,
                |writer, _control| {
                    assert_eq!(writer.rows(), 2);
                    writer.write_row(|row| row.write_scalar(99_u32))?;
                    Ok(())
                },
            )
            .expect_err("invalid appended row must reject"),
        StoreRuntimeError::Build(StoreBuildError::invalid_record_width())
    );
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());

    let root = assembly
        .try_rewrite_canonical_records_with_mapped_prefix_controlled(
            &source.canonical,
            source.canonical.root_ref(),
            RecordPlan::new(2, 2).expect("source plan"),
            RecordPlan::new(3, 2).expect("destination plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, _control| {
                assert_eq!(writer.rows(), 2);
                writer.write_row(|row| {
                    row.write_scalar(15_u32)?;
                    row.write_scalar(16_u32)
                })?;
                Ok(())
            },
        )
        .expect("retry after rolled-back build failure");
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(3),
        )
        .expect("destination evidence");
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical_records(root).is_some_and(|root| {
            root.sequence_ref::<u8, TestTag>(0, 0) == Some(sequence)
                && root.bytes_ref::<TestTag>(1, 0) == Some(bytes)
                && root.scalar::<u32>(2, 0) == Some(15)
                && root.scalar::<u32>(2, 1) == Some(16)
        }) && session.canonical_sequence(sequence).is_some()
            && session.canonical_bytes(bytes) == Some(&[8, 9][..])
            && session.outcome_evidence_sequence(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("retried mapped-prefix graph must validate"),
    };
    let prepared = match validated
        .try_prepare(test_final_progress())
        .expect("retried mapped-prefix preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("retried mapped-prefix graph must prepare as success"),
    };
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.work_dimension)
        .expect("retried mapped-prefix work evidence");
    assert_eq!((work.attempted(), work.consumed()), (8, 8));
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mapped_prefix_ignored_control_failure_is_sticky_and_fails_closed() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) = test_derived_operation_with_work_limits(&source.canonical, 4, 4);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("sticky-control assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let _sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let _bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let before = account.snapshot();

    assert!(matches!(
        assembly.try_rewrite_canonical_records_with_mapped_prefix_controlled(
            &source.canonical,
            source.canonical.root_ref(),
            RecordPlan::new(2, 2).expect("source plan"),
            RecordPlan::new(3, 2).expect("destination plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, control| {
                assert_eq!(writer.rows(), 2);
                assert!(control.charge_observed(1).is_err());
                writer.write_row(|row| {
                    row.write_scalar(17_u32)?;
                    row.write_scalar(18_u32)
                })?;
                Ok(())
            },
        ),
        Err(StoreRuntimeError::Ledger(_))
    ));
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());
    assert!(matches!(
        assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("post-failure plan"),
            TestTag,
            |_writer| panic!("sticky control failure must close successful construction"),
        ),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));

    drop(
        assembly
            .prepare_minimal_fallback()
            .expect("sticky-control fallback")
            .handoff(),
    );
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mapped_prefix_work_failure_during_copy_rolls_back_and_fails_closed() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) = test_derived_operation_with_work_limits(&source.canonical, 3, 3);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("mid-prefix failure assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let _sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let _bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let before = account.snapshot();

    assert!(matches!(
        assembly.try_rewrite_canonical_records_with_mapped_prefix_controlled(
            &source.canonical,
            source.canonical.root_ref(),
            RecordPlan::new(2, 2).expect("source plan"),
            RecordPlan::new(3, 2).expect("destination plan"),
            keys.work_dimension,
            TestRootTag,
            |_writer, _control| panic!("prefix work exhaustion must reject before append"),
        ),
        Err(StoreRuntimeError::Ledger(_))
    ));
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before, account.snapshot());
    assert!(matches!(
        assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("post-failure plan"),
            TestTag,
            |_writer| panic!("mid-prefix ledger failure must close successful construction"),
        ),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));

    drop(
        assembly
            .prepare_minimal_fallback()
            .expect("mid-prefix failure fallback")
            .handoff(),
    );
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mapped_prefix_missing_mapping_and_incomplete_append_roll_back_before_retry() {
    let factory = test_factory();
    let account = test_account(&factory);
    let source = publish_mapped_prefix_source(&account);
    let (operation, keys) =
        test_derived_operation_with_work_limits(&source.canonical, u64::MAX, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("recoverable mapped-prefix assembly");
    assembly
        .try_bind_canonical_source(&source.canonical, keys.dimension)
        .expect("bound canonical source");
    let sequence = assembly
        .try_share_canonical(&source.canonical, source.sequence)
        .expect("mapped sequence");
    let before_missing_mapping = account.snapshot();

    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(2, 2).expect("source plan"),
                RecordPlan::new(3, 2).expect("destination plan"),
                keys.work_dimension,
                TestRootTag,
                |_writer, _control| panic!("missing mapping must reject before append"),
            )
            .expect_err("unmapped historical reference must reject"),
        StoreRuntimeError::Build(StoreBuildError::capacity_envelope_violated())
    );
    assert_eq!(assembly.canonical_len(), 1);
    assert_live_storage_unchanged(before_missing_mapping, account.snapshot());

    let bytes = assembly
        .try_share_canonical(&source.canonical, source.bytes)
        .expect("mapped bytes");
    let before_incomplete_append = account.snapshot();
    assert_eq!(
        assembly
            .try_rewrite_canonical_records_with_mapped_prefix_controlled(
                &source.canonical,
                source.canonical.root_ref(),
                RecordPlan::new(2, 2).expect("source plan"),
                RecordPlan::new(3, 2).expect("destination plan"),
                keys.work_dimension,
                TestRootTag,
                |writer, _control| {
                    assert_eq!(writer.rows(), 2);
                    Ok(())
                },
            )
            .expect_err("incomplete append must reject"),
        StoreRuntimeError::Build(StoreBuildError::incomplete_write())
    );
    assert_eq!(assembly.canonical_len(), 2);
    assert_live_storage_unchanged(before_incomplete_append, account.snapshot());

    let root = assembly
        .try_rewrite_canonical_records_with_mapped_prefix_controlled(
            &source.canonical,
            source.canonical.root_ref(),
            RecordPlan::new(2, 2).expect("source plan"),
            RecordPlan::new(3, 2).expect("destination plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, _control| {
                writer.write_row(|row| {
                    row.write_scalar(19_u32)?;
                    row.write_scalar(20_u32)
                })?;
                Ok(())
            },
        )
        .expect("retry after both recoverable failures");
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(4),
        )
        .expect("destination evidence");
    let validated = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical_records(root).is_some_and(|root| {
            root.sequence_ref::<u8, TestTag>(0, 0) == Some(sequence)
                && root.bytes_ref::<TestTag>(1, 0) == Some(bytes)
                && root.scalar::<u32>(2, 0) == Some(19)
                && root.scalar::<u32>(2, 1) == Some(20)
        }) && session.canonical_sequence(sequence).is_some()
            && session.canonical_bytes(bytes) == Some(&[8, 9][..])
            && session.outcome_evidence_sequence(evidence).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("retried mapped-prefix graph must validate"),
    };
    let prepared = match validated
        .try_prepare(test_final_progress())
        .expect("retried mapped-prefix preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("retried mapped-prefix graph must prepare as success"),
    };
    drop(prepared.handoff());
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
