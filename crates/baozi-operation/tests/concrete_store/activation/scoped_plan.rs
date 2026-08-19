use super::*;

type ScopedOutcome<E> = CanonicalActivationOutcome<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    E,
    ScalarSequence<u8>,
    SequenceTag,
>;

type DerivedScopedOutcome<E> = CanonicalActivationOutcome<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    E,
    ScalarSequence<u32>,
    SequenceTag,
    CanonicalInheritanceSource,
>;

fn bind_test_execution<P>(
    requirements: &mut Option<CanonicalSuccessRequirements>,
    plan: P,
) -> TestActivationDescription<P> {
    let (identity, request) = mint_canonical_activation_request()
        .expect("the test activation request identity must remain available");
    TestActivationDescription::with_request(
        request,
        requirements
            .take()
            .expect("the test success requirements are consumed exactly once")
            .with_request_identity(identity),
        plan,
    )
}

fn bind_test_execution_with_request<P>(
    request: CanonicalActivationRequest,
    requirements: &mut Option<CanonicalSuccessRequirements>,
    plan: P,
) -> TestActivationDescription<P> {
    TestActivationDescription::with_request(
        request,
        requirements
            .take()
            .expect("the test success requirements are consumed exactly once"),
        plan,
    )
}

struct PanicSealDescription<'a> {
    dropped: &'a Cell<bool>,
}

impl Drop for PanicSealDescription<'_> {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

impl CanonicalActivationDescription for PanicSealDescription<'_> {
    type Execution = ();

    fn seal_activation<'scope>(
        self,
        _sealer: CanonicalActivationPlanSealer<'scope>,
    ) -> CanonicalSealedActivationPlan<'scope, Self::Execution> {
        panic!("contained trusted producer seal panic")
    }
}

fn session_activation_policy(keys: TestKeys) -> CanonicalActivationSessionPolicy {
    CanonicalActivationSessionPolicy::new(
        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
        CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
        CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
        keys.dimension,
        keys.dimension,
    )
}

#[test]
fn session_outcome_owner_samples_quiescence_after_producer_work() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let evidence_limits = RetainedStoreBatchLimits::new(2, 2).unwrap();
    let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap());
    let (request_identity, request) = mint_canonical_activation_request().unwrap();
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        2,
        1,
        NonZeroU64::MIN,
    )
    .unwrap()
    .with_request_identity(request_identity);
    let mut owner = operation
        .into_session_outcome_owner(session_activation_policy(keys), evidence_limits, failure)
        .expect("session outcome owner");

    owner
        .producer_host()
        .charge_observed_batch([(keys.dimension, 1)])
        .expect("producer work before Scene activation");
    let producer_reservation = owner
        .control_checkpoint()
        .reserve(keys.phase, keys.dimension, 1)
        .expect("producer reservation before Scene activation");
    let mut success = Some(success);
    let mut request = Some(request);

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(
                bind_test_execution_with_request(
                    request
                        .take()
                        .expect("the test activation request is consumed exactly once"),
                    &mut success,
                    (),
                ),
                completion,
            ) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
            permit.activate(|lane, _plan| {
                lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 73)
            })
        })
        .expect("producer work becomes the activation-entry baseline");
    drop(producer_reservation);
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization rejection must retain the session outcome owner")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn session_outcome_owner_rejects_success_facts_from_another_explicit_request() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let (_request_identity, request) = mint_canonical_activation_request().unwrap();
    let (foreign_identity, _foreign_request) = mint_canonical_activation_request().unwrap();
    let foreign_success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        2,
        1,
        NonZeroU64::MIN,
    )
    .unwrap()
    .with_request_identity(foreign_identity);
    let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap());
    let owner = operation
        .into_session_outcome_owner(
            session_activation_policy(keys),
            RetainedStoreBatchLimits::new(2, 2).unwrap(),
            failure,
        )
        .expect("session outcome owner");
    let mut foreign_success = Some(foreign_success);
    let mut request = Some(request);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.validate_live_owner(&scratch).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        match planning.seal_activation(
            bind_test_execution_with_request(
                request
                    .take()
                    .expect("the test activation request is consumed exactly once"),
                &mut foreign_success,
                plan,
            ),
            completion,
        ) {
            CanonicalActivationSeal::Sealed(_) => {
                panic!("success facts from another explicit request must not seal")
            }
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    });

    assert!(result.is_err());
    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn session_outcome_owner_rejects_a_request_foreign_to_its_failure_envelope() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let (failure_identity, _failure_request) = mint_canonical_activation_request().unwrap();
    let (request_identity, request) = mint_canonical_activation_request().unwrap();
    let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap())
        .with_request_identity(failure_identity);
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        2,
        1,
        NonZeroU64::MIN,
    )
    .unwrap()
    .with_request_identity(request_identity);
    let owner = operation
        .into_session_outcome_owner(
            session_activation_policy(keys),
            RetainedStoreBatchLimits::new(2, 2).unwrap(),
            failure,
        )
        .expect("session outcome owner");
    let mut success = Some(success);
    let mut request = Some(request);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.validate_live_owner(&scratch).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        match planning.seal_activation(
            bind_test_execution_with_request(
                request
                    .take()
                    .expect("the test activation request is consumed exactly once"),
                &mut success,
                plan,
            ),
            completion,
        ) {
            CanonicalActivationSeal::Sealed(_) => {
                panic!("a request foreign to the failure envelope must not seal")
            }
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    });

    assert!(result.is_err());
    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn session_outcome_owner_resolves_an_unsubmitted_request_without_success_capacity() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let owner = operation
        .into_session_outcome_owner(
            session_activation_policy(keys),
            RetainedStoreBatchLimits::new(2, 2).unwrap(),
            CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        )
        .expect("session outcome owner");

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .reject_without_activation(PreflightRejection::Unsupported, 11)
        .expect("unsubmitted request must resolve through the session failure baseline");
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(owner) => owner,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("an unsubmitted request must remain NeverActivated")
        }
    };
    let (prepared, cause) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    assert_eq!(
        cause,
        StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
    );
    let published = prepared.handoff();
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_owner_rejects_success_facts_from_another_construction() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let policy = TestActivationPolicy::new(
        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
        TestEvidenceWriter::new(keys, 2, 2),
        CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
        CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
        keys.dimension,
        keys.dimension,
    );
    let (failure_identity, _failure_request) = mint_canonical_activation_request().unwrap();
    let (success_identity, _success_request) = mint_canonical_activation_request().unwrap();
    let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap())
        .with_request_identity(failure_identity);
    let foreign_success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        2,
        1,
        NonZeroU64::MIN,
    )
    .unwrap()
    .with_request_identity(success_identity);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .expect("session owner");
    let mut foreign_success = Some(foreign_success);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.validate_live_owner(&scratch).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        match planning.seal_activation(bind_test_execution(&mut foreign_success, plan), completion)
        {
            CanonicalActivationSeal::Sealed(_) => {
                panic!("success facts from another construction must not seal")
            }
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    });

    assert!(result.is_err());
    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn dropping_dormant_session_outcome_owner_contains_the_operation() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let owner = operation
        .into_session_outcome_owner(
            session_activation_policy(keys),
            RetainedStoreBatchLimits::new(2, 2).unwrap(),
            CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        )
        .expect("session outcome owner");

    drop(owner);

    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
}

fn derived_operation_for_scoped_activation(
    source: &SourceCanonical,
) -> (
    DerivedRetainedStoreOperation<TestOwner, TestProgress, TestUnit>,
    TestKeys,
) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
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
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_terminal_work,
    );
    (
        TestOperation::try_start_derived(OperationControl::default(), ledger_plan, source, start)
            .unwrap(),
        keys,
    )
}

fn derived_operation_with_materialization_work_limit(
    source: &SourceCanonical,
    materialization_work_limit: u64,
) -> (
    DerivedRetainedStoreOperation<TestOwner, TestProgress, TestUnit>,
    TestKeys,
) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let operation_policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work
            .ordinary_attempted_work_limit()
            .checked_sub(materialization_work_limit)
            .expect("materialization work fits the ordinary attempted budget"),
        failure_terminal_work
            .ordinary_consumed_work_limit()
            .checked_sub(materialization_work_limit)
            .expect("materialization work fits the ordinary consumed budget"),
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let materialization_policy = DimensionPolicy::reservable_bytes(
        materialization_work_limit,
        materialization_work_limit,
        u64::MAX,
        u64::MAX,
    );
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, operation_policy),
            (keys.other_dimension, materialization_policy),
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
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_terminal_work,
    );
    (
        TestOperation::try_start_derived(OperationControl::default(), ledger_plan, source, start)
            .unwrap(),
        keys,
    )
}

fn publish_derived_rejection(
    outcome: DerivedScopedOutcome<MaterializationRejection>,
) -> baozi_operation::PublishedStoreFallbackParts {
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a requested derived rejection must retain its scoped owner")
        }
    };
    let (prepared, _cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    prepared.handoff()
}

fn operation_with_isolated_work_limit(
    account: &PhysicalStorageAccount,
    isolated_work_limit: u64,
) -> (TestOperation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let primary_attempted_limit = failure_terminal_work
        .ordinary_attempted_work_limit()
        .checked_sub(isolated_work_limit)
        .expect("the isolated dimension fits the ordinary attempted budget");
    let primary_consumed_limit = failure_terminal_work
        .ordinary_consumed_work_limit()
        .checked_sub(isolated_work_limit)
        .expect("the isolated dimension fits the ordinary consumed budget");
    let primary_policy = DimensionPolicy::reservable_bytes(
        primary_attempted_limit,
        primary_consumed_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let semantic_policy = DimensionPolicy::reservable_bytes(
        isolated_work_limit,
        isolated_work_limit,
        u64::MAX,
        u64::MAX,
    );
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, primary_policy),
            (keys.other_dimension, semantic_policy),
            (
                keys.failure_terminal_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )
    .expect("scoped semantic ledger limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase])
        .expect("scoped semantic ledger plan");
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, TEST_TERMINAL_ATTRIBUTION_SLOTS, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        TEST_FALLBACK_OUTCOME_EVIDENCE,
        keys.dimension,
        TEST_SNAPSHOT_OUTCOME_EVIDENCE,
        failure_terminal_work,
    );
    (
        TestOperation::try_start(OperationControl::default(), ledger_plan, account, start)
            .expect("scoped semantic operation"),
        keys,
    )
}

fn activation_with_semantic_bytes(keys: TestKeys, semantic_bytes: u64) -> TestScopedActivation {
    let activation = scoped_activation_shape(keys, 1, 1);
    TestScopedActivation {
        policy: activation
            .policy
            .with_semantic_bytes_dimension(keys.other_dimension),
        failure: activation.failure,
        success: activation.success.with_semantic_bytes(semantic_bytes),
    }
}

#[test]
fn activation_semantic_bytes_are_charged_exactly_before_materialization() {
    const SEMANTIC_BYTES: u64 = 3;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_isolated_work_limit(&account, SEMANTIC_BYTES);
    let materialized = Cell::new(false);

    let outcome: ScopedOutcome<MaterializationRejection> = plan_scoped_activation!(
        operation,
        activation_with_semantic_bytes(keys, SEMANTIC_BYTES),
        |lane| {
            materialized.set(true);
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                75,
            )
        },
    )
    .unwrap();

    assert!(materialized.get());
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("semantic-byte accounting must preserve the requested rejection")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let semantic_bytes = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("the semantic-byte dimension remains queryable");
    assert_eq!(
        (semantic_bytes.attempted(), semantic_bytes.consumed()),
        (SEMANTIC_BYTES, SEMANTIC_BYTES)
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn activation_semantic_byte_limit_fails_before_materialization_and_canonical_output() {
    const SEMANTIC_BYTES: u64 = 3;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_isolated_work_limit(&account, SEMANTIC_BYTES - 1);
    let materialized = Cell::new(false);

    let outcome: ScopedOutcome<MaterializationRejection> = plan_scoped_activation!(
        operation,
        activation_with_semantic_bytes(keys, SEMANTIC_BYTES),
        |lane| {
            materialized.set(true);
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                76,
            )
        },
    )
    .unwrap();

    assert!(!materialized.get());
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(never_activated) => never_activated,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("semantic-byte admission failure must remain NeverActivated")
        }
    };
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let semantic_bytes = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("the rejected semantic-byte dimension remains queryable");
    assert_eq!(
        (semantic_bytes.attempted(), semantic_bytes.consumed()),
        (SEMANTIC_BYTES, 0)
    );
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

struct ScratchPlan<'scope> {
    values: Vec<u8>,
    _scratch: CanonicalReservation<'scope, TestOwner>,
}

struct DropMarker<'a> {
    dropped: &'a Cell<bool>,
    account: &'a PhysicalStorageAccount,
    poison_seen_while_dropping: &'a Cell<bool>,
}

struct PanicDropProbe<'a>(&'a Cell<bool>);

impl Drop for PanicDropProbe<'_> {
    fn drop(&mut self) {
        self.0.set(true);
        panic!("injected scoped owner drop panic");
    }
}

impl Drop for DropMarker<'_> {
    fn drop(&mut self) {
        self.poison_seen_while_dropping
            .set(self.account.snapshot().poisoned());
        self.dropped.set(true);
    }
}

struct TrackedScratchPlan<'scope, 'probe> {
    _marker: DropMarker<'probe>,
    _scratch: CanonicalReservation<'scope, TestOwner>,
}

#[test]
fn derived_scoped_activation_shares_only_from_its_exact_source() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let source_baseline = account.snapshot();
    let (operation, keys) = derived_operation_for_scoped_activation(&source.canonical);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                let _shared = lane
                    .try_share_canonical(&source.canonical, source.canonical_sequence)
                    .unwrap();
                assert_eq!(lane.len(), 1);
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the requested derived rejection must preserve its scoped owner")
        }
    };
    let (prepared, cause) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    assert_eq!(
        cause,
        StoreCanonicalRejection::Requested(MaterializationRejection::Semantic)
    );
    let published = prepared.handoff();
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    let after_rejection = account.snapshot();
    assert_eq!(
        after_rejection.live_retained_bytes(),
        source_baseline.live_retained_bytes()
    );
    assert_eq!(
        after_rejection.live_retained_allocations(),
        source_baseline.live_retained_allocations()
    );
    assert!(!after_rejection.poisoned());
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_activation_accepts_destination_total_larger_than_new_capacity() {
    const DESTINATION_BLOCKS: usize = 2;

    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let source_baseline = account.snapshot();
    let (operation, keys) = derived_operation_for_scoped_activation(&source.canonical);
    let activation = TestScopedActivation {
        policy: TestActivationPolicy::new(
            RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
            TestEvidenceWriter::new(keys, 2, 2),
            CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
            CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
            keys.dimension,
            keys.dimension,
        ),
        failure: CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(DESTINATION_BLOCKS, 1).unwrap(),
            2,
            DESTINATION_BLOCKS,
            NonZeroU64::MIN,
        )
        .unwrap(),
    };
    let owner = operation
        .into_test_session_outcome_owner(activation.policy, activation.failure)
        .unwrap();
    let mut success = Some(activation.success);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                let _shared = lane
                    .try_share_canonical(&source.canonical, source.canonical_sequence)
                    .unwrap();
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u32>::new(1).unwrap(),
                        SequenceTag,
                        |writer| writer.push(29),
                    )
                    .unwrap();
                assert_eq!(lane.len(), DESTINATION_BLOCKS);
                lane.complete::<_, MaterializationRejection, _, _>(root, ())
            })
        })
        .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the derived destination must admit inherited and new canonical blocks")
        }
    };
    assert_eq!(activated.canonical_len(), DESTINATION_BLOCKS);

    let validation = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            74,
                        ))
                    },
                )
            },
        )
        .unwrap();
    let rejected = match validation {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the requested central rejection must retain the derived assembly")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    let after_rejection = account.snapshot();
    assert_eq!(
        after_rejection.live_retained_bytes(),
        source_baseline.live_retained_bytes()
    );
    assert_eq!(
        after_rejection.live_retained_allocations(),
        source_baseline.live_retained_allocations()
    );
    assert!(!after_rejection.poisoned());
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_sequence_rewrite_admits_exact_plan_work() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    // One semantic unit, three source-binding units, and two destination sequence elements.
    let (operation, keys) = derived_operation_with_materialization_work_limit(&source.canonical, 6);
    let activation = scoped_activation_shape_with_dimensions(
        keys,
        1,
        1,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    );
    let owner = operation
        .into_test_session_outcome_owner(activation.policy, activation.failure)
        .unwrap();
    let mut success = Some(activation.success);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                let rewritten = lane
                    .try_rewrite_sequence(
                        &source.canonical,
                        source.canonical_sequence,
                        SequencePlan::<u32>::new(2).unwrap(),
                        SequenceTag,
                        |writer| writer.extend_from_slice(&[21, 34]),
                    )
                    .unwrap();
                assert_ne!(rewritten, source.canonical_sequence);
                assert_eq!(lane.len(), 1);
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    let published = publish_derived_rejection(outcome);
    let materialization = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .unwrap();
    assert_eq!(materialization.attempted(), 6);
    assert_eq!(materialization.consumed(), 6);
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_sequence_rewrite_rejects_one_short_before_the_builder() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    // The two-element destination cannot fit after semantic work and exact source binding.
    let (operation, keys) = derived_operation_with_materialization_work_limit(&source.canonical, 5);
    let activation = scoped_activation_shape_with_dimensions(
        keys,
        1,
        1,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    );
    let owner = operation
        .into_test_session_outcome_owner(activation.policy, activation.failure)
        .unwrap();
    let mut success = Some(activation.success);
    let builder_called = Cell::new(false);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                assert!(matches!(
                    lane.try_rewrite_sequence(
                        &source.canonical,
                        source.canonical_sequence,
                        SequencePlan::<u32>::new(2).unwrap(),
                        SequenceTag,
                        |writer| {
                            builder_called.set(true);
                            writer.extend_from_slice(&[21, 34])
                        },
                    ),
                    Err(StoreRuntimeError::Ledger(_))
                ));
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    assert!(!builder_called.get());
    let published = publish_derived_rejection(outcome);
    let materialization = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .unwrap();
    assert_eq!(materialization.attempted(), 6);
    assert_eq!(materialization.consumed(), 4);
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_record_copy_maps_the_exact_source_prefix() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    // One semantic unit, three source-binding units, and two destination record cells.
    let (operation, keys) = derived_operation_with_materialization_work_limit(&source.canonical, 6);
    let activation = scoped_activation_shape_with_dimensions(
        keys,
        3,
        1,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    );
    let owner = operation
        .into_test_session_outcome_owner(activation.policy, activation.failure)
        .unwrap();
    let mut success = Some(activation.success);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                let _sequence = lane
                    .try_share_canonical(&source.canonical, source.canonical_sequence)
                    .unwrap();
                let _bytes = lane
                    .try_share_canonical(&source.canonical, source.canonical_bytes)
                    .unwrap();
                let _root = lane
                    .try_copy_mapped_records(
                        &source.canonical,
                        source.canonical.root_ref(),
                        RecordPlan::new(1, 2).unwrap(),
                        CanonicalRootTag,
                    )
                    .unwrap();
                assert_eq!(lane.len(), 3);
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    let published = publish_derived_rejection(outcome);
    let materialization = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .unwrap();
    assert_eq!(materialization.attempted(), 6);
    assert_eq!(materialization.consumed(), 6);
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_record_copy_appends_inside_the_destination_plan() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    // One semantic unit, three source-binding units, and four destination record cells.
    let (operation, keys) = derived_operation_with_materialization_work_limit(&source.canonical, 8);
    let activation = scoped_activation_shape_with_dimensions(
        keys,
        3,
        1,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    );
    let owner = operation
        .into_test_session_outcome_owner(activation.policy, activation.failure)
        .unwrap();
    let mut success = Some(activation.success);

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                lane.try_bind_canonical_source(&source.canonical).unwrap();
                let _sequence = lane
                    .try_share_canonical(&source.canonical, source.canonical_sequence)
                    .unwrap();
                let _bytes = lane
                    .try_share_canonical(&source.canonical, source.canonical_bytes)
                    .unwrap();
                let _root = lane
                    .try_copy_mapped_records_with_append(
                        &source.canonical,
                        source.canonical.root_ref(),
                        RecordPlan::new(1, 2).unwrap(),
                        RecordPlan::new(2, 2).unwrap(),
                        CanonicalRootTag,
                        |writer| {
                            writer.write_row(|row| {
                                row.write_scalar(55_u32)?;
                                row.write_scalar(89_u32)
                            })
                        },
                    )
                    .unwrap();
                assert_eq!(lane.len(), 3);
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    let published = publish_derived_rejection(outcome);
    let materialization = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .unwrap();
    assert_eq!(materialization.attempted(), 8);
    assert_eq!(materialization.consumed(), 8);
    drop(published);
    drop(source);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn derived_scoped_activation_contains_a_same_shape_sibling_source() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let sibling = publish_sequence_source(&account);
    assert_eq!(source.canonical.len(), sibling.canonical.len());
    let (operation, keys) = derived_operation_for_scoped_activation(&source.canonical);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                assert!(matches!(
                    lane.try_bind_canonical_source(&sibling.canonical),
                    Err(StoreRuntimeError::InvalidSourceSlot)
                ));
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        });

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a foreign source fault must not become a requested result"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_fail_closed_containment(&account);
    drop(source);
    drop(sibling);
}

#[test]
fn recoverable_materialization_fault_then_sibling_bind_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let sibling = publish_sequence_source(&account);
    let (operation, keys) = derived_operation_for_scoped_activation(&source.canonical);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                let _ = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(1)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                assert!(matches!(
                    lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(2)?;
                            Ok(())
                        },
                    ),
                    Err(StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical,
                    })
                ));
                assert!(matches!(
                    lane.try_bind_canonical_source(&sibling.canonical),
                    Err(StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical,
                    })
                ));
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        });

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a sibling source after a recoverable fault must be containment"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_fail_closed_containment(&account);
    drop(source);
    drop(sibling);
}

#[test]
fn recoverable_materialization_fault_then_sibling_share_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let source = publish_sequence_source(&account);
    let sibling = publish_sequence_source(&account);
    let (operation, keys) = derived_operation_for_scoped_activation(&source.canonical);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| {
                let _ = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(1)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                assert!(matches!(
                    lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(2)?;
                            Ok(())
                        },
                    ),
                    Err(StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical,
                    })
                ));
                assert!(matches!(
                    lane.try_share_canonical(&source.canonical, sibling.canonical_sequence),
                    Err(StoreRuntimeError::SlotLimit {
                        destination: StoreDestination::Canonical,
                    })
                ));
                lane.reject::<(), _, ScalarSequence<u32>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        });

    let containment = match result {
        Err(containment) => containment,
        Ok(_) => panic!("a sibling source ref after a recoverable fault must be containment"),
    };
    assert_eq!(
        containment.cause().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_fail_closed_containment(&account);
    drop(source);
    drop(sibling);
}

#[test]
fn scoped_activation_permit_retains_the_core_plan_through_activation() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    // Operation verifies semantic completion and physical shape without interpreting the
    // core-owned execution plan. Activation retains it until the materializer returns or panics.
    let outcome = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let mut scratch = semantic.reserve(4).unwrap();
            semantic.resize(&mut scratch, 8).unwrap();
            semantic.validate_live_owner(&scratch).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();

            let plan = ScratchPlan {
                values: vec![3, 5, 8],
                _scratch: scratch,
            };
            let permit = match planning
                .seal_activation(bind_test_execution(&mut success, plan), completion)
            {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
            permit.activate(|lane, plan| {
                assert_eq!(plan.values, [3, 5, 8]);
                lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    21,
                )
            })
        })
        .unwrap();

    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a materialization rejection must discard the canonical lane")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let ordinary = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("scoped scratch accounting remains queryable");
    assert_eq!(ordinary.current_live_bytes(), 0);
    assert!(ordinary.peak_live_bytes() >= 8);
    drop(published);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn scoped_semantic_rejection_binds_only_failure_shape_and_never_allocates_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let after_start = account.snapshot();
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|planning| {
            assert_eq!(account.snapshot(), after_start);
            planning.reject(PreflightRejection::Unsupported, 11)
        })
        .unwrap();
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("semantic rejection must remain NeverActivated")
        }
    };
    assert_eq!(account.snapshot(), after_start);

    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
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
fn scoped_activation_permit_and_semantic_completion_are_linear() {
    trait AmbiguousIfCopy<Marker> {
        fn assert_not_copy() {}
    }
    impl<T: ?Sized> AmbiguousIfCopy<()> for T {}
    struct CopyMarker;
    impl<T: Copy> AmbiguousIfCopy<CopyMarker> for T {}

    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }
    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    struct CloneMarker;
    impl<T: Clone> AmbiguousIfClone<CloneMarker> for T {}

    type Permit = CanonicalActivationPermit<
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    type Completion = CanonicalSemanticCompletion<'static>;
    type SealedPlan = CanonicalSealedActivationPlan<'static, ()>;
    type SealFailure = CanonicalActivationSealFailure<
        'static,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    type Receipt = CanonicalScopedDisposition<'static>;

    let _ = <Permit as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Permit as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Completion as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Completion as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <SealedPlan as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <SealedPlan as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <SealFailure as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <SealFailure as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Receipt as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Receipt as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn dropping_an_unconsumed_scoped_outcome_owner_fails_closed() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    drop(owner);

    assert_fail_closed_containment(&account);
}

#[test]
fn abandoning_a_scoped_activation_releases_scratch_before_containment() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        let permit =
            match planning.seal_activation(bind_test_execution(&mut success, plan), completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
        drop(permit);
        panic!("abandoning an activation permit cannot synthesize a publishable outcome")
    });

    assert!(result.is_err());
    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert!(account.snapshot().poisoned());
}

#[derive(Clone, Copy)]
enum ScopedRoleFault {
    CanonicalWriter,
    EvidenceWriter,
    MaterializationWork,
    MaterializationScratch,
    ValidationPhase,
    ValidationWork,
    ValidationScratch,
    CanonicalDestination,
    EvidenceDestination,
}

#[test]
fn scoped_constructor_validates_every_role_before_authoring_authority_exists() {
    for fault in [
        ScopedRoleFault::CanonicalWriter,
        ScopedRoleFault::EvidenceWriter,
        ScopedRoleFault::MaterializationWork,
        ScopedRoleFault::MaterializationScratch,
        ScopedRoleFault::ValidationPhase,
        ScopedRoleFault::ValidationWork,
        ScopedRoleFault::ValidationScratch,
        ScopedRoleFault::CanonicalDestination,
        ScopedRoleFault::EvidenceDestination,
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let (operation, keys) = operation(&account);
        let foreign_writer_keys = TestKeys {
            phase: foreign_phase(),
            dimension: foreign_dimension(),
            ..keys
        };
        let canonical_writer_keys = if matches!(fault, ScopedRoleFault::CanonicalWriter) {
            foreign_writer_keys
        } else {
            keys
        };
        let policy = TestActivationPolicy::new(
            RetainedStoreWriterRole::new(
                canonical_writer_keys.phase,
                canonical_writer_keys.dimension,
                canonical_writer_keys.dimension,
            ),
            TestEvidenceWriter::new(
                if matches!(fault, ScopedRoleFault::EvidenceWriter) {
                    foreign_writer_keys
                } else {
                    keys
                },
                2,
                2,
            ),
            CanonicalMaterializationRole::new(
                if matches!(fault, ScopedRoleFault::MaterializationWork) {
                    foreign_dimension()
                } else {
                    keys.dimension
                },
                if matches!(fault, ScopedRoleFault::MaterializationScratch) {
                    foreign_dimension()
                } else {
                    keys.dimension
                },
            ),
            CanonicalValidationRole::new(
                if matches!(fault, ScopedRoleFault::ValidationPhase) {
                    foreign_phase()
                } else {
                    keys.phase
                },
                if matches!(fault, ScopedRoleFault::ValidationWork) {
                    foreign_dimension()
                } else {
                    keys.dimension
                },
                if matches!(fault, ScopedRoleFault::ValidationScratch) {
                    foreign_dimension()
                } else {
                    keys.dimension
                },
            ),
            if matches!(fault, ScopedRoleFault::CanonicalDestination) {
                foreign_dimension()
            } else {
                keys.dimension
            },
            if matches!(fault, ScopedRoleFault::EvidenceDestination) {
                foreign_dimension()
            } else {
                keys.dimension
            },
        );
        let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap());

        let result = operation.into_test_session_outcome_owner(policy, failure);

        assert!(result.is_err());
        assert_fail_closed_containment(&account);
    }
}

#[test]
fn scoped_materialization_panic_drops_the_plan_before_account_containment() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        match planning.seal_activation(bind_test_execution(&mut success, plan), completion) {
            CanonicalActivationSeal::Sealed(permit) => permit.activate(
                |_lane, _plan| -> CanonicalMaterialization<
                    '_,
                    (),
                    (),
                    ScalarSequence<u8>,
                    SequenceTag,
                > { panic!("contained scoped materialization panic") },
            ),
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    });

    assert!(result.is_err());
    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert!(account.snapshot().poisoned());
}

fn assert_fail_closed_containment(account: &PhysicalStorageAccount) {
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert!(snapshot.poison_event_count() > 0);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
}

fn assert_control_never_activated<E>(outcome: ScopedOutcome<E>, account: &PhysicalStorageAccount) {
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a final planning control fault must remain NeverActivated")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn panic_after_pending_never_activated_is_fail_closed() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<PreflightRejection>, _> =
        owner.plan_and_activate_scoped(|planning| {
            let _receipt = planning.reject(PreflightRejection::Unsupported, 31);
            panic!("panic after installing pending NeverActivated")
        });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn panic_after_pending_rejected_is_fail_closed() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            let _receipt = permit.activate(|lane, _plan| {
                lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                    MaterializationRejection::Semantic,
                    32,
                )
            });
            panic!("panic after installing pending Rejected")
        });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn panic_after_pending_activated_is_fail_closed() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            let _receipt = permit.activate(|mut lane, _plan| {
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer| {
                            writer.push(7)?;
                            Ok(())
                        },
                    )
                    .unwrap();
                lane.complete::<(), MaterializationRejection, _, _>(root, ())
            });
            panic!("panic after installing pending Activated")
        });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn control_interruption_then_reject_preserves_runtime_never_activated() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    cancellation.cancel();
    let action_executed = Cell::new(false);

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let cause = semantic.run_unit(|| action_executed.set(true)).unwrap_err();
            assert!(matches!(cause, StoreRuntimeError::Control(_)));
            assert!(!action_executed.get());
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Control(_))
            ));
            planning.reject(PreflightRejection::Unsupported, 41)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("trusted Control must remain recoverable NeverActivated")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    drop(prepared.handoff());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ledger_interruption_then_reject_preserves_runtime_never_activated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let cause = match semantic.reserve(u64::MAX) {
                Ok(reservation) => {
                    let cause = match semantic.reserve(1) {
                        Ok(_) => panic!("the account cannot reserve beyond u64::MAX live bytes"),
                        Err(cause) => cause,
                    };
                    drop(reservation);
                    cause
                }
                Err(cause) => cause,
            };
            assert!(matches!(cause, StoreRuntimeError::Ledger(_)));
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Ledger(_))
            ));
            planning.reject(PreflightRejection::Unsupported, 42)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("trusted Ledger must remain recoverable NeverActivated")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Ledger(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    drop(prepared.handoff());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn ledger_rejection_before_activation_skips_materialization_and_prepares_failure() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    assert!(
        operation
            .producer_host()
            .charge_attempted(keys.dimension, u64::MAX)
            .is_err()
    );
    let materialized = Cell::new(false);

    let outcome: ScopedOutcome<()> =
        run_scoped_activation(operation, scoped_activation_shape(keys, 1, 1), |lane| {
            materialized.set(true);
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>((), 43)
        })
        .unwrap();

    assert!(!materialized.get());
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a pre-existing ledger rejection must prevent canonical activation")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::SuccessOutcomeRejected)
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Ledger
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn runtime_fault_before_semantic_completion_resolves_without_activation() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    cancellation.cancel();

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let semantic = planning.begin_semantic_work().unwrap();
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Control(_))
            ));
            planning.reject((), 42)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("runtime seal failure must remain NeverActivated")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    drop(prepared.handoff());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn forgetting_a_semantic_session_cannot_publish_a_requested_disposition() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<PreflightRejection>, _> =
        owner.plan_and_activate_scoped(|mut planning| {
            let semantic = planning.begin_semantic_work().unwrap();
            std::mem::forget(semantic);
            planning.reject(PreflightRejection::Unsupported, 43)
        });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[derive(Clone, Copy)]
enum ForgottenReservationTerminal {
    Activated,
    Rejected,
}

fn assert_forgotten_reservation_is_contained(terminal: ForgottenReservationTerminal) {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let scratch = semantic.reserve(1).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            std::mem::forget(scratch);
            let permit =
                match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                    CanonicalActivationSeal::Sealed(permit) => permit,
                    CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                };
            permit.activate(|mut lane, _plan| match terminal {
                ForgottenReservationTerminal::Activated => {
                    let root = lane
                        .try_retain_sequence(
                            SequencePlan::<u8>::new(1).unwrap(),
                            SequenceTag,
                            |writer| writer.push(7),
                        )
                        .unwrap();
                    lane.complete::<(), MaterializationRejection, _, _>(root, ())
                }
                ForgottenReservationTerminal::Rejected => lane
                    .reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                        MaterializationRejection::Semantic,
                        44,
                    ),
            })
        });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn forgetting_a_detached_reservation_cannot_publish_activated() {
    assert_forgotten_reservation_is_contained(ForgottenReservationTerminal::Activated);
}

#[test]
fn forgetting_a_detached_reservation_cannot_publish_rejected() {
    assert_forgotten_reservation_is_contained(ForgottenReservationTerminal::Rejected);
}

fn scoped_semantic_case(execute_limit: u64, cancel_after: Option<u64>) -> (u64, u64, u64, bool) {
    let baseline = scoped_semantic_baseline();
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let executed = Cell::new(0_u64);
    let interrupted = Cell::new(false);

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            while executed.get() < execute_limit {
                let result = semantic.run_unit(|| {
                    let completed = executed.get() + 1;
                    executed.set(completed);
                    if cancel_after == Some(completed) {
                        cancellation.cancel();
                    }
                });
                if result.is_err() {
                    interrupted.set(true);
                    break;
                }
            }
            let finish = semantic.finish_rejected();
            if interrupted.get() {
                assert!(matches!(finish, Err(StoreRuntimeError::Control(_))));
            } else {
                finish.unwrap();
            }
            planning.reject(PreflightRejection::Unsupported, 51)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("semantic work cases remain pre-activation outcomes")
        }
    };
    let runtime = matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(_)
    );
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("scoped semantic work remains queryable");
    let result = (
        work.attempted()
            .checked_sub(baseline.0)
            .expect("semantic attempted work includes the startup baseline"),
        work.consumed()
            .checked_sub(baseline.1)
            .expect("semantic consumed work includes the startup baseline"),
        executed.get(),
        runtime,
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
    result
}

fn scoped_semantic_baseline() -> (u64, u64) {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|planning| planning.reject(PreflightRejection::Unsupported, 50))
        .unwrap();
    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the semantic baseline must remain pre-activation")
        }
    };
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("the semantic baseline dimension remains queryable");
    let baseline = (work.attempted(), work.consumed());
    drop(published);
    assert!(!account.snapshot().poisoned());
    baseline
}

#[test]
fn finish_rejected_preserves_the_exact_partial_semantic_prefix_without_contract_fault() {
    assert_eq!(scoped_semantic_case(1, None), (1, 1, 1, false));
}

#[test]
fn semantic_work_is_admitted_per_action_at_exact_one_short_and_zero_capacity() {
    const REQUESTED_UNITS: u64 = 2;

    for (
        semantic_work_limit,
        expected_executed,
        expected_attempted,
        expected_consumed,
        expected_cause,
    ) in [
        (
            REQUESTED_UNITS,
            REQUESTED_UNITS,
            REQUESTED_UNITS,
            REQUESTED_UNITS,
            StorePreparationFailureClass::Requested,
        ),
        (
            1,
            1,
            REQUESTED_UNITS,
            1,
            StorePreparationFailureClass::Ledger,
        ),
        (0, 0, 1, 0, StorePreparationFailureClass::Ledger),
    ] {
        let factory = factory(1);
        let account = account(&factory);
        let (operation, keys) = operation_with_isolated_work_limit(&account, semantic_work_limit);
        let TestScopedActivation {
            policy,
            failure,
            success: _,
        } = scoped_activation_shape_with_dimensions(
            keys,
            1,
            1,
            keys.other_dimension,
            keys.dimension,
            keys.dimension,
            keys.dimension,
        );
        let owner = operation
            .into_test_session_outcome_owner(policy, failure)
            .unwrap();
        let executed = Cell::new(0_u64);

        let outcome: ScopedOutcome<PreflightRejection> = owner
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                let mut admission_failed = false;
                for _ in 0..REQUESTED_UNITS {
                    match semantic.run_unit(|| executed.set(executed.get() + 1)) {
                        Ok(()) => {}
                        Err(StoreRuntimeError::Ledger(_)) => {
                            admission_failed = true;
                            break;
                        }
                        Err(cause) => panic!("unexpected semantic-work failure: {cause:?}"),
                    }
                }
                let finish = semantic.finish_rejected();
                if admission_failed {
                    assert!(matches!(finish, Err(StoreRuntimeError::Ledger(_))));
                } else {
                    finish.unwrap();
                }
                planning.reject(PreflightRejection::Unsupported, 56)
            })
            .unwrap();

        assert_eq!(executed.get(), expected_executed);
        let never_activated = match outcome {
            CanonicalActivationOutcome::NeverActivated(state) => state,
            CanonicalActivationOutcome::Activated(_)
            | CanonicalActivationOutcome::Rejected(_)
            | CanonicalActivationOutcome::Fallback(_) => {
                panic!("semantic work admission must remain pre-activation")
            }
        };
        let (prepared, _) = never_activated
            .prepare_minimal_fallback()
            .unwrap()
            .into_parts();
        let published = prepared.handoff();
        let semantic_work = published
            .ledger_evidence
            .dimension(keys.other_dimension)
            .expect("the isolated semantic dimension remains queryable");
        assert_eq!(semantic_work.attempted(), expected_attempted);
        assert_eq!(semantic_work.consumed(), expected_consumed);
        assert_eq!(
            store_failure_cause(&published.ledger_evidence).class(),
            expected_cause
        );
        drop(published);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn semantic_domain_rejection_consumes_the_returned_action_unit() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_isolated_work_limit(&account, 1);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape_with_dimensions(
        keys,
        1,
        1,
        keys.other_dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    );
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let rejection = semantic
                .run_unit(|| Err::<(), _>("invalid domain value"))
                .unwrap();
            assert_eq!(rejection, Err("invalid domain value"));
            semantic.finish_rejected().unwrap();
            planning.reject(PreflightRejection::Unsupported, 57)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a returned semantic rejection must remain pre-activation")
        }
    };
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let semantic_work = published
        .ledger_evidence
        .dimension(keys.other_dimension)
        .expect("the rejected semantic action remains queryable");
    assert_eq!(semantic_work.attempted(), 1);
    assert_eq!(semantic_work.consumed(), 1);
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Requested
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn caught_semantic_action_panic_cannot_be_downgraded_to_requested() {
    for completed_prefix in [0_u64, 1] {
        let factory = factory(1);
        let account = account(&factory);
        let (operation, keys) = operation_with_isolated_work_limit(&account, 2);
        let TestScopedActivation {
            policy,
            failure,
            success: _,
        } = scoped_activation_shape_with_dimensions(
            keys,
            1,
            1,
            keys.other_dimension,
            keys.dimension,
            keys.dimension,
            keys.dimension,
        );
        let owner = operation
            .into_test_session_outcome_owner(policy, failure)
            .unwrap();
        let completed = Cell::new(0_u64);

        let result: Result<ScopedOutcome<PreflightRejection>, _> =
            owner.plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                if completed_prefix == 1 {
                    semantic
                        .run_unit(|| completed.set(completed.get() + 1))
                        .unwrap();
                }
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = semantic.run_unit(|| -> () { panic!("caught semantic action panic") });
                }));
                assert!(panicked.is_err());
                assert!(matches!(
                    semantic.finish_rejected(),
                    Err(StoreRuntimeError::ConstructionInvariantUnverified)
                ));
                planning.reject(PreflightRejection::Unsupported, 58)
            });

        assert!(result.is_err());
        assert_eq!(completed.get(), completed_prefix);
        assert_fail_closed_containment(&account);
    }
}

#[test]
fn semantic_work_records_exact_255_256_and_cancelled_257_prefixes() {
    assert_eq!(scoped_semantic_case(255, None), (255, 255, 255, false));
    assert_eq!(scoped_semantic_case(256, None), (256, 256, 256, false));
    assert_eq!(scoped_semantic_case(257, Some(256)), (256, 256, 256, true));
}

#[test]
fn cancellation_in_the_last_semantic_action_precedes_requested_rejection() {
    let baseline = scoped_semantic_baseline();
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| cancellation.cancel()).unwrap();
            let cause = semantic.finish_rejected().unwrap_err();
            assert!(matches!(cause, StoreRuntimeError::Control(_)));
            planning.reject(PreflightRejection::Unsupported, 52)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the final controlled poll must remain NeverActivated")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("the final semantic action remains visible in ledger evidence");
    assert_eq!(
        work.attempted()
            .checked_sub(baseline.0)
            .expect("semantic attempted work includes the startup baseline"),
        1
    );
    assert_eq!(
        work.consumed()
            .checked_sub(baseline.1)
            .expect("semantic consumed work includes the startup baseline"),
        1
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn due_control_poll_precedes_scratch_reservation_after_the_checkpoint() {
    let baseline = scoped_semantic_baseline();
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            for index in 0..256 {
                semantic
                    .run_unit(|| {
                        if index == 255 {
                            cancellation.cancel();
                        }
                    })
                    .unwrap();
            }
            let before_scratch = account.snapshot();
            assert!(matches!(
                semantic.reserve(1),
                Err(StoreRuntimeError::Control(_))
            ));
            let after_scratch = account.snapshot();
            assert_eq!(
                after_scratch.live_retained_allocations(),
                before_scratch.live_retained_allocations()
            );
            assert_eq!(
                after_scratch.live_retained_bytes(),
                before_scratch.live_retained_bytes()
            );
            assert_eq!(
                after_scratch.total_admitted_retained_allocations(),
                before_scratch.total_admitted_retained_allocations()
            );
            assert_eq!(
                after_scratch.peak_retained_allocations(),
                before_scratch.peak_retained_allocations()
            );
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Control(_))
            ));
            planning.reject(PreflightRejection::Unsupported, 54)
        })
        .unwrap();

    let never_activated = match outcome {
        CanonicalActivationOutcome::NeverActivated(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a due cancellation must stop scratch before activation")
        }
    };
    assert!(matches!(
        never_activated.cause(),
        StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(_))
    ));
    let (prepared, _) = never_activated
        .prepare_minimal_fallback()
        .unwrap()
        .into_parts();
    let published = prepared.handoff();
    let work = published
        .ledger_evidence
        .dimension(keys.dimension)
        .expect("checkpointed semantic work remains queryable");
    assert_eq!(
        work.attempted()
            .checked_sub(baseline.0)
            .expect("semantic attempted work includes the startup baseline"),
        256
    );
    assert_eq!(
        work.consumed()
            .checked_sub(baseline.1)
            .expect("semantic consumed work includes the startup baseline"),
        256
    );
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn due_control_poll_precedes_live_owner_validation_after_the_checkpoint() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let scratch = semantic.reserve(4).unwrap();
            for index in 0..256 {
                semantic
                    .run_unit(|| {
                        if index == 255 {
                            cancellation.cancel();
                        }
                    })
                    .unwrap();
            }

            let before_validation = account.snapshot();
            assert!(matches!(
                semantic.validate_live_owner(&scratch),
                Err(StoreRuntimeError::Control(_))
            ));
            let after_validation = account.snapshot();
            assert_eq!(
                after_validation.live_retained_bytes(),
                before_validation.live_retained_bytes()
            );
            assert_eq!(
                after_validation.live_retained_allocations(),
                before_validation.live_retained_allocations()
            );
            assert_eq!(
                after_validation.total_admitted_retained_bytes(),
                before_validation.total_admitted_retained_bytes()
            );
            assert_eq!(
                after_validation.total_admitted_retained_allocations(),
                before_validation.total_admitted_retained_allocations()
            );
            assert_eq!(
                after_validation.peak_retained_bytes(),
                before_validation.peak_retained_bytes()
            );
            assert_eq!(
                after_validation.peak_retained_allocations(),
                before_validation.peak_retained_allocations()
            );
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Control(_))
            ));
            drop(scratch);
            planning.reject(PreflightRejection::Unsupported, 60)
        })
        .unwrap();

    assert_control_never_activated(outcome, &account);
}

#[test]
fn due_control_poll_precedes_scratch_resize_after_the_checkpoint() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let mut scratch = semantic.reserve(4).unwrap();
            for index in 0..256 {
                semantic
                    .run_unit(|| {
                        if index == 255 {
                            cancellation.cancel();
                        }
                    })
                    .unwrap();
            }

            let before_resize = account.snapshot();
            assert!(matches!(
                semantic.resize(&mut scratch, 8),
                Err(StoreRuntimeError::Control(_))
            ));
            let after_resize = account.snapshot();
            assert_eq!(
                after_resize.live_retained_bytes(),
                before_resize.live_retained_bytes()
            );
            assert_eq!(
                after_resize.live_retained_allocations(),
                before_resize.live_retained_allocations()
            );
            assert_eq!(
                after_resize.total_admitted_retained_bytes(),
                before_resize.total_admitted_retained_bytes()
            );
            assert_eq!(
                after_resize.total_admitted_retained_allocations(),
                before_resize.total_admitted_retained_allocations()
            );
            assert_eq!(
                after_resize.peak_retained_bytes(),
                before_resize.peak_retained_bytes()
            );
            assert_eq!(
                after_resize.peak_retained_allocations(),
                before_resize.peak_retained_allocations()
            );
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::Control(_))
            ));
            drop(scratch);
            planning.reject(PreflightRejection::Unsupported, 61)
        })
        .unwrap();

    assert_control_never_activated(outcome, &account);
}

#[test]
fn cancellation_before_direct_reject_precedes_the_requested_cause() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|planning| {
            cancellation.cancel();
            planning.reject(PreflightRejection::Unsupported, 53)
        })
        .unwrap();

    assert_control_never_activated(outcome, &account);
}

#[test]
fn empty_semantic_work_cannot_issue_a_completion() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let baseline_allocations = account.snapshot().live_retained_allocations();

    let result: Result<ScopedOutcome<PreflightRejection>, _> =
        owner.plan_and_activate_scoped(|mut planning| {
            let semantic = planning.begin_semantic_work().unwrap();
            assert!(matches!(
                semantic.finish_exact(),
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            ));
            planning.reject(PreflightRejection::Unsupported, 55)
        });

    assert!(result.is_err());
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        baseline_allocations
    );
    assert_fail_closed_containment(&account);
}

#[test]
fn cancellation_after_finish_exact_prevents_success_from_sealing() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            cancellation.cancel();
            match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
                CanonicalActivationSeal::Sealed(_) => {
                    panic!("final planning control must prevent a cancelled plan from sealing")
                }
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        })
        .unwrap();

    assert_control_never_activated(outcome, &account);
}

#[test]
fn cancelled_seal_releases_scoped_plan_before_never_activated() {
    let factory = factory(1);
    let account = account(&factory);
    let cancellation = CancellationToken::new();
    let control = OperationControl::default().with_cancellation(cancellation.clone());
    let (operation, keys) = operation_with_control(&account, control);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let scratch = semantic.reserve(1).unwrap();
            semantic.validate_live_owner(&scratch).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let plan = TrackedScratchPlan {
                _marker: DropMarker {
                    dropped: &dropped,
                    account: &account,
                    poison_seen_while_dropping: &poison_seen_while_dropping,
                },
                _scratch: scratch,
            };
            cancellation.cancel();
            match planning.seal_activation(bind_test_execution(&mut success, plan), completion) {
                CanonicalActivationSeal::Sealed(_) => {
                    panic!("final cancellation must prevent activation from sealing")
                }
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        })
        .unwrap();

    assert!(dropped.get());
    assert!(!poison_seen_while_dropping.get());
    assert_control_never_activated(outcome, &account);
}

#[test]
fn semantic_action_panics_are_contained_after_the_exact_completed_prefix() {
    for completed_prefix in [0_u64, 1] {
        let factory = factory(1);
        let account = account(&factory);
        let (operation, keys) = operation(&account);
        let TestScopedActivation {
            policy,
            failure,
            success: _,
        } = scoped_activation_shape(keys, 1, 1);
        let owner = operation
            .into_test_session_outcome_owner(policy, failure)
            .unwrap();
        let completed = Cell::new(0_u64);

        let result: Result<ScopedOutcome<PreflightRejection>, _> =
            owner.plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work().unwrap();
                if completed_prefix == 1 {
                    semantic
                        .run_unit(|| completed.set(completed.get() + 1))
                        .unwrap();
                }
                let _ = semantic.run_unit(|| panic!("contained semantic action panic"));
                unreachable!("the panicking action cannot return a disposition")
            });

        assert!(result.is_err());
        assert_eq!(completed.get(), completed_prefix);
        assert_fail_closed_containment(&account);
    }
}

#[test]
fn semantic_action_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let capture_dropped = Cell::new(false);
    let capture = PanicDropProbe(&capture_dropped);
    let mut action = Some(move || -> () {
        let _capture = &capture;
        panic!("contained semantic action panic")
    });

    let result: Result<ScopedOutcome<PreflightRejection>, _> =
        owner.plan_and_activate_scoped(move |mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let action = action
                .take()
                .expect("the semantic action is transferred exactly once");
            let _ = semantic.run_unit(action);
            unreachable!("the panicking semantic action cannot return a disposition")
        });

    assert!(result.is_err());
    assert!(capture_dropped.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn scoped_materialization_callback_capture_is_dropped_before_the_outcome_escapes() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let dropped = Cell::new(false);
    let capture = DropProbe(&dropped);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        move |mut lane| {
            let _capture = &capture;
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, root)
        },
    )
    .unwrap();

    assert!(dropped.get());
    drop(outcome);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_materialization_callback_drop_panic_is_contained() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        move |mut lane| {
            let _capture = &capture;
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)?;
                    Ok(())
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, root)
        },
    );

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn scoped_transaction_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let capture_dropped = Cell::new(false);
    let capture = PanicDropProbe(&capture_dropped);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(move |_planning| {
        let _capture = &capture;
        panic!("contained scoped transaction panic")
    });

    assert!(result.is_err());
    assert!(capture_dropped.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn trusted_producer_seal_panic_drops_the_description_before_containment() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let description_dropped = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let _ = planning.seal_activation(
            PanicSealDescription {
                dropped: &description_dropped,
            },
            completion,
        );
        unreachable!("a panicking producer description cannot return an activation seal")
    });

    assert!(result.is_err());
    assert!(description_dropped.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn scoped_materialization_body_and_capture_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let capture = PanicOnDrop;

    let result = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        move |_lane| -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
            let _capture = &capture;
            panic!("contained scoped materialization callback panic")
        },
    );

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn scoped_materialization_body_and_plan_drop_panics_are_isolated() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);
    let plan_dropped = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let plan = PanicDropProbe(&plan_dropped);
        let permit =
            match planning.seal_activation(bind_test_execution(&mut success, plan), completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
        permit.activate(
            |_lane,
             _plan|
             -> CanonicalMaterialization<'_, (), (), ScalarSequence<u8>, SequenceTag> {
                panic!("contained scoped materialization panic")
            },
        )
    });

    assert!(result.is_err());
    assert!(plan_dropped.get());
    assert_fail_closed_containment(&account);
}

#[test]
fn scoped_materialization_rejection_eagerly_cleans_canonical_storage() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let _ = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(9)?;
                    Ok(())
                })
                .unwrap();
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                41,
            )
        },
    )
    .unwrap();
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(state) => state,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("post-activation semantic rejection must remain Rejected")
        }
    };
    assert_eq!(
        rejected.cause(),
        &StoreCanonicalRejection::Requested(MaterializationRejection::Semantic)
    );
    assert_eq!(rejected.canonical_len(), 0);

    let evidence = rejected
        .build_failure_evidence(|lane| {
            let root = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    EvidenceLeafTag,
                    |writer| writer.push(1),
                )
                .unwrap();
            lane.complete(root)
        })
        .unwrap();
    let rooted = match evidence {
        FailureEvidenceOutcome::Rooted(rooted) => rooted,
        FailureEvidenceOutcome::MinimalFallback(_) => {
            panic!("valid failure evidence must remain rooted")
        }
    };
    let (prepared, cause) = rooted.prepare_minimal_fallback().unwrap().into_parts();
    assert_eq!(
        cause,
        StoreCanonicalRejection::Requested(MaterializationRejection::Semantic)
    );
    let published = prepared.handoff();
    assert_eq!(
        published
            .ledger_evidence
            .dimension(keys.dimension)
            .unwrap()
            .committed_canonical_output(),
        0
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_seal_is_storage_pure_until_owner_consuming_activation() {
    const CANONICAL_BLOCKS: usize = 4;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_terminal_attribution_slots(&account, 2);
    let activation = TestScopedActivation {
        policy: TestActivationPolicy::new(
            RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
            TestEvidenceWriter::new(keys, 2, 2),
            CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
            CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
            keys.dimension,
            keys.dimension,
        ),
        failure: CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(CANONICAL_BLOCKS, CANONICAL_BLOCKS).unwrap(),
            2,
            CANONICAL_BLOCKS,
            NonZeroU64::MIN,
        )
        .unwrap(),
    };
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = activation;
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let before_seal = account.snapshot();
            let permit = match planning.seal_activation(
                bind_test_execution(&mut success, ()),
                completion,
            ) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
            };
            assert_eq!(account.snapshot(), before_seal);
            permit.activate(|lane, _plan| {
                assert!(
                    account.snapshot().total_admitted_retained_bytes()
                        > before_seal.total_admitted_retained_bytes(),
                    "canonical attribution storage must be rebound only after activation consumes the dormant owner"
                );
                lane.reject::<(), (), ScalarSequence<u8>, SequenceTag>((), 73)
            })
        })
        .unwrap();

    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("materialization rejection must remain a rejected activation")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_success_rebinds_minimal_startup_writer_to_exact_canonical_shape() {
    const CANONICAL_BLOCKS: usize = 4;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_terminal_attribution_slots(&account, 2);
    let activation = TestScopedActivation {
        policy: TestActivationPolicy::new(
            RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
            TestEvidenceWriter::new(keys, 2, 2),
            CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
            CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
            keys.dimension,
            keys.dimension,
        ),
        failure: CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(CANONICAL_BLOCKS, CANONICAL_BLOCKS).unwrap(),
            2,
            CANONICAL_BLOCKS,
            NonZeroU64::MIN,
        )
        .unwrap(),
    };

    let outcome = plan_scoped_activation!(operation, activation, |mut lane| {
        let first = lane
            .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                writer.push(1)
            })
            .unwrap();
        let second = lane
            .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                writer.push(2)
            })
            .unwrap();
        let third = lane
            .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                writer.push(3)
            })
            .unwrap();
        let root = lane
            .try_retain_records(RecordPlan::new(1, 3).unwrap(), CanonicalRootTag, |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(first)?;
                    row.write_sequence_ref(second)?;
                    row.write_sequence_ref(third)
                })
            })
            .unwrap();
        lane.complete::<_, (), _, _>(root, root)
    })
    .unwrap();
    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the exact canonical writer rebind must activate")
        }
    };
    assert_eq!(activated.canonical_len(), CANONICAL_BLOCKS);

    let outcome = activated
        .validate_centrally(
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _output| {
                let evidence = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(1),
                    )
                    .unwrap();
                lane.complete(evidence)
            },
            |planning, _output| {
                planning.validate_with(
                    |_| Ok(()),
                    |_session, _canonical, _evidence, &mut ()| {
                        Ok(CanonicalValidationDecision::<(), _>::reject(
                            ValidationRejection::Semantic,
                            74,
                        ))
                    },
                )
            },
        )
        .unwrap();
    let rejected = match outcome {
        CanonicalCentralValidationOutcome::Rejected(rejected) => rejected,
        CanonicalCentralValidationOutcome::Candidate(_)
        | CanonicalCentralValidationOutcome::Fallback(_)
        | CanonicalCentralValidationOutcome::Interrupted(_) => {
            panic!("the test rejection must retain the exact canonical assembly")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
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
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_materialization_first_fault_is_sticky_through_rejection() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let _ = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(9)?;
                    Ok(())
                })
                .unwrap();
            let cause = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(10)?;
                    Ok(())
                })
                .unwrap_err();
            assert!(matches!(
                cause,
                StoreRuntimeError::SlotLimit {
                    destination: StoreDestination::Canonical
                }
            ));
            assert!(matches!(
                lane.control_checkpoint(),
                Err(StoreRuntimeError::SlotLimit {
                    destination: StoreDestination::Canonical
                })
            ));
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                42,
            )
        },
    )
    .unwrap();

    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("a handled materialization fault must remain an ordinary rejection")
        }
    };
    assert!(matches!(
        rejected.cause(),
        StoreCanonicalRejection::Runtime(StoreRuntimeError::SlotLimit {
            destination: StoreDestination::Canonical
        })
    ));
    assert_eq!(rejected.canonical_len(), 0);

    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::ProductGraph
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_materialization_build_fault_preserves_its_exact_product_graph_code() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = plan_scoped_activation!(
        operation,
        scoped_activation_shape(keys, 1, 1),
        |mut lane| {
            let expected = StoreBuildError::domain(73).unwrap();
            let cause = lane
                .try_retain_sequence(
                    SequencePlan::<u8>::new(1).unwrap(),
                    SequenceTag,
                    |_writer| Err(expected.into()),
                )
                .unwrap_err();
            assert_eq!(cause, StoreRuntimeError::Build(expected));
            lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                MaterializationRejection::Semantic,
                43,
            )
        },
    )
    .unwrap();
    let rejected = match outcome {
        CanonicalActivationOutcome::Rejected(rejected) => rejected,
        CanonicalActivationOutcome::Activated(_)
        | CanonicalActivationOutcome::NeverActivated(_)
        | CanonicalActivationOutcome::Fallback(_) => {
            panic!("the handled build fault must remain an ordinary rejection")
        }
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    let terminal = store_failure_cause(&published.ledger_evidence);
    assert_eq!(terminal.class(), StorePreparationFailureClass::ProductGraph);
    assert_eq!(terminal.code(), 73);
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn scoped_invalid_failure_shape_is_contained_before_materialization() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success,
    } = scoped_activation_shape(keys, 1, TEST_MAX_FAILURE_EVIDENCE_MEMBERS + 1);
    let owner = operation
        .into_test_session_outcome_owner(policy, failure)
        .unwrap();
    let mut success = Some(success);
    let materialized = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        match planning.seal_activation(bind_test_execution(&mut success, ()), completion) {
            CanonicalActivationSeal::Sealed(permit) => permit.activate(
                |_lane, _plan| -> CanonicalMaterialization<
                    '_,
                    (),
                    (),
                    ScalarSequence<u8>,
                    SequenceTag,
                > {
                    materialized.set(true);
                    unreachable!("an invalid failure shape cannot expose canonical storage")
                },
            ),
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    });

    assert!(result.is_err());
    assert!(!materialized.get());
    assert_fail_closed_containment(&account);
}
