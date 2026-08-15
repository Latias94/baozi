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

fn semantic_units(units: u64) -> NonZeroU64 {
    NonZeroU64::new(units).expect("semantic work fixtures are nonzero")
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
    scratch: CanonicalReservation<'scope, TestOwner>,
}

struct DropMarker<'a> {
    dropped: &'a Cell<bool>,
    account: &'a PhysicalStorageAccount,
    poison_seen_while_dropping: &'a Cell<bool>,
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
fn scoped_activation_permit_moves_the_core_plan_only_at_activation() {
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

    // Operation verifies the semantic completion and physical shape without interpreting the
    // core-owned execution plan. The plan moves behind the owner-consuming transition only here.
    let outcome = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
                    scratch,
                },
                |lane, plan| {
                    assert_eq!(plan.values, [3, 5, 8]);
                    let ScratchPlan { values: _, scratch } = plan;
                    drop(scratch);
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
        let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
    ValidationWork,
    CanonicalDestination,
    EvidenceDestination,
}

#[test]
fn scoped_constructor_validates_every_role_before_authoring_authority_exists() {
    for fault in [
        ScopedRoleFault::CanonicalWriter,
        ScopedRoleFault::EvidenceWriter,
        ScopedRoleFault::MaterializationWork,
        ScopedRoleFault::ValidationWork,
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
            if matches!(fault, ScopedRoleFault::MaterializationWork) {
                foreign_dimension()
            } else {
                keys.dimension
            },
            if matches!(fault, ScopedRoleFault::ValidationWork) {
                foreign_dimension()
            } else {
                keys.dimension
            },
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
        let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            let _receipt = permit.activate((), |lane, ()| {
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
            semantic.run_unit(|| ()).unwrap();
            let completion = semantic.finish_exact().unwrap();
            let permit = match planning.seal_activation(success, completion) {
                CanonicalActivationSeal::Sealed(permit) => permit,
                CanonicalActivationSeal::Unsealed(failure) => {
                    return failure.finish_after_plan_drop();
                }
            };
            let _receipt = permit.activate((), |mut lane, ()| {
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

    let outcome: ScopedOutcome<PreflightRejection> = owner
        .plan_and_activate_scoped(|mut planning| {
            let cause = match planning.begin_semantic_work(NonZeroU64::MIN) {
                Ok(_) => panic!("pre-cancelled semantic admission must retain Control"),
                Err(cause) => cause,
            };
            assert!(matches!(cause, StoreRuntimeError::Control(_)));
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            semantic.finish_rejected().unwrap();
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
            assert!(matches!(
                planning.begin_semantic_work(NonZeroU64::MIN),
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
            let semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            permit.activate((), |mut lane, ()| match terminal {
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

fn scoped_semantic_case(
    attempted_units: u64,
    execute_limit: u64,
    cancel_after: Option<u64>,
) -> (u64, u64, u64, bool) {
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
            let mut semantic = planning
                .begin_semantic_work(semantic_units(attempted_units))
                .unwrap();
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
            if executed.get() == attempted_units && !interrupted.get() {
                let _completion = semantic.finish_exact().unwrap();
            } else {
                let finish = semantic.finish_rejected();
                if interrupted.get() {
                    assert!(matches!(finish, Err(StoreRuntimeError::Control(_))));
                } else {
                    finish.unwrap();
                }
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
    assert_eq!(scoped_semantic_case(3, 1, None), (3, 1, 1, false));
}

#[test]
fn semantic_work_admission_is_atomic_at_exact_one_short_and_zero_capacity() {
    const REQUESTED_UNITS: u64 = 2;

    for (semantic_work_limit, expected_admitted, expected_attempted, expected_cause) in [
        (
            REQUESTED_UNITS,
            true,
            REQUESTED_UNITS,
            StorePreparationFailureClass::Requested,
        ),
        (
            1,
            false,
            REQUESTED_UNITS,
            StorePreparationFailureClass::Ledger,
        ),
        (
            0,
            false,
            REQUESTED_UNITS,
            StorePreparationFailureClass::Ledger,
        ),
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
        let admitted = Cell::new(false);

        let outcome: ScopedOutcome<PreflightRejection> = owner
            .plan_and_activate_scoped(|mut planning| {
                match planning.begin_semantic_work(semantic_units(REQUESTED_UNITS)) {
                    Ok(semantic) => {
                        admitted.set(true);
                        semantic.finish_rejected().unwrap();
                    }
                    Err(cause) => {
                        assert!(matches!(cause, StoreRuntimeError::Ledger(_)));
                    }
                }
                planning.reject(PreflightRejection::Unsupported, 56)
            })
            .unwrap();

        assert_eq!(admitted.get(), expected_admitted);
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
        assert_eq!(semantic_work.consumed(), 0);
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
fn caught_semantic_action_panic_preserves_the_exact_completed_prefix() {
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

        let outcome: ScopedOutcome<PreflightRejection> = owner
            .plan_and_activate_scoped(|mut planning| {
                let mut semantic = planning.begin_semantic_work(semantic_units(2)).unwrap();
                if completed_prefix == 1 {
                    semantic.run_unit(|| ()).unwrap();
                }
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = semantic.run_unit(|| -> () { panic!("caught semantic action panic") });
                }));
                assert!(panicked.is_err());
                semantic.finish_rejected().unwrap();
                planning.reject(PreflightRejection::Unsupported, 58)
            })
            .unwrap();

        let never_activated = match outcome {
            CanonicalActivationOutcome::NeverActivated(state) => state,
            CanonicalActivationOutcome::Activated(_)
            | CanonicalActivationOutcome::Rejected(_)
            | CanonicalActivationOutcome::Fallback(_) => {
                panic!("a caught semantic panic must remain pre-activation")
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
            .expect("the panicking semantic action remains queryable");
        assert_eq!(semantic_work.attempted(), 2);
        assert_eq!(semantic_work.consumed(), completed_prefix);
        assert_eq!(
            store_failure_cause(&published.ledger_evidence).class(),
            StorePreparationFailureClass::Requested
        );
        drop(published);
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn semantic_work_polling_is_bounded_at_255_256_and_257_units() {
    assert_eq!(scoped_semantic_case(255, 255, None), (255, 255, 255, false));
    assert_eq!(scoped_semantic_case(256, 256, None), (256, 256, 256, false));
    assert_eq!(
        scoped_semantic_case(257, 257, Some(256)),
        (257, 256, 256, true)
    );
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            let mut semantic = planning.begin_semantic_work(semantic_units(256)).unwrap();
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
fn semantic_work_rejects_the_first_unit_beyond_the_admitted_envelope() {
    const ADMITTED_UNITS: u64 = 2;

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
    let extra_action_executed = Cell::new(false);

    let result: Result<ScopedOutcome<PreflightRejection>, _> =
        owner.plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning
                .begin_semantic_work(semantic_units(ADMITTED_UNITS))
                .unwrap();
            for _ in 0..ADMITTED_UNITS {
                semantic
                    .run_unit(|| completed.set(completed.get() + 1))
                    .unwrap();
            }

            assert!(matches!(
                semantic.run_unit(|| extra_action_executed.set(true)),
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            ));
            assert_eq!(completed.get(), ADMITTED_UNITS);
            assert!(!extra_action_executed.get());
            assert!(matches!(
                semantic.finish_rejected(),
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            ));
            planning.reject(PreflightRejection::Unsupported, 59)
        });

    assert!(result.is_err());
    assert_eq!(completed.get(), ADMITTED_UNITS);
    assert!(!extra_action_executed.get());
    assert_fail_closed_containment(&account);
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
            let mut semantic = planning.begin_semantic_work(semantic_units(256)).unwrap();
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
            let mut semantic = planning.begin_semantic_work(semantic_units(256)).unwrap();
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
fn incomplete_semantic_work_cannot_issue_a_completion() {
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
            let mut semantic = planning.begin_semantic_work(semantic_units(2)).unwrap();
            semantic.run_unit(|| ()).unwrap();
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

fn assert_semantic_completion_mismatch(completed_units: u64, required_units: u64) {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);
    let TestScopedActivation {
        policy,
        failure,
        success: _,
    } = scoped_activation_shape(keys, 1, 1);
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        1,
        NonZeroU64::new(required_units).unwrap(),
        NonZeroU64::MIN,
    )
    .unwrap();
    let owner = operation
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning
            .begin_semantic_work(semantic_units(completed_units))
            .unwrap();
        for _ in 0..completed_units {
            semantic.run_unit(|| ()).unwrap();
        }
        let completion = semantic.finish_exact().unwrap();
        match planning.seal_activation(success, completion) {
            CanonicalActivationSeal::Sealed(_) => {
                panic!("a mismatched semantic completion cannot issue an activation permit")
            }
            CanonicalActivationSeal::Unsealed(failure) => failure.finish_after_plan_drop(),
        }
    });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
}

#[test]
fn semantic_completion_must_match_the_sealed_success_envelope_exactly() {
    assert_semantic_completion_mismatch(1, 2);
    assert_semantic_completion_mismatch(2, 1);
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
            let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
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
fn cancellation_cannot_mask_a_semantic_completion_mismatch() {
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
    let mismatched_success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
        1,
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::MIN,
    )
    .unwrap();
    let owner = operation
        .into_scoped_session_outcome_owner(policy, failure)
        .unwrap();

    let result: Result<ScopedOutcome<()>, _> = owner.plan_and_activate_scoped(|mut planning| {
        let mut semantic = planning.begin_semantic_work(NonZeroU64::MIN).unwrap();
        semantic.run_unit(|| ()).unwrap();
        let completion = semantic.finish_exact().unwrap();
        cancellation.cancel();
        match planning.seal_activation(mismatched_success, completion) {
            CanonicalActivationSeal::Sealed(_) => {
                panic!("a mismatched completion cannot issue an activation permit")
            }
            CanonicalActivationSeal::Unsealed(failure) => failure.finish_after_plan_drop(),
        }
    });

    assert!(result.is_err());
    assert_fail_closed_containment(&account);
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
                let mut semantic = planning.begin_semantic_work(semantic_units(2)).unwrap();
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
