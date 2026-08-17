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
    baozi_operation::CanonicalInheritanceSource,
>;

fn derived_operation_for_scoped_activation(
    source: &SourceCanonical,
) -> (
    baozi_operation::DerivedRetainedStoreOperation<TestOwner, TestProgress, TestUnit>,
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

fn operation_with_scoped_semantic_limit(
    account: &PhysicalStorageAccount,
    semantic_work_limit: u64,
) -> (TestOperation, TestKeys) {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let failure_terminal_work = unbounded_failure_terminal_work(keys);
    let primary_attempted_limit = failure_terminal_work
        .ordinary_attempted_work_limit()
        .checked_sub(semantic_work_limit)
        .expect("the scoped semantic dimension fits the ordinary attempted budget");
    let primary_consumed_limit = failure_terminal_work
        .ordinary_consumed_work_limit()
        .checked_sub(semantic_work_limit)
        .expect("the scoped semantic dimension fits the ordinary consumed budget");
    let primary_policy = DimensionPolicy::reservable_bytes(
        primary_attempted_limit,
        primary_consumed_limit,
        u64::MAX,
        u64::MAX,
    )
    .with_published_outcome_evidence_limit(u64::MAX);
    let semantic_policy = DimensionPolicy::reservable_bytes(
        semantic_work_limit,
        semantic_work_limit,
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: DerivedScopedOutcome<MaterializationRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate((), |mut lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate((), |mut lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate((), |mut lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<DerivedScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate((), |mut lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

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

            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    drop(scratch);
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate(
                ScratchPlan {
                    values: vec![3, 5, 8],
                    _scratch: scratch,
                },
                |lane, plan| {
                    assert_eq!(plan.values, [3, 5, 8]);
                    lane.reject::<(), _, ScalarSequence<u8>, SequenceTag>(
                        MaterializationRejection::Semantic,
                        21,
                    )
                },
            )
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let _plan = TrackedScratchPlan {
            _marker: DropMarker {
                dropped: &dropped,
                account: &account,
                poison_seen_while_dropping: &poison_seen_while_dropping,
            },
            _scratch: scratch,
        };
        let permit = match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => {
                drop(_plan);
                return failure.finish_after_plan_drop();
            }
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
        let policy = CanonicalActivationPolicy::new(
            writer_plan(
                if matches!(fault, ScopedRoleFault::CanonicalWriter) {
                    foreign_writer_keys
                } else {
                    keys
                },
                2,
                2,
            ),
            writer_plan(
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

        let result = operation.into_scoped_session_outcome_owner(policy, failure);

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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let dropped = Cell::new(false);
    let poison_seen_while_dropping = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit.activate(
                TrackedScratchPlan {
                    _marker: DropMarker {
                        dropped: &dropped,
                        account: &account,
                        poison_seen_while_dropping: &poison_seen_while_dropping,
                    },
                    _scratch: scratch,
                },
                |_lane, _plan| -> CanonicalMaterialization<
                    '_,
                    (),
                    (),
                    ScalarSequence<u8>,
                    SequenceTag,
                > { panic!("contained scoped materialization panic") },
            ),
            CanonicalActivationSeal::Unsealed(failure) => {
                drop(scratch);
                failure.finish_after_plan_drop()
            }
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            let _receipt = permit.activate((), |lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            let _receipt = permit.activate((), |mut lane, _plan| {
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<MaterializationRejection>, _> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            let scratch = semantic.reserve(1).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            std::mem::forget(scratch);
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            permit.activate((), |mut lane, _plan| match terminal {
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        let (operation, keys) = operation_with_scoped_semantic_limit(&account, semantic_work_limit);
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
            .into_scoped_session_outcome_owner(policy, failure)
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
    let (operation, keys) = operation_with_scoped_semantic_limit(&account, 1);
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        let (operation, keys) = operation_with_scoped_semantic_limit(&account, 2);
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
            .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work().unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            cancellation.cancel();
            match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(_) => {
                    panic!("final planning control must prevent a cancelled plan from sealing")
                }
                CanonicalActivationSeal::Unsealed(failure) => failure.finish_after_plan_drop(),
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
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
            match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(_) => {
                    panic!("final cancellation must prevent activation from sealing")
                }
                CanonicalActivationSeal::Unsealed(failure) => {
                    drop(plan);
                    failure.finish_after_plan_drop()
                }
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
            .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let plan_dropped = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        let permit = match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => {
                return failure.finish_after_plan_drop();
            }
        };
        permit.activate(
            PanicDropProbe(&plan_dropped),
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
fn scoped_success_rebinds_minimal_startup_writer_to_exact_canonical_shape() {
    const CANONICAL_BLOCKS: usize = 4;

    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation_with_terminal_attribution_slots(&account, 2);
    let activation = TestScopedActivation {
        policy: CanonicalActivationPolicy::new(
            writer_plan(keys, CANONICAL_BLOCKS + 1, 2),
            writer_plan(keys, 2, 2),
            CanonicalMaterializationRole::new(keys.dimension, keys.dimension),
            CanonicalValidationRole::new(keys.phase, keys.dimension, keys.dimension),
            keys.dimension,
            keys.dimension,
        ),
        failure: CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
        success: CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(CANONICAL_BLOCKS, CANONICAL_BLOCKS).unwrap(),
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
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();
    let materialized = Cell::new(false);

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work().unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit.activate(
                (),
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
            CanonicalActivationSeal::Unsealed(failure) => failure.finish_after_plan_drop(),
        }
    });

    assert!(result.is_err());
    assert!(!materialized.get());
    assert_fail_closed_containment(&account);
}
