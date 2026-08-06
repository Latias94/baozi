use std::hash::{Hash, Hasher};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Arc, OnceLock};

#[cfg(feature = "_u4-activation")]
use baozi_operation::StoreBuildError;
use baozi_operation::{
    AccountPoisonReason, BytesPlan, CancellationToken, CanonicalSequenceRef, DimensionPolicy,
    FinalProgress, LedgerCause, LedgerLimits, OperationControl, OperationOwner,
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PreparedStoreFailureOutcome,
    PreparedStoreSuccessOutcome, ProgressPhase, ProgressUnit, PublishedStoreFailureParts,
    PublishedStoreSuccessParts, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreOperation, RetainedStorePublicationPlan,
    RetainedStoreSharePlan, RetainedStoreStartError, RetainedStoreStartPlan,
    RetainedStoreWriterPlan, SequencePlan, StoreBuildFailureClass, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFailureValidation, StoreFallbackEnvelope, StoreKeyDomain,
    StoreLedgerEvidence, StoreLedgerPlan, StoreLedgerPlanError, StorePhaseKey,
    StorePhaseSemanticId, StorePreparationFailure, StorePreparationFailureClass, StorePrepareError,
    StoreRuntimeError, StoreSemanticCatalogId, StoreSuccessValidation, StoreTerminalCause,
    Utf8Plan,
};

struct TestOwner;

impl OperationOwner for TestOwner {}

const TEST_TERMINAL_ATTRIBUTION_SLOTS: usize = 32;
const TEST_MAX_FAILURE_EVIDENCE_MEMBERS: usize = 5;
const TEST_FAILURE_EVIDENCE_WORK_PER_MEMBER: u64 = 64;
const TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT: u64 =
    TEST_MAX_FAILURE_EVIDENCE_MEMBERS as u64 * TEST_FAILURE_EVIDENCE_WORK_PER_MEMBER;
const TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT: u64 =
    TEST_MAX_FAILURE_EVIDENCE_MEMBERS as u64 * TEST_FAILURE_EVIDENCE_WORK_PER_MEMBER;

#[derive(Default)]
struct HashTranscript(Vec<u8>);

impl Hasher for HashTranscript {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

fn hash_transcript(value: &impl Hash) -> Vec<u8> {
    let mut transcript = HashTranscript::default();
    value.hash(&mut transcript);
    transcript.0
}

const fn phase_semantic_for(catalog: StoreSemanticCatalogId, value: u64) -> StorePhaseSemanticId {
    StorePhaseSemanticId::new(
        catalog,
        NonZeroU64::new(value).expect("test phase semantic id is nonzero"),
    )
}

const fn phase_semantic(value: u64) -> StorePhaseSemanticId {
    phase_semantic_for(test_catalog(), value)
}

const fn dimension_semantic_for(
    catalog: StoreSemanticCatalogId,
    value: u64,
) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        catalog,
        NonZeroU64::new(value).expect("test dimension semantic id is nonzero"),
    )
}

const fn dimension_semantic(value: u64) -> StoreDimensionSemanticId {
    dimension_semantic_for(test_catalog(), value)
}

const fn test_catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(
        *b"baozi-test-store",
        NonZeroU32::new(1).expect("test catalog revision is nonzero"),
    )
}

#[derive(Clone, Copy)]
struct TestKeys {
    phase: StorePhaseKey,
    other_phase: StorePhaseKey,
    dimension: StoreDimensionKey,
    other_dimension: StoreDimensionKey,
    failure_terminal_dimension: StoreDimensionKey,
}

struct TestVocabulary {
    domain: StoreKeyDomain,
    keys: TestKeys,
}

fn test_vocabulary() -> TestVocabulary {
    test_vocabulary_for(test_catalog())
}

fn test_vocabulary_for(catalog: StoreSemanticCatalogId) -> TestVocabulary {
    let domain = StoreKeyDomain::try_new(catalog).expect("test store key domain");
    let keys = TestKeys {
        phase: domain
            .try_admit_phase(phase_semantic_for(catalog, 1))
            .expect("test phase key"),
        other_phase: domain
            .try_admit_phase(phase_semantic_for(catalog, 2))
            .expect("other test phase key"),
        dimension: domain
            .try_admit_dimension(dimension_semantic_for(catalog, 1))
            .expect("test dimension key"),
        other_dimension: domain
            .try_admit_dimension(dimension_semantic_for(catalog, 2))
            .expect("other test dimension key"),
        failure_terminal_dimension: domain
            .try_admit_dimension(dimension_semantic_for(catalog, 3))
            .expect("failure-terminal test dimension key"),
    };
    TestVocabulary { domain, keys }
}

struct FakeProducer {
    catalog: StoreSemanticCatalogId,
}

fn fake_producer() -> Arc<FakeProducer> {
    Arc::new(FakeProducer {
        catalog: test_catalog(),
    })
}

fn foreign_phase() -> StorePhaseKey {
    static KEY: OnceLock<StorePhaseKey> = OnceLock::new();
    *KEY.get_or_init(|| {
        StoreKeyDomain::try_new(test_catalog())
            .expect("foreign phase key domain")
            .try_admit_phase(phase_semantic(1))
            .expect("foreign phase key")
    })
}

fn store_failure_cause(evidence: &StoreLedgerEvidence) -> StorePreparationFailure {
    match evidence.header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause,
        StoreTerminalCause::Success => panic!("published failure must not report success"),
        _ => panic!("published failure reported an unsupported terminal cause"),
    }
}

fn assert_semantic_and_ordinal_evidence_queries(evidence: &StoreLedgerEvidence, keys: TestKeys) {
    let dimension = evidence
        .dimension_by_semantic(keys.dimension.semantic_id())
        .expect("admitted dimension remains queryable by semantic identity");
    assert_eq!(dimension.dimension(), keys.dimension.semantic_id());
    assert_eq!(evidence.dimension_at(0), Some(dimension));
    let failure_terminal = evidence
        .dimension_by_semantic(keys.failure_terminal_dimension.semantic_id())
        .expect("protected failure-terminal dimension remains explicit evidence");
    assert_eq!(
        failure_terminal.dimension(),
        keys.failure_terminal_dimension.semantic_id()
    );
    assert_eq!(evidence.dimension_at(1), Some(failure_terminal));
    assert_eq!(evidence.dimension_at(2), None);
    assert_eq!(
        evidence.dimension_by_semantic(keys.other_dimension.semantic_id()),
        None,
        "an admitted key omitted from the frozen plan is not evidence authority"
    );

    let phase = evidence
        .phase_by_semantic(keys.phase.semantic_id())
        .expect("admitted phase remains queryable by semantic identity");
    assert_eq!(phase.phase(), keys.phase.semantic_id());
    assert_eq!(evidence.phase_at(0), Some(phase));
    assert_eq!(evidence.phase_at(1), None);
    assert_eq!(
        evidence.phase_by_semantic(keys.other_phase.semantic_id()),
        None,
        "an admitted key omitted from the frozen plan is not evidence authority"
    );

    let foreign_catalog = StoreSemanticCatalogId::new(
        *b"foreign-catalog!",
        NonZeroU32::new(1).expect("foreign catalog revision is nonzero"),
    );
    assert_eq!(
        evidence.dimension_by_semantic(StoreDimensionSemanticId::new(
            foreign_catalog,
            NonZeroU64::new(1).expect("foreign dimension id is nonzero"),
        )),
        None
    );
    assert_eq!(
        evidence.phase_by_semantic(StorePhaseSemanticId::new(
            foreign_catalog,
            NonZeroU64::new(1).expect("foreign phase id is nonzero"),
        )),
        None
    );

    let revision_variant = StoreSemanticCatalogId::new(
        test_catalog().namespace(),
        NonZeroU32::new(test_catalog().revision().get() + 1).expect("revision variant is nonzero"),
    );
    assert_eq!(
        evidence.dimension_by_semantic(StoreDimensionSemanticId::new(
            revision_variant,
            NonZeroU64::new(1).expect("revision-variant dimension id is nonzero"),
        )),
        None,
        "same-namespace identities from another revision are not dimension evidence authority"
    );
    assert_eq!(
        evidence.phase_by_semantic(StorePhaseSemanticId::new(
            revision_variant,
            NonZeroU64::new(1).expect("revision-variant phase id is nonzero"),
        )),
        None,
        "same-namespace identities from another revision are not phase evidence authority"
    );
}

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

fn foreign_dimension() -> StoreDimensionKey {
    static KEY: OnceLock<StoreDimensionKey> = OnceLock::new();
    *KEY.get_or_init(|| {
        StoreKeyDomain::try_new(test_catalog())
            .expect("foreign dimension key domain")
            .try_admit_dimension(dimension_semantic(1))
            .expect("foreign dimension key")
    })
}

#[derive(Clone, Copy)]
struct SequenceTag;
#[derive(Clone, Copy)]
struct BytesTag;
#[derive(Clone, Copy)]
struct TextTag;
#[derive(Clone, Copy)]
struct CanonicalRootTag;
#[derive(Clone, Copy)]
struct EvidenceRootTag;
#[derive(Clone, Copy)]
struct EvidenceLeafTag;
#[derive(Clone, Copy)]
struct EvidenceBytesTag;
#[derive(Clone, Copy)]
struct EvidenceTextTag;
#[derive(Clone, Copy)]
struct RecordLeafTag;
#[derive(Clone, Copy)]
struct EvidenceRecordLeafTag;
#[derive(Clone, Copy)]
struct WrongTag;
#[derive(Clone, Copy)]
struct ScalarTag<const INDEX: u8>;

type TestOperation = RetainedStoreOperation<TestOwner, TestProgress, TestUnit>;
type SourceCanonical =
    baozi_operation::PublishedCanonicalStore<baozi_operation::RecordBlock, CanonicalRootTag>;
type SourceEvidence =
    baozi_operation::PublishedOutcomeEvidenceStore<baozi_operation::RecordBlock, EvidenceRootTag>;

struct PublishedSource {
    canonical: SourceCanonical,
    canonical_sequence: CanonicalSequenceRef<u32, SequenceTag>,
    canonical_bytes: baozi_operation::CanonicalBytesRef<BytesTag>,
    evidence: SourceEvidence,
    evidence_sequence: baozi_operation::OutcomeEvidenceSequenceRef<u8, EvidenceLeafTag>,
    evidence_text: baozi_operation::OutcomeEvidenceUtf8Ref<EvidenceTextTag>,
}

fn factory(max_accounts: u64) -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_accounts).unwrap(),
    )
    .unwrap()
}

fn account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap())
        .unwrap()
}

fn writer_plan(keys: TestKeys, attributions: usize, commitments: usize) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attributions, commitments).unwrap(),
    )
}

fn unbounded_failure_terminal_work(keys: TestKeys) -> RetainedStoreFailureTerminalWorkPlan {
    RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.failure_terminal_dimension,
        TEST_MAX_FAILURE_EVIDENCE_MEMBERS,
        TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT,
        TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT,
        u64::MAX,
        u64::MAX,
    )
    .unwrap()
}

fn operation(account: &PhysicalStorageAccount) -> (TestOperation, TestKeys) {
    operation_with_control(account, OperationControl::default())
}

fn operation_with_control(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> (TestOperation, TestKeys) {
    operation_with_control_and_evidence_limit(account, control, u64::MAX)
}

fn operation_with_evidence_limit(
    account: &PhysicalStorageAccount,
    evidence_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_control_and_evidence_limit(account, OperationControl::default(), evidence_limit)
}

#[cfg(feature = "_u4-activation")]
fn operation_with_commit_limit(
    account: &PhysicalStorageAccount,
    committed_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_catalog_and_limits(
        account,
        OperationControl::default(),
        test_catalog(),
        u64::MAX,
        committed_limit,
    )
}

fn operation_with_control_and_evidence_limit(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    evidence_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_catalog_and_evidence_limit(account, control, test_catalog(), evidence_limit)
}

fn operation_with_catalog_and_evidence_limit(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    catalog: StoreSemanticCatalogId,
    evidence_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_catalog_and_limits(account, control, catalog, evidence_limit, u64::MAX)
}

fn operation_with_catalog_and_limits(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    catalog: StoreSemanticCatalogId,
    evidence_limit: u64,
    committed_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_catalog_limits_and_failure_work(
        account,
        control,
        catalog,
        evidence_limit,
        committed_limit,
        TEST_FAILURE_EVIDENCE_CONSTRUCTION_WORK_LIMIT,
        TEST_FAILURE_EVIDENCE_VALIDATION_WORK_LIMIT,
    )
}

#[cfg(feature = "_u4-activation")]
fn operation_with_failure_work_limits(
    account: &PhysicalStorageAccount,
    construction_work_limit: u64,
    validation_work_limit: u64,
) -> (TestOperation, TestKeys) {
    operation_with_catalog_limits_and_failure_work(
        account,
        OperationControl::default(),
        test_catalog(),
        u64::MAX,
        u64::MAX,
        construction_work_limit,
        validation_work_limit,
    )
}

fn operation_with_catalog_limits_and_failure_work(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    catalog: StoreSemanticCatalogId,
    evidence_limit: u64,
    committed_limit: u64,
    construction_work_limit: u64,
    validation_work_limit: u64,
) -> (TestOperation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary_for(catalog);
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.failure_terminal_dimension,
        TEST_MAX_FAILURE_EVIDENCE_MEMBERS,
        construction_work_limit,
        validation_work_limit,
        u64::MAX,
        u64::MAX,
    )
    .unwrap();
    let policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work.ordinary_attempted_work_limit(),
        failure_terminal_work.ordinary_consumed_work_limit(),
        committed_limit,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(evidence_limit);
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
        TestOperation::try_start(control, ledger_plan, account, start).unwrap(),
        keys,
    )
}

fn publication_plan(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .unwrap(),
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, evidence_blocks + 1, 2),
            evidence_blocks,
            evidence_blocks,
        )
        .unwrap(),
    )
}

fn rich_failure_plan(keys: TestKeys, evidence_blocks: usize) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::rich_failure(
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, evidence_blocks + 1, 2),
            evidence_blocks,
            evidence_blocks,
        )
        .unwrap(),
    )
}

fn publication_plan_with_writer(writer: RetainedStoreWriterPlan) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::rich_failure(
        RetainedStoreBlockTablePlan::new(writer, 1, 1).unwrap(),
    )
}

fn reject_publication_plan_without_side_effects<F>(
    account: &PhysicalStorageAccount,
    build_plan: F,
) -> StoreRuntimeError
where
    F: FnOnce(TestKeys) -> RetainedStorePublicationPlan,
{
    let (operation, keys) = operation(account);
    let plan = build_plan(keys);
    let before = account.snapshot();
    let error = match operation.try_publication_assembly(plan) {
        Ok(assembly) => {
            drop(assembly);
            panic!("invalid publication keys must be rejected");
        }
        Err(error) => error,
    };
    assert_eq!(account.snapshot(), before);
    let recovery = error
        .into_disposition()
        .expect("key preflight must preserve the operation");
    assert!(recovery.is_retryable());
    let cause = recovery.cause().clone();
    assert_eq!(account.snapshot(), before);
    drop(recovery);
    cause
}

fn final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn physical_delta(
    before: PhysicalStorageAccountSnapshot,
    after: PhysicalStorageAccountSnapshot,
) -> (u64, u64) {
    (
        after
            .live_retained_bytes()
            .checked_sub(before.live_retained_bytes())
            .expect("retained bytes must grow monotonically within the measurement"),
        after
            .live_retained_allocations()
            .checked_sub(before.live_retained_allocations())
            .expect("retained allocations must grow monotonically within the measurement"),
    )
}

fn publish_sequence_source(account: &PhysicalStorageAccount) -> PublishedSource {
    let (operation, keys) = operation(account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 3, 3))
        .unwrap();
    let sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |writer| writer.push(11),
        )
        .unwrap();
    let canonical_bytes = assembly
        .try_retain_canonical_bytes(BytesPlan::new(2).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[12, 13])
        })
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_records(RecordPlan::new(1, 2).unwrap(), CanonicalRootTag, |writer| {
            writer.write_row(|row| {
                row.write_sequence_ref(sequence)?;
                row.write_bytes_ref(canonical_bytes)
            })
        })
        .unwrap();
    let evidence_leaf = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |writer| writer.push(7),
        )
        .unwrap();
    let evidence_text = assembly
        .try_retain_outcome_evidence_utf8(Utf8Plan::new(2).unwrap(), EvidenceTextTag, |writer| {
            writer.write_str("ok")
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 2).unwrap(),
            EvidenceRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(evidence_leaf)?;
                    row.write_utf8_ref(evidence_text)
                })
            },
        )
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        let Some(root) = session.canonical_records(c) else {
            return false;
        };
        let Some(sequence) = root.sequence_ref::<u32, SequenceTag>(0, 0) else {
            return false;
        };
        let Some(canonical_bytes) = root.bytes_ref::<BytesTag>(0, 1) else {
            return false;
        };
        let Some(evidence_root) = session.outcome_evidence_records(e) else {
            return false;
        };
        let Some(evidence_leaf) = evidence_root.sequence_ref::<u8, EvidenceLeafTag>(0, 0) else {
            return false;
        };
        let Some(evidence_text) = evidence_root.utf8_ref::<EvidenceTextTag>(0, 1) else {
            return false;
        };
        session
            .canonical_sequence(sequence)
            .and_then(|view| view.get(0))
            == Some(11)
            && session.canonical_bytes(canonical_bytes) == Some(&[12, 13][..])
            && session
                .outcome_evidence_sequence(evidence_leaf)
                .and_then(|view| view.get(0))
                == Some(7)
            && session.outcome_evidence_utf8(evidence_text) == Some("ok")
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("source must validate"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("source must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedSource {
        canonical,
        canonical_sequence: sequence,
        canonical_bytes,
        evidence: outcome_evidence,
        evidence_sequence: evidence_leaf,
        evidence_text,
    }
}

#[cfg(feature = "_u4-activation")]
#[path = "concrete_store/activation.rs"]
mod activation;
#[path = "concrete_store/construction.rs"]
mod construction;
#[path = "concrete_store/domain.rs"]
mod domain;
#[path = "concrete_store/evidence.rs"]
mod evidence;
#[path = "concrete_store/failure.rs"]
mod failure;
#[path = "concrete_store/publication.rs"]
mod publication;
#[path = "concrete_store/sharing.rs"]
mod sharing;
#[path = "concrete_store/transcript.rs"]
mod transcript;
