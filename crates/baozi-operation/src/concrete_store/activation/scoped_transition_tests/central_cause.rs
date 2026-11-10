use super::*;

#[test]
fn semantic_false_accept_preserves_contract_cause_through_resumed_rich_failure_panic() {
    let account = account();
    let canonical_element_inspected = Cell::new(false);
    let body_capture_dropped = Cell::new(false);
    let capture_poison_seen = Cell::new(true);
    let capture_poison_events = Cell::new(u128::MAX);
    let rich_failure_validator_entered = Cell::new(false);
    let body_capture = TransitionDropMarker {
        dropped: &body_capture_dropped,
        account: &account,
        poison_seen_while_dropping: &capture_poison_seen,
        poison_event_count_while_dropping: &capture_poison_events,
    };
    let body_account = &account;
    let inspected = &canonical_element_inspected;
    let mut invocation_host = TestInvocationHost::new();
    {
        let finish = invocation_host
            .run_body(move |parts, disposition| {
                let _capture = &body_capture;
                let CanonicalBodyParts {
                    startup,
                    output,
                    proof,
                    activation,
                    validation,
                    ..
                } = parts;
                let (operation, keys) = operation(startup, body_account);
                let (policy, evidence_limits, failure, success) = scoped_shape(keys);
                let owner = operation
                    .into_session_outcome_owner(policy, evidence_limits, failure)
                    .unwrap();
                let mut description = Some(TestActivationDescription::new(success, ()));
                let outcome: TestOutcome<'_> = owner
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let ((), completion) = planning
                            .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                            .unwrap();
                        let permit = match planning
                            .seal_activation(description.take().unwrap(), completion)
                        {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                        permit.activate(|mut lane, _plan| {
                            let root = lane
                                .try_retain_sequence(
                                    SequencePlan::<u8>::new(1).unwrap(),
                                    CanonicalTag,
                                    |writer| writer.push(7),
                                )
                                .unwrap();
                            lane.complete(root, ())
                        })
                    })
                    .unwrap();
                let CanonicalActivationOutcome::Activated(activated) = outcome else {
                    panic!("the real canonical sequence must activate");
                };
                assert!(!body_account.snapshot().poisoned());
                let outcome = activated
                    .validate_centrally(
                        proof,
                        validation,
                        CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                        |lane, _output| {
                            let root = lane
                                .try_retain_sequence(
                                    SequencePlan::<u8>::new(1).unwrap(),
                                    CanonicalTag,
                                    |writer| writer.push(9),
                                )
                                .unwrap();
                            lane.complete(root)
                        },
                        |planning, _output| {
                            planning.validate_with(
                                |_| Ok(()),
                                |session, canonical, _evidence, &mut ()| {
                                    assert!(!session.visit_canonical_sequence(
                                        canonical,
                                        1,
                                        |index, value| {
                                            assert_eq!(index, 0);
                                            assert_eq!(value, 7);
                                            inspected.set(true);
                                            false
                                        }
                                    )?);
                                    Ok(CanonicalValidationDecision::<(), TestRejection>::accept(()))
                                },
                            )
                        },
                    )
                    .unwrap();
                let CanonicalCentralValidationOutcome::Rejected(rejected) = outcome else {
                    panic!(
                        "semantic false followed by Accept must be a central contract rejection"
                    );
                };
                assert!(inspected.get());
                assert!(matches!(
                    rejected.cause(),
                    StoreCanonicalRejection::Runtime(
                        StoreRuntimeError::CanonicalValidationContractRejected
                    )
                ));
                assert_eq!(rejected.canonical_len(), 0);
                rejected.request_finish(disposition).unwrap();
                assert!(!body_capture.dropped.get());
                assert!(!body_account.snapshot().poisoned());
            })
            .unwrap();
        assert!(canonical_element_inspected.get());
        assert!(body_capture_dropped.get());
        assert!(!capture_poison_seen.get());
        assert_eq!(capture_poison_events.get(), 0);
        assert!(!account.snapshot().poisoned());
        let CanonicalInvocationCompletion {
            outcome,
            failure_fallback,
            ..
        } = finish.resume();
        let CanonicalInvocationOutcome::CentralRejected(rejected) = outcome else {
            panic!("the original central contract rejection must resume after body cleanup");
        };
        assert!(matches!(
            rejected.cause(),
            StoreCanonicalRejection::Runtime(
                StoreRuntimeError::CanonicalValidationContractRejected
            )
        ));
        assert_eq!(rejected.canonical_len(), 0);
        let validation = rejected
            .validate_rich_failure(failure_fallback, |session, root| {
                rich_failure_validator_entered.set(true);
                assert!(body_capture_dropped.get());
                assert!(!account.snapshot().poisoned());
                assert!(session.visit_sequence(root, |_, value| value == 9).unwrap());
                panic!("injected rich-failure validation panic after central contract rejection");
            })
            .unwrap();
        assert!(rich_failure_validator_entered.get());
        let RootedFailureValidation::Fallback(fallback) = validation else {
            panic!("the protected rich-failure validator panic must prepare minimal fallback");
        };
        let (prepared, _) = fallback.into_parts();
        let published = prepared.handoff();
        let crate::StoreTerminalCause::Failure(cause) =
            published.ledger_evidence.header().terminal_cause()
        else {
            panic!("minimal fallback must publish the original central terminal failure");
        };
        assert_eq!(
            cause.class(),
            crate::StorePreparationFailureClass::ProductGraph
        );
        assert_eq!(cause.code(), CANONICAL_VALIDATION_CONTRACT_FAILURE_CODE);
        assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
        assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
        assert!(!account.snapshot().poisoned());
        drop(published);
    }
    drop(invocation_host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
