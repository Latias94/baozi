use super::*;

#[test]
fn finite_foreign_and_occupied_interruption_slots_are_denied_before_freeze_control() {
    for occupied in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let foreign_host = Host::new();
        let foreign_identity = foreign_host.store.as_ref().unwrap().invocation_identity();
        let mut host = Host::new();
        let (error, dimensions, before_issue) = {
            let (completion, dimensions) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (
                    Payload::new("output", 41, &drops),
                    Payload::new("proof", 43, &drops),
                ),
                &calls,
                0,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("slot admission requires the original validated finite candidate");
            };
            let slot = completion.generation.interruption;
            assert_ne!(slot.identity, foreign_identity);
            assert!(slot.value.is_none());
            if occupied {
                *slot.value = Some(CanonicalFreezeInterruption::from_failure_code(
                    crate::retained::PreparationFailureCode::domain(127),
                    None,
                ));
            }
            let slot = CanonicalCauseSlot {
                value: slot.value,
                order: slot.order,
                identity: if occupied {
                    slot.identity
                } else {
                    foreign_identity
                },
            };
            let before_issue = probe.counts();
            let admitted = account.snapshot();
            assert!(admitted.live_retained_bytes() > 0);
            assert!(admitted.live_retained_allocations() > 0);
            assert!(!admitted.poisoned());
            let error = candidate
                .try_issue_freeze_permit(slot, progress())
                .err()
                .expect("an invalid original recovery slot cannot issue a finite permit");
            assert_eq!(probe.counts(), before_issue);
            assert_eq!(calls.counts(), [1, 1, 1, 0]);
            assert_eq!(*drops.borrow(), ["proof", "output"]);
            assert_eq!(
                account.snapshot().total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            (error, dimensions, before_issue)
        };
        assert!(host.output.is_none() && host.proof.is_none());
        if occupied {
            let cause = host.freeze_interrupted.as_ref().unwrap();
            assert_eq!(cause.stable_code(), 127);
            assert_eq!(
                cause.class(),
                CanonicalFreezeInterruptionClass::ProductGraph
            );
            assert_eq!(cause.control_cause(), None);
        } else {
            assert!(host.freeze_interrupted.is_none());
        }
        assert!(error.has_residue());
        assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
        assert_eq!(error.original_freeze_control(), None);
        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert!(contained.first_poison_evidence().is_some());
        assert!(contained.live_retained_bytes() > 0);
        assert!(contained.live_retained_allocations() > 0);
        drop(host);
        drop(foreign_host);
        drop(error);
        assert_eq!(account.snapshot(), contained);
        assert_eq!(probe.counts(), [before_issue[0], before_issue[1], 1, 1]);
        assert_eq!(*drops.borrow(), ["proof", "output"]);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn finite_foreign_root_pairing_disposes_generation_captures_before_original_payloads() {
    struct Capture<'account> {
        account: &'account PhysicalStorageAccount,
        drops: Rc<RefCell<Vec<&'static str>>>,
        label: &'static str,
        panic_on_drop: bool,
    }

    impl Drop for Capture<'_> {
        fn drop(&mut self) {
            let retained = self.account.snapshot();
            assert!(retained.live_retained_bytes() > 0);
            assert!(retained.live_retained_allocations() > 0);
            let mut drops = self.drops.borrow_mut();
            assert!(!drops.contains(&"output") && !drops.contains(&"proof"));
            drops.push(self.label);
            if self.panic_on_drop {
                panic!("denied finite generation capture destructor fault");
            }
        }
    }

    for (replace_canonical, panic_on_drop) in [(false, false), (true, false), (true, true)] {
        let factory = test_factory();
        let account = test_account(&factory);
        let foreign_factory = test_factory();
        let foreign_account = test_account(&foreign_factory);
        let probe = Arc::new(ControlProbe::default());
        let foreign_probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let foreign_calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let foreign_drops = Rc::new(RefCell::new(Vec::new()));
        let generated = Cell::new(0);
        let mut host = Host::new();
        let mut foreign_host = Host::new();
        let (error, dimensions, foreign_dimensions, frozen_counts, foreign_counts) = {
            let (completion, dimensions) = candidate_completion(
                &mut host,
                &account,
                probe.control(),
                (
                    Payload::new("output", 41, &drops),
                    Payload::new("proof", 43, &drops),
                ),
                &calls,
                0,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                panic!("pairing admission requires the original finite candidate");
            };
            let canonical = candidate.proof().canonical;
            let evidence = candidate.proof().evidence;
            let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("pairing admission must follow an actual Ready decision");
            };
            let (foreign_completion, foreign_dimensions) = candidate_completion(
                &mut foreign_host,
                &foreign_account,
                foreign_probe.control(),
                (
                    Payload::new("foreign output", 47, &foreign_drops),
                    Payload::new("foreign proof", 53, &foreign_drops),
                ),
                &foreign_calls,
                0,
                false,
            );
            let FiniteCanonicalInvocationOutcome::Candidate(foreign_candidate) =
                foreign_completion.outcome
            else {
                panic!("the foreign roots must come from a second real finite candidate");
            };
            let foreign_canonical = foreign_candidate.proof().canonical;
            let foreign_evidence = foreign_candidate.proof().evidence;
            assert!(canonical != foreign_canonical);
            assert!(evidence != foreign_evidence);
            let FiniteCanonicalFreezePermitIssue::Ready(foreign_permit) = foreign_candidate
                .try_issue_freeze_permit(foreign_completion.generation.interruption, progress())
                .unwrap()
            else {
                panic!("the foreign invocation must retain its own healthy frozen graph");
            };
            let frozen_counts = probe.counts();
            let foreign_counts = foreign_probe.counts();
            let admitted = account.snapshot();
            let foreign_retained = foreign_account.snapshot();
            let captures = (
                Capture {
                    account: &account,
                    drops: Rc::clone(&drops),
                    label: "generation",
                    panic_on_drop,
                },
                Capture {
                    account: &account,
                    drops: Rc::clone(&drops),
                    label: "context",
                    panic_on_drop: false,
                },
            );
            let generated_ref = &generated;
            let error = permit
                .admit_generation_pairing(
                    if replace_canonical {
                        foreign_canonical
                    } else {
                        canonical
                    },
                    if replace_canonical {
                        evidence
                    } else {
                        foreign_evidence
                    },
                    move || {
                        let _captures = &captures;
                        generated_ref.set(generated_ref.get() + 1);
                    },
                )
                .err()
                .expect("either foreign root must deny generation admission");
            assert_eq!(generated.get(), 0);
            assert_eq!(
                *drops.borrow(),
                ["generation", "context", "output", "proof"]
            );
            assert_eq!(probe.counts(), frozen_counts);
            assert_eq!(foreign_probe.counts(), foreign_counts);
            assert_eq!(calls.counts(), [1, 1, 1, 0]);
            assert_eq!(foreign_calls.counts(), [1, 1, 1, 0]);
            assert_eq!(foreign_account.snapshot(), foreign_retained);
            assert!(foreign_drops.borrow().is_empty());
            assert_eq!(
                account.snapshot().total_admitted_retained_allocations(),
                admitted.total_admitted_retained_allocations(),
            );
            drop(foreign_permit);
            (
                error,
                dimensions,
                foreign_dimensions,
                frozen_counts,
                foreign_counts,
            )
        };
        assert!(matches!(
            error.cause(),
            FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
                crate::StorePrepareError::FinalizationContainment(cause),
            )) if cause.reason() == crate::AccountPoisonReason::OwnershipGraphUnverified
        ));
        assert!(error.has_residue());
        assert!(error.domain().unwrap().owns_dimension(dimensions[0]));
        assert!(
            !error
                .domain()
                .unwrap()
                .owns_dimension(foreign_dimensions[0])
        );
        assert_eq!(*foreign_drops.borrow(), ["foreign output", "foreign proof"]);
        drop(foreign_host);
        assert!(!foreign_account.snapshot().poisoned());
        assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
        assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert!(contained.live_retained_bytes() > 0);
        assert!(contained.live_retained_allocations() > 0);
        drop(host);
        drop(error);
        assert_eq!(account.snapshot(), contained);
        assert_eq!(generated.get(), 0);
        assert_eq!(
            *drops.borrow(),
            ["generation", "context", "output", "proof"]
        );
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(foreign_probe.counts(), foreign_counts);
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
        assert_eq!(foreign_factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn forgotten_finite_permit_and_joint_leave_original_graph_recoverable_by_host_cleanup() {
    for bind_joint in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory);
        let probe = Arc::new(ControlProbe::default());
        let calls = WorkCalls::default();
        let drops = Rc::new(RefCell::new(Vec::new()));
        let (retained, frozen_counts) = {
            let mut host = Host::new();
            let (retained, frozen_counts) = {
                let (completion, _) = candidate_completion(
                    &mut host,
                    &account,
                    probe.control(),
                    (
                        Payload::new("output", 41, &drops),
                        Payload::new("proof", 43, &drops),
                    ),
                    &calls,
                    0,
                    false,
                );
                let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome
                else {
                    panic!("forgetting must consume the real restored finite candidate");
                };
                let FiniteCanonicalFreezePermitIssue::Ready(mut permit) = candidate
                    .try_issue_freeze_permit(completion.generation.interruption, progress())
                    .unwrap()
                else {
                    panic!("the forgotten permission must own an actual frozen success loan");
                };
                assert!(permit.destination_counters_are_zero());
                let retained = account.snapshot();
                let frozen_counts = probe.counts();
                assert!(retained.live_retained_bytes() > 0);
                assert!(retained.live_retained_allocations() > 0);
                assert!(!retained.poisoned());
                if bind_joint {
                    permit.bind_joint_commit(|joint, _seal| std::mem::forget(joint));
                } else {
                    std::mem::forget(permit);
                }
                (retained, frozen_counts)
            };
            assert!(host.output.is_some() && host.proof.is_some());
            assert!(drops.borrow().is_empty());
            assert_eq!(account.snapshot(), retained);
            assert_eq!(probe.counts(), frozen_counts);
            (retained, frozen_counts)
        };
        assert_eq!(*drops.borrow(), ["output", "proof"]);
        assert_eq!(probe.counts(), frozen_counts);
        assert_eq!(calls.counts(), [1, 1, 1, 0]);
        let released = account.snapshot();
        assert!(!released.poisoned());
        assert!(released.first_poison_evidence().is_none());
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert_eq!(
            released.total_admitted_retained_allocations(),
            retained.total_admitted_retained_allocations(),
        );
        assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
    }
}

#[test]
fn finite_payload_drop_fault_preserves_outer_unwind_and_contains_original_frozen_graph() {
    struct DropCallback<Callback: FnOnce()>(Option<Callback>);

    impl<Callback: FnOnce()> Drop for DropCallback<Callback> {
        fn drop(&mut self) {
            if let Some(callback) = self.0.take() {
                callback();
            }
        }
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let probe = Arc::new(ControlProbe::default());
    let calls = WorkCalls::default();
    let output_drops = Rc::new(RefCell::new(Vec::new()));
    let proof_drops = Rc::new(RefCell::new(Vec::new()));
    let mut host = Host::new();
    let (frozen_counts, retained) = {
        let (completion, _) = candidate_completion(
            &mut host,
            &account,
            probe.control(),
            (
                Payload::new("output", 41, &output_drops),
                Payload::new("proof", 43, &proof_drops),
            ),
            &calls,
            0,
            false,
        );
        let FiniteCanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("the unwind witness requires the actual finite candidate");
        };
        let FiniteCanonicalFreezePermitIssue::Ready(permit) = candidate
            .try_issue_freeze_permit(completion.generation.interruption, progress())
            .unwrap()
        else {
            panic!("the payload fault must follow a successful finite freeze");
        };
        let frozen_counts = probe.counts();
        let retained = account.snapshot();
        let output_log_loan = output_drops.borrow();
        let observed = Cell::new(false);
        let execution = catch_unwind(AssertUnwindSafe(|| {
            let _during_unwind = DropCallback(Some(|| {
                assert!(std::thread::panicking());
                drop(permit);
                assert_eq!(*proof_drops.borrow(), ["proof"]);
                assert_eq!(probe.counts(), frozen_counts);
                assert!(account.snapshot().poisoned());
                assert!(account.snapshot().live_retained_allocations() > 0);
                observed.set(true);
            }));
            panic!("finite freeze original outer unwind");
        }));
        drop(output_log_loan);
        let payload = execution.expect_err("the original outer unwind must escape payload cleanup");
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"finite freeze original outer unwind"),
        );
        assert!(observed.get());
        (frozen_counts, retained)
    };
    assert!(host.output.is_none() && host.proof.is_none());
    assert!(output_drops.borrow().is_empty());
    assert_eq!(*proof_drops.borrow(), ["proof"]);
    assert_eq!(calls.counts(), [1, 1, 1, 0]);
    let contained = account.snapshot();
    assert!(contained.poisoned());
    assert!(contained.first_poison_evidence().is_some());
    assert!(contained.live_retained_bytes() > 0);
    assert!(contained.live_retained_allocations() > 0);
    assert_eq!(
        contained.total_admitted_retained_allocations(),
        retained.total_admitted_retained_allocations(),
    );
    drop(host);
    assert_eq!(account.snapshot(), contained);
    assert!(output_drops.borrow().is_empty());
    assert_eq!(*proof_drops.borrow(), ["proof"]);
    assert_eq!(probe.counts(), frozen_counts);
    assert_eq!(factory.snapshot().total_admitted_accounts(), 1);
}
