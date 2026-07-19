use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
use std::sync::OnceLock;

use baozi_operation::{
    AccountPoisonReason, BytesPlan, CancellationToken, CanonicalSequenceRef, DimensionPolicy,
    FinalProgress, LedgerCause, LedgerLimits, OperationControl, OperationOwner,
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PreparedStoreFailureOutcome,
    PreparedStoreSuccessOutcome, ProgressPhase, ProgressUnit, PublishedStoreFailureParts,
    PublishedStoreSuccessParts, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreOperation, RetainedStorePublicationPlan, RetainedStoreSharePlan,
    RetainedStoreStartError, RetainedStoreStartPlan, RetainedStoreWriterPlan, SequencePlan,
    StoreBuildFailureClass, StoreDimensionKey, StoreFailureValidation, StoreFallbackEnvelope,
    StoreKeyDomain, StoreLedgerPlan, StoreLedgerPlanError, StorePhaseKey,
    StorePreparationFailureClass, StorePrepareError, StoreRuntimeError, StoreSuccessValidation,
    Utf8Plan,
};

struct TestOwner;

impl OperationOwner for TestOwner {}

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

fn test_domain() -> &'static StoreKeyDomain {
    static DOMAIN: OnceLock<StoreKeyDomain> = OnceLock::new();
    DOMAIN.get_or_init(|| StoreKeyDomain::try_new().expect("test store key domain"))
}

fn foreign_domain() -> &'static StoreKeyDomain {
    static DOMAIN: OnceLock<StoreKeyDomain> = OnceLock::new();
    DOMAIN.get_or_init(|| StoreKeyDomain::try_new().expect("foreign test store key domain"))
}

fn test_phase() -> StorePhaseKey {
    test_domain().phase_key(NonZeroU32::new(1).expect("test phase local id"))
}

fn other_phase() -> StorePhaseKey {
    test_domain().phase_key(NonZeroU32::new(2).expect("other phase local id"))
}

fn foreign_phase() -> StorePhaseKey {
    foreign_domain().phase_key(NonZeroU32::new(1).expect("foreign phase local id"))
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

fn test_dimension() -> StoreDimensionKey {
    test_domain().dimension_key(NonZeroU32::new(1).expect("test dimension local id"))
}

fn other_dimension() -> StoreDimensionKey {
    test_domain().dimension_key(NonZeroU32::new(2).expect("other dimension local id"))
}

fn foreign_dimension() -> StoreDimensionKey {
    foreign_domain().dimension_key(NonZeroU32::new(1).expect("foreign dimension local id"))
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

fn writer_plan(attributions: usize, commitments: usize) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        test_phase(),
        test_dimension(),
        test_dimension(),
        RetainedStoreBatchLimits::new(attributions, commitments).unwrap(),
    )
}

fn operation(account: &PhysicalStorageAccount) -> TestOperation {
    operation_with_control(account, OperationControl::default())
}

fn operation_with_control(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
) -> TestOperation {
    operation_with_control_and_evidence_limit(account, control, u64::MAX)
}

fn operation_with_evidence_limit(
    account: &PhysicalStorageAccount,
    evidence_limit: u64,
) -> TestOperation {
    operation_with_control_and_evidence_limit(account, OperationControl::default(), evidence_limit)
}

fn operation_with_control_and_evidence_limit(
    account: &PhysicalStorageAccount,
    control: OperationControl<TestProgress, TestUnit>,
    evidence_limit: u64,
) -> TestOperation {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(evidence_limit);
    let limits = LedgerLimits::new(u64::MAX, [(test_dimension(), policy)]).unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(test_domain(), limits, &[test_phase()]).unwrap();
    let start = RetainedStoreStartPlan::new(
        writer_plan(32, 8),
        RetainedStoreSharePlan::new(test_dimension(), test_dimension()),
        StoreFallbackEnvelope::new(91),
        test_dimension(),
        1,
        test_dimension(),
        1,
    );
    TestOperation::try_start(control, ledger_plan, account, start).unwrap()
}

fn publication_plan(
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            writer_plan(canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .unwrap(),
        RetainedStoreBlockTablePlan::new(
            writer_plan(evidence_blocks + 1, 2),
            evidence_blocks,
            evidence_blocks,
        )
        .unwrap(),
    )
}

fn rich_failure_plan(evidence_blocks: usize) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::rich_failure(
        RetainedStoreBlockTablePlan::new(
            writer_plan(evidence_blocks + 1, 2),
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

fn reject_publication_plan_without_side_effects(
    account: &PhysicalStorageAccount,
    plan: RetainedStorePublicationPlan,
) -> StoreRuntimeError {
    let operation = operation(account);
    let before = account.snapshot();
    let error = match operation.try_publication_assembly(plan) {
        Ok(assembly) => {
            drop(assembly);
            panic!("invalid publication keys must be rejected");
        }
        Err(error) => error,
    };
    let cause = error.cause().clone();
    assert_eq!(account.snapshot(), before);
    let recovered = match error.try_into_operation() {
        Ok(operation) => operation,
        Err(cause) => panic!("key preflight must preserve the operation: {cause:?}"),
    };
    assert_eq!(account.snapshot(), before);
    drop(recovered);
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

#[test]
fn concrete_store_keys_are_fixed_inert_non_zero_values() {
    assert_eq!(
        std::mem::size_of::<StorePhaseKey>(),
        std::mem::size_of::<u128>()
    );
    assert_eq!(
        std::mem::size_of::<StoreDimensionKey>(),
        std::mem::size_of::<u128>()
    );
    assert_eq!(
        std::mem::size_of::<Option<StorePhaseKey>>(),
        std::mem::size_of::<u128>()
    );
    assert_eq!(
        std::mem::size_of::<Option<StoreDimensionKey>>(),
        std::mem::size_of::<u128>()
    );
    assert!(!std::mem::needs_drop::<StorePhaseKey>());
    assert!(!std::mem::needs_drop::<StoreDimensionKey>());
    assert!(!std::mem::needs_drop::<StoreKeyDomain>());
    assert_eq!(test_phase().local_id().get(), 1);
    assert_eq!(test_dimension().local_id().get(), 1);
    assert_ne!(test_phase(), foreign_phase());
    assert_ne!(test_dimension(), foreign_dimension());
    assert_eq!(
        format!("{:?}", test_domain()),
        format!("{:?}", foreign_domain())
    );
    assert_eq!(
        format!("{:?}", test_phase()),
        format!("{:?}", foreign_phase())
    );
    assert_eq!(
        format!("{:?}", test_dimension()),
        format!("{:?}", foreign_dimension())
    );
}

#[test]
fn concrete_store_key_hashing_exposes_only_the_public_local_identifier() {
    assert_eq!(
        hash_transcript(&test_phase()),
        hash_transcript(&foreign_phase())
    );
    assert_eq!(
        hash_transcript(&test_phase()),
        hash_transcript(&test_phase().local_id())
    );
    assert_ne!(
        hash_transcript(&test_phase()),
        hash_transcript(&other_phase())
    );
    assert_eq!(
        hash_transcript(&test_dimension()),
        hash_transcript(&foreign_dimension())
    );
    assert_eq!(
        hash_transcript(&test_dimension()),
        hash_transcript(&test_dimension().local_id())
    );
    assert_ne!(
        hash_transcript(&test_dimension()),
        hash_transcript(&other_dimension())
    );
}

#[test]
fn store_ledger_plan_rejects_incomplete_or_non_canonical_phase_catalogs() {
    let limits = || {
        LedgerLimits::new(
            16,
            [(
                test_dimension(),
                DimensionPolicy::reservable_bytes(16, 16, 16, 16),
            )],
        )
        .unwrap()
    };

    assert!(matches!(
        StoreLedgerPlan::try_new(test_domain(), limits(), &[]),
        Err(StoreLedgerPlanError::EmptyPhaseCatalog)
    ));
    assert!(matches!(
        StoreLedgerPlan::try_new(test_domain(), limits(), &[test_phase(), test_phase()]),
        Err(StoreLedgerPlanError::DuplicatePhase { phase }) if phase == test_phase()
    ));
    assert!(matches!(
        StoreLedgerPlan::try_new(test_domain(), limits(), &[other_phase(), test_phase()]),
        Err(StoreLedgerPlanError::PhaseOutOfOrder { previous, phase })
            if previous == other_phase() && phase == test_phase()
    ));
    assert!(matches!(
        StoreLedgerPlan::try_new(test_domain(), limits(), &[foreign_phase()]),
        Err(StoreLedgerPlanError::ForeignPhaseDomain { phase }) if phase == foreign_phase()
    ));
    let foreign_limits = LedgerLimits::new(
        16,
        [(
            foreign_dimension(),
            DimensionPolicy::reservable_bytes(16, 16, 16, 16),
        )],
    )
    .unwrap();
    assert!(matches!(
        StoreLedgerPlan::try_new(test_domain(), foreign_limits, &[test_phase()]),
        Err(StoreLedgerPlanError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));

    let plan =
        StoreLedgerPlan::try_new(test_domain(), limits(), &[test_phase(), other_phase()]).unwrap();
    assert_eq!(plan.dimension_count(), 1);
    assert_eq!(plan.phase_count(), 2);
}

#[test]
fn retained_store_start_rejects_phase_outside_frozen_catalog_without_retained_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let before = account.snapshot();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(test_dimension(), policy)]).unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(test_domain(), limits, &[test_phase()]).unwrap();
    let start = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            other_phase(),
            test_dimension(),
            test_dimension(),
            RetainedStoreBatchLimits::new(8, 4).unwrap(),
        ),
        RetainedStoreSharePlan::new(test_dimension(), test_dimension()),
        StoreFallbackEnvelope::new(91),
        test_dimension(),
        1,
        test_dimension(),
        1,
    );

    let rejected =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

    let cause = match rejected {
        Err(cause) => cause,
        Ok(operation) => {
            drop(operation);
            panic!("phase outside the frozen catalog must be rejected");
        }
    };
    assert!(
        matches!(
            cause,
            RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))
        ),
        "unexpected frozen phase rejection: {cause:?}"
    );
    assert_eq!(account.snapshot(), before);
}

#[test]
fn retained_store_start_preflights_every_dimension_key_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    for dimensions in [
        [
            other_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
        ],
        [
            test_dimension(),
            other_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
        ],
        [
            test_dimension(),
            test_dimension(),
            other_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
        ],
        [
            test_dimension(),
            test_dimension(),
            test_dimension(),
            other_dimension(),
            test_dimension(),
            test_dimension(),
        ],
        [
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            other_dimension(),
            test_dimension(),
        ],
        [
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            test_dimension(),
            other_dimension(),
        ],
    ] {
        let before = account.snapshot();
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let ledger_limits = LedgerLimits::new(u64::MAX, [(test_dimension(), policy)]).unwrap();
        let ledger_plan =
            StoreLedgerPlan::try_new(test_domain(), ledger_limits, &[test_phase()]).unwrap();
        let start = RetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(
                test_phase(),
                dimensions[0],
                dimensions[1],
                RetainedStoreBatchLimits::new(8, 4).unwrap(),
            ),
            RetainedStoreSharePlan::new(dimensions[2], dimensions[3]),
            StoreFallbackEnvelope::new(91),
            dimensions[4],
            1,
            dimensions[5],
            1,
        );

        let rejected =
            TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

        assert!(matches!(
            rejected,
            Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::UnknownDimension { dimension }
            ))) if dimension == other_dimension()
        ));
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn retained_store_start_rejects_foreign_phase_and_dimension_domains_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let cases = [
        (
            foreign_phase(),
            test_dimension(),
            StoreRuntimeError::ForeignPhaseDomain {
                phase: foreign_phase(),
            },
        ),
        (
            test_phase(),
            foreign_dimension(),
            StoreRuntimeError::ForeignDimensionDomain {
                dimension: foreign_dimension(),
            },
        ),
    ];

    for (phase, dimension, expected) in cases {
        let before = account.snapshot();
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(u64::MAX, [(test_dimension(), policy)]).unwrap();
        let ledger_plan = StoreLedgerPlan::try_new(test_domain(), limits, &[test_phase()]).unwrap();
        let start = RetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(
                phase,
                dimension,
                test_dimension(),
                RetainedStoreBatchLimits::new(8, 4).unwrap(),
            ),
            RetainedStoreSharePlan::new(test_dimension(), test_dimension()),
            StoreFallbackEnvelope::new(91),
            test_dimension(),
            1,
            test_dimension(),
            1,
        );

        let rejected =
            TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

        assert!(matches!(
            rejected,
            Err(RetainedStoreStartError::Runtime(cause)) if cause == expected
        ));
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn publication_key_preflight_rejects_unknown_phase_and_dimensions_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let limits = RetainedStoreBatchLimits::new(2, 2).unwrap();

    let phase_cause = reject_publication_plan_without_side_effects(
        &account,
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            other_phase(),
            test_dimension(),
            test_dimension(),
            limits,
        )),
    );
    assert!(matches!(
        phase_cause,
        StoreRuntimeError::Ledger(LedgerCause::UnknownPhase)
    ));

    for writer in [
        RetainedStoreWriterPlan::new(test_phase(), other_dimension(), test_dimension(), limits),
        RetainedStoreWriterPlan::new(test_phase(), test_dimension(), other_dimension(), limits),
    ] {
        let dimension_cause = reject_publication_plan_without_side_effects(
            &account,
            publication_plan_with_writer(writer),
        );
        assert!(matches!(
            dimension_cause,
            StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension })
                if dimension == other_dimension()
        ));
    }
}

#[test]
fn publication_key_preflight_rejects_foreign_domains_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let limits = RetainedStoreBatchLimits::new(2, 2).unwrap();

    let phase_cause = reject_publication_plan_without_side_effects(
        &account,
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            foreign_phase(),
            test_dimension(),
            test_dimension(),
            limits,
        )),
    );
    assert!(matches!(
        phase_cause,
        StoreRuntimeError::ForeignPhaseDomain { phase } if phase == foreign_phase()
    ));

    let dimension_cause = reject_publication_plan_without_side_effects(
        &account,
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            test_phase(),
            foreign_dimension(),
            test_dimension(),
            limits,
        )),
    );
    assert!(matches!(
        dimension_cause,
        StoreRuntimeError::ForeignDimensionDomain { dimension }
            if dimension == foreign_dimension()
    ));
}

#[test]
fn sequence_bytes_utf8_and_records_round_trip_through_typed_roots() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(4, 1))
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
        assembly.try_commit_canonical(other_dimension(), 13),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension }))
            if dimension == other_dimension()
    ));
    assert!(matches!(
        assembly.try_commit_outcome_evidence(foreign_dimension(), 13),
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension()
    ));
    assert!(matches!(
        assembly.try_commit_outcome_evidence(other_dimension(), 13),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension }))
            if dimension == other_dimension()
    ));
    assert_eq!(account.snapshot(), before_rejected_commits);

    assembly.try_commit_canonical(test_dimension(), 5).unwrap();
    assembly
        .try_commit_outcome_evidence(test_dimension(), 6)
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
    let snapshot = ledger_evidence
        .dimension(test_dimension())
        .expect("published dimension evidence");
    assert_eq!(snapshot.committed_canonical_output(), 5);
    assert_eq!(snapshot.published_outcome_evidence(), 7);
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sequence_bytes_and_utf8_roots_round_trip_through_every_terminal_owner() {
    let factory = factory(1);
    let account = account(&factory);

    {
        let mut assembly = operation(&account)
            .try_publication_assembly(publication_plan(1, 1))
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
        let mut assembly = operation(&account)
            .try_publication_assembly(publication_plan(1, 1))
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
        let mut assembly = operation(&account)
            .try_publication_assembly(publication_plan(1, 1))
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
        let mut assembly = operation(&account)
            .try_publication_assembly(rich_failure_plan(1))
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
        let mut assembly = operation(&account)
            .try_publication_assembly(rich_failure_plan(1))
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
        let mut assembly = operation(&account)
            .try_publication_assembly(rich_failure_plan(1))
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
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(5, 5))
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
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(11, 1))
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
    assert!(RecordPlan::new(1, 33).is_err());

    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(2, 1))
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
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(3, 1))
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

#[test]
fn builder_panic_is_fatal_and_cannot_publish_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap();
    let before = account.snapshot();
    let error = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u32>::new(1).unwrap(),
            SequenceTag,
            |_writer| -> Result<(), baozi_operation::StoreBuildError> {
                panic!("contained builder panic")
            },
        )
        .unwrap_err();
    assert!(matches!(error, StoreRuntimeError::BuilderPanicked));
    let after = account.snapshot();
    assert!(after.poisoned());
    assert_eq!(
        after
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert!(after.live_retained_bytes() > before.live_retained_bytes());
    assert!(after.live_retained_allocations() > before.live_retained_allocations());
    assert!(matches!(
        assembly.try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).unwrap(),
            EvidenceRootTag,
            |writer| writer.write_row(|row| row.write_scalar(1_u8)),
        ),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    for dimension in [foreign_dimension(), other_dimension()] {
        assert!(matches!(
            assembly.try_commit_canonical(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
        assert!(matches!(
            assembly.try_commit_outcome_evidence(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn failure_assembly_commit_preserves_fallback_first_error_precedence() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(1, 2))
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
    let mut failure = match assembly.validate_success(canonical, evidence, |_session, _, _| false) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must produce a failure assembly"),
    };
    let error = failure
        .try_retain_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |_writer| -> Result<(), baozi_operation::StoreBuildError> {
                panic!("contained failure builder panic")
            },
        )
        .unwrap_err();
    assert!(matches!(error, StoreRuntimeError::BuilderPanicked));

    for dimension in [foreign_dimension(), other_dimension()] {
        assert!(matches!(
            failure.try_commit(dimension, 1),
            Err(StoreRuntimeError::FallbackOnly)
        ));
    }
    assert!(matches!(
        failure.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedBuilderPanicked
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn semantic_rejection_can_publish_rich_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(1, 1))
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

    let failure = match assembly.validate_success(canonical, evidence, |_session, _, _| false) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must preserve an evidence-only builder"),
    };
    let validated = match failure.validate_rich_failure(evidence, |session, evidence| {
        session
            .records(evidence)
            .and_then(|records| records.scalar::<u8>(0, 0))
            == Some(7)
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("evidence-only graph must validate"),
    };
    let prepared = match validated.try_prepare(42, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared,
        PreparedStoreFailureOutcome::MinimalFallback(_) => {
            panic!("validated evidence must prepare rich failure")
        }
    };
    let PublishedStoreFailureParts {
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    assert_eq!(outcome_evidence.cause().code(), 42);
    assert_eq!(
        outcome_evidence
            .outcome_evidence()
            .root()
            .scalar::<u8>(0, 0),
        Some(7)
    );
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn success_primary_rejection_returns_the_exact_minimal_fallback_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation_with_evidence_limit(&account, 4)
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
            writer.push(1)
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).unwrap(),
            EvidenceLeafTag,
            |writer| writer.push(7),
        )
        .unwrap();
    assembly.try_commit_canonical(test_dimension(), 9).unwrap();
    assembly
        .try_commit_outcome_evidence(test_dimension(), 4)
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some() && session.outcome_evidence_sequence(e).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("success primary-rejection graph must validate"),
    };
    let physical_before_prepare = account.snapshot();
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::MinimalFallback(prepared) => prepared,
        PreparedStoreSuccessOutcome::Success(_) => {
            panic!("primary ledger rejection must not expose canonical success")
        }
        PreparedStoreSuccessOutcome::RichFailure(_) => {
            panic!("primary ledger rejection must not retry as rich failure")
        }
    };
    let physical_after_prepare = account.snapshot();
    assert!(
        physical_after_prepare.live_retained_bytes()
            < physical_before_prepare.live_retained_bytes()
    );
    assert!(
        physical_after_prepare.live_retained_allocations()
            < physical_before_prepare.live_retained_allocations()
    );

    let published = prepared.handoff();
    assert!(account.is_same_account(published.fallback.account()));
    assert_eq!(published.fallback.value().code(), 91);
    assert_eq!(
        published.fallback.cause().class(),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(published.fallback.cause().code(), 1);
    assert_eq!(
        published
            .ledger_evidence
            .dimension(test_dimension())
            .expect("success rejection ledger evidence")
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn rich_failure_primary_rejection_returns_the_exact_minimal_fallback_owner() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation_with_evidence_limit(&account, 4)
        .try_publication_assembly(rich_failure_plan(1))
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_bytes(BytesPlan::new(2).unwrap(), EvidenceBytesTag, |writer| {
            writer.write_bytes(&[7, 8])
        })
        .unwrap();
    assembly
        .try_commit_outcome_evidence(test_dimension(), 4)
        .unwrap();
    let validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
        session.bytes(evidence) == Some(&[7, 8][..])
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("rich-failure primary-rejection graph must validate"),
    };
    let physical_before_prepare = account.snapshot();
    let prepared = match validated.try_prepare(73, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::MinimalFallback(prepared) => prepared,
        PreparedStoreFailureOutcome::RichFailure(_) => {
            panic!("primary ledger rejection must not expose rich failure")
        }
    };
    let physical_after_prepare = account.snapshot();
    assert!(
        physical_after_prepare.live_retained_bytes()
            < physical_before_prepare.live_retained_bytes()
    );
    assert!(
        physical_after_prepare.live_retained_allocations()
            < physical_before_prepare.live_retained_allocations()
    );

    let published = prepared.handoff();
    assert!(account.is_same_account(published.fallback.account()));
    assert_eq!(published.fallback.value().code(), 91);
    assert_eq!(
        published.fallback.cause().class(),
        StorePreparationFailureClass::Ledger
    );
    assert_eq!(published.fallback.cause().code(), 1);
    assert_eq!(
        published
            .ledger_evidence
            .dimension(test_dimension())
            .expect("failure rejection ledger evidence")
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn final_cancellation_downgrades_validated_success_before_pure_handoff() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let mut assembly = operation_with_control(&account, control)
        .try_publication_assembly(publication_plan(1, 1))
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
    assembly.try_commit_canonical(test_dimension(), 9).unwrap();
    assembly
        .try_commit_outcome_evidence(test_dimension(), 6)
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
        outcome_evidence.cause().class(),
        StorePreparationFailureClass::Control
    );
    assert_eq!(outcome_evidence.cause().code(), 1);
    assert_eq!(
        outcome_evidence
            .outcome_evidence()
            .root()
            .scalar::<u8>(0, 0),
        Some(7)
    );
    let dimension = ledger_evidence
        .dimension(test_dimension())
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
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(1, 3))
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
    assembly.try_commit_canonical(test_dimension(), 9).unwrap();
    assembly
        .try_commit_outcome_evidence(test_dimension(), 6)
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
        failure.try_commit(other_dimension(), 11),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { dimension }))
            if dimension == other_dimension()
    ));
    assert_eq!(account.snapshot(), before_rejected_commits);
    failure.try_commit(test_dimension(), 4).unwrap();
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
    assert_eq!(outcome_evidence.cause().code(), 74);
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
        .dimension(test_dimension())
        .expect("downgrade ledger evidence");
    assert_eq!(dimension.committed_canonical_output(), 0);
    drop(outcome_evidence);
    drop(ledger_evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn downgrade_revalidation_panic_exposes_only_minimal_fallback() {
    let factory = factory(1);
    let account = account(&factory);
    let mut assembly = operation(&account)
        .try_publication_assembly(publication_plan(1, 1))
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
    assembly.try_commit_canonical(test_dimension(), 9).unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_sequence(c).is_some() && session.outcome_evidence_records(e).is_some()
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("success candidate must validate before downgrade"),
    };
    let failure = validated.downgrade_to_rich_failure(82);
    assert_eq!(failure.evidence_root(), evidence);
    let fallback_only = match failure.validate_rich_failure(evidence, |_session, _evidence| {
        panic!("contained downgrade validation panic")
    }) {
        StoreFailureValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("validation panic must leave only the pre-admitted fallback"),
    };
    let published = fallback_only.prepare_minimal_fallback().unwrap().handoff();
    assert_eq!(
        published.fallback.cause().class(),
        StorePreparationFailureClass::ProductGraph
    );
    assert_eq!(published.fallback.cause().code(), 5);
    let dimension = published
        .ledger_evidence
        .dimension(test_dimension())
        .expect("fallback downgrade ledger evidence");
    assert_eq!(dimension.committed_canonical_output(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

fn publish_sequence_source(account: &PhysicalStorageAccount) -> PublishedSource {
    let mut assembly = operation(account)
        .try_publication_assembly(publication_plan(3, 3))
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

#[test]
fn same_account_share_is_zero_physical_growth_and_foreign_or_stale_refs_fail() {
    let factory = factory(2);
    let source_account = account(&factory);
    let foreign_account = account(&factory);
    let source = publish_sequence_source(&source_account);

    let share_plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(writer_plan(2, 2), 3, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(writer_plan(2, 2), 3, 1).unwrap(),
    );
    let mut assembly = operation(&source_account)
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

    let mut stale_root = operation(&source_account)
        .try_publication_assembly(publication_plan(1, 1))
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

    let mut foreign = operation(&foreign_account)
        .try_publication_assembly(RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(writer_plan(1, 2), 1, 0).unwrap(),
            RetainedStoreBlockTablePlan::new(writer_plan(1, 2), 1, 0).unwrap(),
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

    let target_plan = || {
        RetainedStorePublicationPlan::success(
            RetainedStoreBlockTablePlan::new(writer_plan(1, 2), 1, 0).unwrap(),
            RetainedStoreBlockTablePlan::new(writer_plan(1, 2), 1, 0).unwrap(),
        )
    };
    let mut challenged = operation(&account)
        .try_publication_assembly(target_plan())
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
        .dimension(test_dimension())
        .expect("challenged work snapshot");
    drop(ledger_evidence);

    let mut control = operation(&account)
        .try_publication_assembly(target_plan())
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
        .dimension(test_dimension())
        .expect("control work snapshot");
    assert_eq!(challenged_work.attempted(), control_work.attempted());
    assert_eq!(challenged_work.consumed(), control_work.consumed());
    drop(ledger_evidence);

    drop(source_a);
    drop(source_b);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
