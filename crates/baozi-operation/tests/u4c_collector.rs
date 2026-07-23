use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Mutex, MutexGuard, PoisonError};

use baozi_operation::__u4c::MeasurementSession;
use baozi_operation::{
    BytesPlan, CanonicalSequenceRef, DimensionPolicy, FinalProgress, LedgerLimits,
    OperationControl, OperationOwner, OutcomeEvidenceSequenceRef, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, PreparedStoreSuccessOutcome, ProgressPhase, ProgressUnit,
    PublishedCanonicalStore, PublishedOutcomeEvidenceStore, PublishedStoreSuccessParts, RecordPlan,
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStoreOperation,
    RetainedStorePublicationPlan, RetainedStoreSharePlan, RetainedStoreStartPlan,
    RetainedStoreWriterPlan, ScalarSequence, SequencePlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan,
    StorePhaseKey, StorePhaseSemanticId, StoreSemanticCatalogId, StoreSuccessValidation, Utf8Plan,
};

#[path = "../src/u4c_collector.rs"]
mod isolated_collector;

static COLLECTOR_TEST_LOCK: Mutex<()> = Mutex::new(());

fn collector_test_lock() -> MutexGuard<'static, ()> {
    COLLECTOR_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestProgress {
    Build,
}

impl ProgressPhase for TestProgress {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestUnit {
    Items,
}

impl ProgressUnit for TestUnit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

#[derive(Clone, Copy)]
struct CanonicalSequenceTag;
#[derive(Clone, Copy)]
struct CanonicalBytesTag;
#[derive(Clone, Copy)]
struct CanonicalUtf8Tag;
#[derive(Clone, Copy)]
struct CanonicalRootTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceSequenceTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceBytesTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceUtf8Tag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceRootTag;
#[derive(Clone, Copy)]
struct SharedCanonicalTag;
#[derive(Clone, Copy)]
struct SharedOutcomeEvidenceTag;

type TestOperation = RetainedStoreOperation<TestOwner, TestProgress, TestUnit>;
type SharedCanonical = PublishedCanonicalStore<ScalarSequence<u8>, SharedCanonicalTag>;
type SharedOutcomeEvidence =
    PublishedOutcomeEvidenceStore<ScalarSequence<u8>, SharedOutcomeEvidenceTag>;

#[derive(Clone, Copy)]
struct TestKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
}

struct PublishedSharedSource {
    canonical: SharedCanonical,
    canonical_root: CanonicalSequenceRef<u8, SharedCanonicalTag>,
    outcome_evidence: SharedOutcomeEvidence,
    outcome_evidence_root: OutcomeEvidenceSequenceRef<u8, SharedOutcomeEvidenceTag>,
}

const fn catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(
        *b"baozi-u4c-test!!",
        NonZeroU32::new(1).expect("test catalog revision is nonzero"),
    )
}

const fn phase_semantic(local: u64) -> StorePhaseSemanticId {
    StorePhaseSemanticId::new(
        catalog(),
        NonZeroU64::new(local).expect("test phase semantic id is nonzero"),
    )
}

const fn dimension_semantic(local: u64) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        catalog(),
        NonZeroU64::new(local).expect("test dimension semantic id is nonzero"),
    )
}

fn keys() -> (StoreKeyDomain, TestKeys) {
    let domain = StoreKeyDomain::try_new(catalog()).expect("test store key domain");
    let keys = TestKeys {
        phase: domain
            .try_admit_phase(phase_semantic(1))
            .expect("test phase key"),
        dimension: domain
            .try_admit_dimension(dimension_semantic(1))
            .expect("test dimension key"),
    };
    (domain, keys)
}

fn writer_plan(keys: TestKeys, attributions: usize, commitments: usize) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attributions, commitments).expect("test batch limits"),
    )
}

fn operation(account: &PhysicalStorageAccount) -> (TestOperation, TestKeys) {
    let (domain, keys) = keys();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("test ledger");
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
        TestOperation::try_start(OperationControl::default(), ledger_plan, account, start)
            .expect("test operation"),
        keys,
    )
}

fn publication_plan(
    keys: TestKeys,
    canonical_blocks: usize,
    outcome_evidence_blocks: usize,
) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .expect("canonical table plan"),
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, outcome_evidence_blocks + 1, 2),
            outcome_evidence_blocks,
            outcome_evidence_blocks,
        )
        .expect("outcome-evidence table plan"),
    )
}

fn final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn account() -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).expect("test factory limits"),
    )
    .expect("test factory");
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 256).expect("test account limits"),
        )
        .expect("test account")
}

fn publish_shared_source(account: &PhysicalStorageAccount) -> PublishedSharedSource {
    let (operation, keys) = operation(account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .expect("source publication assembly");
    let canonical_root = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical sequence plan"),
            SharedCanonicalTag,
            |writer| writer.push(41),
        )
        .expect("source canonical sequence");
    let outcome_evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("outcome-evidence sequence plan"),
            SharedOutcomeEvidenceTag,
            |writer| writer.push(73),
        )
        .expect("source outcome-evidence sequence");
    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |session, canonical_root, outcome_evidence_root| {
            session
                .canonical_sequence(canonical_root)
                .and_then(|view| view.get(0))
                == Some(41)
                && session
                    .outcome_evidence_sequence(outcome_evidence_root)
                    .and_then(|view| view.get(0))
                    == Some(73)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("source must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("source preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("source must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedSharedSource {
        canonical,
        canonical_root,
        outcome_evidence,
        outcome_evidence_root,
    }
}

#[test]
fn real_publication_and_view_paths_emit_the_fixed_taxonomy() {
    let _lock = collector_test_lock();
    let account = account();
    let (operation, keys) = operation(&account);
    let session = MeasurementSession::begin().expect("collector session");
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 4, 4))
        .expect("publication assembly");

    let canonical_sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical sequence plan"),
            CanonicalSequenceTag,
            |writer| writer.push(11),
        )
        .expect("canonical sequence");
    let canonical_bytes = assembly
        .try_retain_canonical_bytes(
            BytesPlan::new(3).expect("canonical bytes plan"),
            CanonicalBytesTag,
            |writer| writer.write_bytes(b"abc"),
        )
        .expect("canonical bytes");
    let canonical_utf8 = assembly
        .try_retain_canonical_utf8(
            Utf8Plan::new(3).expect("canonical UTF-8 plan"),
            CanonicalUtf8Tag,
            |writer| writer.write_str("cat"),
        )
        .expect("canonical UTF-8");
    let canonical_root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, 4).expect("canonical record plan"),
            CanonicalRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(canonical_sequence)?;
                    row.write_bytes_ref(canonical_bytes)?;
                    row.write_utf8_ref(canonical_utf8)?;
                    row.write_scalar(5u8)
                })
            },
        )
        .expect("canonical record root");

    let outcome_evidence_sequence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("outcome-evidence sequence plan"),
            OutcomeEvidenceSequenceTag,
            |writer| writer.push(17),
        )
        .expect("outcome-evidence sequence");
    let outcome_evidence_bytes = assembly
        .try_retain_outcome_evidence_bytes(
            BytesPlan::new(3).expect("outcome-evidence bytes plan"),
            OutcomeEvidenceBytesTag,
            |writer| writer.write_bytes(b"def"),
        )
        .expect("outcome-evidence bytes");
    let outcome_evidence_utf8 = assembly
        .try_retain_outcome_evidence_utf8(
            Utf8Plan::new(3).expect("outcome-evidence UTF-8 plan"),
            OutcomeEvidenceUtf8Tag,
            |writer| writer.write_str("dog"),
        )
        .expect("outcome-evidence UTF-8");
    let outcome_evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 4).expect("outcome-evidence record plan"),
            OutcomeEvidenceRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(outcome_evidence_sequence)?;
                    row.write_bytes_ref(outcome_evidence_bytes)?;
                    row.write_utf8_ref(outcome_evidence_utf8)?;
                    row.write_scalar(9u8)
                })
            },
        )
        .expect("outcome-evidence record root");

    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |validation, canonical_root, outcome_evidence_root| {
            let Some(canonical_root) = validation.canonical_records(canonical_root) else {
                return false;
            };
            let Some(canonical_sequence) =
                canonical_root.sequence_ref::<u8, CanonicalSequenceTag>(0, 0)
            else {
                return false;
            };
            let Some(canonical_bytes) = canonical_root.bytes_ref::<CanonicalBytesTag>(0, 1) else {
                return false;
            };
            let Some(canonical_utf8) = canonical_root.utf8_ref::<CanonicalUtf8Tag>(0, 2) else {
                return false;
            };
            let Some(outcome_evidence_root) =
                validation.outcome_evidence_records(outcome_evidence_root)
            else {
                return false;
            };
            let Some(outcome_evidence_sequence) =
                outcome_evidence_root.sequence_ref::<u8, OutcomeEvidenceSequenceTag>(0, 0)
            else {
                return false;
            };
            let Some(outcome_evidence_bytes) =
                outcome_evidence_root.bytes_ref::<OutcomeEvidenceBytesTag>(0, 1)
            else {
                return false;
            };
            let Some(outcome_evidence_utf8) =
                outcome_evidence_root.utf8_ref::<OutcomeEvidenceUtf8Tag>(0, 2)
            else {
                return false;
            };

            validation
                .canonical_sequence(canonical_sequence)
                .and_then(|view| view.get(0))
                == Some(11)
                && validation.canonical_bytes(canonical_bytes) == Some(&b"abc"[..])
                && validation.canonical_utf8(canonical_utf8) == Some("cat")
                && validation
                    .outcome_evidence_sequence(outcome_evidence_sequence)
                    .and_then(|view| view.get(0))
                    == Some(17)
                && validation.outcome_evidence_bytes(outcome_evidence_bytes) == Some(&b"def"[..])
                && validation.outcome_evidence_utf8(outcome_evidence_utf8) == Some("dog")
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("publication must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("publication preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("publication must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);

    let canonical_root = canonical.root();
    let canonical_sequence = canonical_root
        .sequence_ref::<u8, CanonicalSequenceTag>(0, 0)
        .expect("canonical sequence reference");
    let canonical_bytes = canonical_root
        .bytes_ref::<CanonicalBytesTag>(0, 1)
        .expect("canonical bytes reference");
    let canonical_utf8 = canonical_root
        .utf8_ref::<CanonicalUtf8Tag>(0, 2)
        .expect("canonical UTF-8 reference");
    assert_eq!(
        canonical
            .sequence(canonical_sequence)
            .and_then(|view| view.get(0)),
        Some(11)
    );
    assert_eq!(canonical.bytes(canonical_bytes), Some(&b"abc"[..]));
    assert_eq!(canonical.utf8(canonical_utf8), Some("cat"));

    let outcome_evidence_root = outcome_evidence.root();
    let outcome_evidence_sequence = outcome_evidence_root
        .sequence_ref::<u8, OutcomeEvidenceSequenceTag>(0, 0)
        .expect("outcome-evidence sequence reference");
    let outcome_evidence_bytes = outcome_evidence_root
        .bytes_ref::<OutcomeEvidenceBytesTag>(0, 1)
        .expect("outcome-evidence bytes reference");
    let outcome_evidence_utf8 = outcome_evidence_root
        .utf8_ref::<OutcomeEvidenceUtf8Tag>(0, 2)
        .expect("outcome-evidence UTF-8 reference");
    assert_eq!(
        outcome_evidence
            .sequence(outcome_evidence_sequence)
            .and_then(|view| view.get(0)),
        Some(17)
    );
    assert_eq!(
        outcome_evidence.bytes(outcome_evidence_bytes),
        Some(&b"def"[..])
    );
    assert_eq!(outcome_evidence.utf8(outcome_evidence_utf8), Some("dog"));

    let report = session.finish().expect("complete measurement");
    assert_eq!(report.semantic_cell_writes(), 16);
    assert_eq!(report.reference_writes(), 6);
    assert_eq!(report.producer_events(), 46);
    assert!(report.payload_copy_bytes() > 14);
    assert_eq!(report.payload_backing_allocations(), 8);
    assert_eq!(report.block_begins(), 8);
    assert_eq!(report.block_seals(), 8);
    assert_eq!(report.slot_publications(), 8);
    assert_eq!(report.new_block_begins(), 8);
    assert_eq!(report.new_block_seals(), 8);
    assert_eq!(report.new_slot_publications(), 8);
    assert_eq!(report.inherited_block_begins(), 0);
    assert_eq!(report.inherited_block_seals(), 0);
    assert_eq!(report.inherited_slot_publications(), 0);
    assert_eq!(report.element_decodes(), 0);
    assert_eq!(report.completed_lookups(), 8);
    assert_eq!(report.lookup_events(), 64);
    assert_eq!(report.max_lookup_events(), 8);
}

#[test]
fn inherited_shares_emit_lifecycles_without_copying_or_hot_leaf_lookup() {
    let _lock = collector_test_lock();
    let account = account();
    let source = publish_shared_source(&account);
    let (operation, keys) = operation(&account);
    let session = MeasurementSession::begin().expect("collector session");
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .expect("shared publication assembly");
    let canonical_root = assembly
        .try_share_canonical(&source.canonical, source.canonical_root)
        .expect("shared canonical root");
    let outcome_evidence_root = assembly
        .try_share_outcome_evidence(&source.outcome_evidence, source.outcome_evidence_root)
        .expect("shared outcome-evidence root");
    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |validation, canonical_root, outcome_evidence_root| {
            validation
                .canonical_sequence(canonical_root)
                .and_then(|view| view.get(0))
                == Some(41)
                && validation
                    .outcome_evidence_sequence(outcome_evidence_root)
                    .and_then(|view| view.get(0))
                    == Some(73)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("shared publication must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("shared preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("shared publication must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);

    let report = session.finish().expect("complete share measurement");
    assert_eq!(report.semantic_cell_writes(), 0);
    assert_eq!(report.reference_writes(), 0);
    assert_eq!(report.payload_copy_bytes(), 0);
    assert_eq!(report.payload_backing_allocations(), 0);
    assert_eq!(report.producer_events(), 6);
    assert_eq!(report.block_begins(), 2);
    assert_eq!(report.block_seals(), 2);
    assert_eq!(report.slot_publications(), 2);
    assert_eq!(report.new_block_begins(), 0);
    assert_eq!(report.new_block_seals(), 0);
    assert_eq!(report.new_slot_publications(), 0);
    assert_eq!(report.inherited_block_begins(), 2);
    assert_eq!(report.inherited_block_seals(), 2);
    assert_eq!(report.inherited_slot_publications(), 2);
    assert_eq!(report.element_decodes(), 0);
    assert_eq!(report.completed_lookups(), 0);
    assert_eq!(report.lookup_events(), 0);
    assert_eq!(report.max_lookup_events(), 0);

    assert_eq!(canonical.root().get(0), Some(41));
    assert_eq!(outcome_evidence.root().get(0), Some(73));
}

#[test]
fn source_gates_keep_public_views_on_the_checked_lookup_seam() {
    let terminal_output = include_str!("../src/concrete_store/terminal_output.rs");
    let retained = include_str!("../src/retained/block_table.rs");
    let validation = include_str!("../src/concrete_store/validation.rs");

    assert_eq!(
        terminal_output
            .matches("project_sequence_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all canonical, outcome-evidence, and rich-failure sequence views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_bytes_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all binary views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_utf8_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all UTF-8 views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_canonical_record_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        1,
        "canonical record views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_outcome_evidence_record_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        2,
        "outcome-evidence and rich-failure record views use the checked seam"
    );
    assert!(
        !terminal_output.contains(".get(slot.raw)")
            && !terminal_output.contains(".member(slot.raw)"),
        "published concrete-store views must not bypass checked lookup with a raw slot"
    );
    assert!(
        !validation.contains("record_lookup_"),
        "construction validation is not a published typed-view measurement path"
    );
    assert!(
        !terminal_output.contains("record_element_decode("),
        "the selected typed-view implementation stays zero-copy"
    );

    for stage in [
        "LookupStage::CheckTableBrand",
        "LookupStage::CheckSlotSet",
        "LookupStage::CheckRange",
        "LookupStage::LoadMember",
    ] {
        assert!(
            retained.contains(stage),
            "retained lookup must retain the {stage} taxonomy stage"
        );
    }
    for stage in [
        "LookupStage::CheckVariant",
        "LookupStage::CheckSchemaTag",
        "LookupStage::ProjectView",
        "LookupStage::ReturnView",
    ] {
        assert!(
            terminal_output.contains(stage),
            "terminal projection must retain the {stage} taxonomy stage"
        );
    }
    assert!(
        retained.contains("record_lookup_rejected") && terminal_output.contains("reject_lookup"),
        "both retained and terminal rejection paths must explicitly complete the taxonomy"
    );
}

#[test]
fn collector_state_machine_remains_fail_closed() {
    isolated_collector::run_state_machine_tests();
}
