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
fn scoped_private_migration_plan_moves_its_payload_directly_into_materialization() {
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

    // This proves only a linear move through the temporary generic seam. It does not prove that
    // arbitrary `P` storage was allocated after reservation; the real core sealed plan must
    // structurally own every metered collection before the U4e cutover.
    let outcome = owner
        .plan_and_activate_scoped(|mut planning| {
            let mut semantic = planning.begin_semantic_work(1).unwrap();
            let mut scratch = semantic.reserve(4).unwrap();
            semantic.resize(&mut scratch, 8).unwrap();
            semantic.validate_live_owner(&scratch).unwrap();
            semantic.run_unit(|| ()).unwrap();
            semantic.finish_exact().unwrap();

            let sealed = match planning.seal_plan(
                success,
                ScratchPlan {
                    values: vec![3, 5, 8],
                    scratch,
                },
            ) {
                CanonicalPlanSeal::Sealed(sealed) => sealed,
                CanonicalPlanSeal::Terminal(disposition) => return disposition,
            };
            sealed.activate(|lane, plan| {
                assert_eq!(plan.values, [3, 5, 8]);
                let ScratchPlan { values: _, scratch } = plan;
                drop(scratch);
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
fn scoped_sealed_plan_is_single_use_and_not_clone_or_copy() {
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

    // Keep the payload itself Copy + Clone so this test cannot pass only because a conditional
    // implementation inherits non-Copy/non-Clone behavior from the payload.
    type Plan = ();
    type Sealed = CanonicalSealedPlan<
        'static,
        Plan,
        TestOwner,
        TestProgress,
        TestUnit,
        (),
        (),
        ScalarSequence<u8>,
        SequenceTag,
    >;
    type Receipt = CanonicalScopedDisposition<'static>;

    let _ = <Sealed as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Sealed as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <Receipt as AmbiguousIfCopy<_>>::assert_not_copy;
    let _ = <Receipt as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn dropping_a_scoped_sealed_plan_releases_scratch_before_containment() {
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
        let mut semantic = planning.begin_semantic_work(1).unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        semantic.finish_exact().unwrap();
        let sealed = match planning.seal_plan(
            success,
            TrackedScratchPlan {
                _marker: DropMarker {
                    dropped: &dropped,
                    account: &account,
                    poison_seen_while_dropping: &poison_seen_while_dropping,
                },
                _scratch: scratch,
            },
        ) {
            CanonicalPlanSeal::Sealed(sealed) => sealed,
            CanonicalPlanSeal::Terminal(disposition) => return disposition,
        };
        drop(sealed);
        panic!("dropping a sealed plan cannot synthesize a publishable outcome")
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
        let mut semantic = planning.begin_semantic_work(1).unwrap();
        let scratch = semantic.reserve(1).unwrap();
        semantic.run_unit(|| ()).unwrap();
        semantic.finish_exact().unwrap();
        match planning.seal_plan(
                success,
                TrackedScratchPlan {
                    _marker: DropMarker {
                        dropped: &dropped,
                        account: &account,
                        poison_seen_while_dropping: &poison_seen_while_dropping,
                    },
                    _scratch: scratch,
                },
            ) {
            CanonicalPlanSeal::Sealed(sealed) => sealed.activate(
                |_lane, _plan| -> CanonicalMaterialization<
                    '_,
                    (),
                    (),
                    ScalarSequence<u8>,
                    SequenceTag,
                > { panic!("contained scoped materialization panic") },
            ),
            CanonicalPlanSeal::Terminal(disposition) => disposition,
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
            planning
                .begin_semantic_work(0)
                .unwrap()
                .finish_exact()
                .unwrap();
            let sealed = match planning.seal_plan(success, ()) {
                CanonicalPlanSeal::Sealed(sealed) => sealed,
                CanonicalPlanSeal::Terminal(disposition) => return disposition,
            };
            let _receipt = sealed.activate(|lane, ()| {
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
            planning
                .begin_semantic_work(0)
                .unwrap()
                .finish_exact()
                .unwrap();
            let sealed = match planning.seal_plan(success, ()) {
                CanonicalPlanSeal::Sealed(sealed) => sealed,
                CanonicalPlanSeal::Terminal(disposition) => return disposition,
            };
            let _receipt = sealed.activate(|mut lane, ()| {
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
            let cause = match planning.begin_semantic_work(1) {
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
            let mut semantic = planning.begin_semantic_work(1).unwrap();
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
fn runtime_fault_during_seal_returns_terminal_receipt_without_panicking() {
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
    cancellation.cancel();

    let outcome: ScopedOutcome<()> = owner
        .plan_and_activate_scoped(|mut planning| {
            assert!(matches!(
                planning.begin_semantic_work(1),
                Err(StoreRuntimeError::Control(_))
            ));
            match planning.seal_plan(success, ()) {
                CanonicalPlanSeal::Sealed(_) => {
                    panic!("an interrupted semantic envelope cannot seal")
                }
                CanonicalPlanSeal::Terminal(disposition) => disposition,
            }
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
            let semantic = planning.begin_semantic_work(1).unwrap();
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
            let mut semantic = planning.begin_semantic_work(1).unwrap();
            let scratch = semantic.reserve(1).unwrap();
            semantic.run_unit(|| ()).unwrap();
            semantic.finish_exact().unwrap();
            std::mem::forget(scratch);
            let sealed = match planning.seal_plan(success, ()) {
                CanonicalPlanSeal::Sealed(sealed) => sealed,
                CanonicalPlanSeal::Terminal(disposition) => return disposition,
            };
            sealed.activate(|mut lane, ()| match terminal {
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
            let mut semantic = planning.begin_semantic_work(attempted_units).unwrap();
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
                semantic.finish_exact().unwrap();
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
            let mut semantic = planning.begin_semantic_work(1).unwrap();
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
    assert_eq!(
        store_failure_cause(&published.ledger_evidence).class(),
        StorePreparationFailureClass::Control
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
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
fn cancellation_before_zero_work_rejection_is_observed_at_final_planning_poll() {
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
            let semantic = planning.begin_semantic_work(0).unwrap();
            cancellation.cancel();
            semantic.finish_rejected().unwrap();
            planning.reject(PreflightRejection::Unsupported, 54)
        })
        .unwrap();

    assert_control_never_activated(outcome, &account);
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
            let mut semantic = planning.begin_semantic_work(1).unwrap();
            semantic.run_unit(|| ()).unwrap();
            semantic.finish_exact().unwrap();
            cancellation.cancel();
            match planning.seal_plan(success, ()) {
                CanonicalPlanSeal::Sealed(_) => {
                    panic!("final planning control must prevent a cancelled plan from sealing")
                }
                CanonicalPlanSeal::Terminal(disposition) => disposition,
            }
        })
        .unwrap();

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
                let mut semantic = planning.begin_semantic_work(2).unwrap();
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
