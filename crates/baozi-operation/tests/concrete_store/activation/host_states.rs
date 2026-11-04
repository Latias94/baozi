use super::*;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalInvocationOutcome, CanonicalOperationPublication,
};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;

type BodyPayloadHost<T, Proof> = CanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    T,
    Proof,
    (),
    ValidationRejection,
    GenerationRejection,
    ScalarSequence<u8>,
    SequenceTag,
    ScalarSequence<u8>,
    EvidenceLeafTag,
>;

type PayloadCentral<'inv, T, Proof> = CanonicalCentralValidationOutcome<
    'inv,
    TestOwner,
    TestProgress,
    TestUnit,
    T,
    Proof,
    ValidationRejection,
    ScalarSequence<u8>,
    SequenceTag,
    ScalarSequence<u8>,
    EvidenceLeafTag,
>;

fn central_with_payloads<'inv, T, Proof>(
    parts: CanonicalBodyParts<
        'inv,
        TestOwner,
        TestProgress,
        TestUnit,
        T,
        Proof,
        (),
        ValidationRejection,
    >,
    account: &PhysicalStorageAccount,
    output: T,
    proof: Proof,
) -> PayloadCentral<'inv, T, Proof> {
    let (operation, keys) = operation(parts.startup, account);
    let mut output = Some(output);
    let activated = plan_scoped_activation!(
        operation,
        parts.output,
        parts.activation,
        scoped_activation_shape_with_validation_work(keys, 1, 1, NonZeroU64::new(2).unwrap()),
        |mut lane| {
            let root = lane
                .try_retain_sequence(SequencePlan::<u8>::new(1).unwrap(), SequenceTag, |writer| {
                    writer.push(7)
                })
                .unwrap();
            lane.complete::<_, (), _, _>(root, output.take().unwrap())
        },
    )
    .unwrap();
    let CanonicalActivationOutcome::Activated(activated) = activated else {
        panic!("the exact borrowed output fixture must activate")
    };
    let mut proof = Some(proof);
    activated
        .validate_centrally(
            parts.proof,
            parts.validation,
            CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
            |lane, _| {
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(9),
                    )
                    .unwrap();
                lane.complete(root)
            },
            |planning, _| {
                planning.validate_with(
                    |_| Ok(()),
                    |session, canonical, evidence, &mut ()| {
                        assert!(session.visit_canonical_sequence(
                            canonical,
                            1,
                            |_, value| value == 7
                        )?);
                        assert!(session.visit_outcome_evidence_sequence(
                            evidence,
                            1,
                            |_, value| value == 9
                        )?);
                        Ok(
                            CanonicalValidationDecision::<_, ValidationRejection>::accept(
                                proof.take().unwrap(),
                            ),
                        )
                    },
                )
            },
        )
        .unwrap()
}

struct BorrowedPayload<'a> {
    label: u8,
    drops: &'a std::cell::RefCell<Vec<u8>>,
    account: &'a PhysicalStorageAccount,
    panic_on_drop: bool,
}

impl Drop for BorrowedPayload<'_> {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.label);
        let snapshot = self.account.snapshot();
        assert!(
            !snapshot.poisoned(),
            "all host values must drop before physical containment"
        );
        assert!(snapshot.live_retained_allocations() > 0);
        if self.panic_on_drop {
            match self.label {
                0 => panic!("capture payload panic"),
                1 => panic!("output payload panic"),
                _ => panic!("proof payload panic"),
            }
        }
    }
}

#[test]
fn retained_body_borrowed_payloads_drop_once_before_storage_after_forgotten_finish() {
    for forget_resumed_final in [false, true] {
        for panic_mask in 0_u8..=3 {
            let factory = factory(1);
            let account = account(&factory);
            let drops = std::cell::RefCell::new(Vec::new());
            let mut host = BodyPayloadHost::<BorrowedPayload<'_>, BorrowedPayload<'_>>::new();
            let mut output = Some(BorrowedPayload {
                label: 1,
                drops: &drops,
                account: &account,
                panic_on_drop: panic_mask & 1 != 0,
            });
            let mut proof = Some(BorrowedPayload {
                label: 2,
                drops: &drops,
                account: &account,
                panic_on_drop: panic_mask & 2 != 0,
            });
            let body_account = &account;
            let finish = host
                .run_body(move |parts, disposition| {
                    central_with_payloads(
                        parts,
                        body_account,
                        output.take().unwrap(),
                        proof.take().unwrap(),
                    )
                    .request_finish(disposition)
                    .unwrap();
                })
                .unwrap();
            assert!(drops.borrow().is_empty());
            #[allow(
                clippy::forget_non_drop,
                reason = "the regression abandons finish or its resumed final view while actual owners stay in the host"
            )]
            if forget_resumed_final {
                let CanonicalInvocationOutcome::Candidate(candidate) = finish.resume().outcome
                else {
                    panic!("the recorded payloads must resume as the original candidate")
                };
                std::mem::forget(candidate);
            } else {
                std::mem::forget(finish);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(host)));
            assert_eq!(*drops.borrow(), [2, 1]);
            if panic_mask == 0 {
                assert!(result.is_ok());
                assert!(!account.snapshot().poisoned());
                assert_eq!(account.snapshot().live_retained_allocations(), 0);
            } else {
                let expected = if panic_mask & 2 != 0 {
                    "proof payload panic"
                } else {
                    "output payload panic"
                };
                assert_eq!(result.unwrap_err().downcast_ref::<&str>(), Some(&expected));
                assert!(account.snapshot().poisoned());
                assert!(account.snapshot().live_retained_allocations() > 0);
            }
        }
    }
}

#[test]
fn retained_body_original_panic_survives_capture_and_both_borrowed_payload_failures() {
    let factory = factory(1);
    let account = account(&factory);
    let drops = std::cell::RefCell::new(Vec::new());
    let mut host = BodyPayloadHost::<BorrowedPayload<'_>, BorrowedPayload<'_>>::new();
    let mut output = Some(BorrowedPayload {
        label: 1,
        drops: &drops,
        account: &account,
        panic_on_drop: true,
    });
    let mut proof = Some(BorrowedPayload {
        label: 2,
        drops: &drops,
        account: &account,
        panic_on_drop: true,
    });
    let capture = BorrowedPayload {
        label: 0,
        drops: &drops,
        account: &account,
        panic_on_drop: true,
    };
    let body_account = &account;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = host.run_body(move |parts, disposition| {
            let _capture = &capture;
            central_with_payloads(
                parts,
                body_account,
                output.take().unwrap(),
                proof.take().unwrap(),
            )
            .request_finish(disposition)
            .unwrap();
            panic!("original retained body panic");
        });
    }));
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"original retained body panic")
    );
    assert_eq!(*drops.borrow(), [0, 2, 1]);
    assert!(account.snapshot().poisoned());
    assert!(account.snapshot().live_retained_allocations() > 0);
    drop(host);
    assert_eq!(*drops.borrow(), [0, 2, 1]);
}

#[test]
fn retained_operation_body_disposes_capture_before_unchanged_or_requested_fallback() {
    for code in [None, Some(71)] {
        let factory = factory(1);
        let account = account(&factory);
        let mut host = TestInvocationHost::<()>::new();
        let capture_dropped = Cell::new(false);
        let capture = DropProbe(&capture_dropped);
        let body_account = &account;
        let dropped_in_body = &capture_dropped;
        let finish = host
            .run_body(move |parts, disposition| {
                let _capture = &capture;
                let (operation, _) = operation(parts.startup, body_account);
                if let Some(code) = code {
                    operation
                        .request_minimal_fallback(disposition, code)
                        .unwrap();
                } else {
                    operation.request_unchanged(disposition).unwrap();
                }
                assert!(!dropped_in_body.get());
            })
            .unwrap();
        assert!(capture_dropped.get());
        let CanonicalInvocationOutcome::Operation(operation) = finish.resume().outcome else {
            panic!("the actual unopened operation must retain its requested terminal")
        };
        let evidence = match operation.finish(final_progress()).unwrap() {
            CanonicalOperationPublication::Unchanged(
                PreparedStoreEvidenceOnlyOutcome::Success(prepared),
            ) => {
                assert_eq!(code, None);
                let published = prepared.handoff();
                assert_eq!(
                    published.ledger_evidence.header().terminal_cause(),
                    StoreTerminalCause::Success
                );
                published.ledger_evidence
            }
            CanonicalOperationPublication::MinimalFallback(prepared) => {
                let published = prepared.handoff();
                let StoreTerminalCause::Failure(cause) =
                    published.ledger_evidence.header().terminal_cause()
                else {
                    panic!("the requested fallback cannot publish successful evidence")
                };
                assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
                assert_eq!(Some(cause.code()), code);
                drop(published.fallback);
                published.ledger_evidence
            }
            CanonicalOperationPublication::Unchanged(
                PreparedStoreEvidenceOnlyOutcome::MinimalFallback(_),
            ) => {
                panic!("clean unchanged work must preserve evidence-only success")
            }
        };
        assert_eq!(evidence.header().current_live_bytes(), 0);
        assert_eq!(evidence.header().active_reservations(), 0);
        drop(evidence);
        drop(host);
        assert!(!account.snapshot().poisoned());
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn retained_never_activated_body_preserves_rich_failure_construction_after_cleanup() {
    let factory = factory(1);
    let account = account(&factory);
    let mut host = TestInvocationHost::<(), (), PreflightRejection>::new();
    let capture_dropped = Cell::new(false);
    let capture = DropProbe(&capture_dropped);
    let body_account = &account;
    {
        let finish = host
            .run_body(move |parts, disposition| {
                let _capture = &capture;
                let (operation, keys) = operation(parts.startup, body_account);
                let (never, _) = rejected_session_from_operation(
                    operation,
                    parts.output,
                    parts.activation,
                    keys,
                    1,
                );
                never.request_finish(disposition).unwrap();
            })
            .unwrap();
        assert!(capture_dropped.get());
        let completion = finish.resume();
        let CanonicalInvocationOutcome::NeverActivated(never) = completion.outcome else {
            panic!("the exact never-activated state must survive body cleanup")
        };
        assert!(matches!(
            never.cause(),
            StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
        ));
        let built = never
            .build_failure_evidence(|lane| {
                let root = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        EvidenceLeafTag,
                        |writer| writer.push(7),
                    )
                    .unwrap();
                lane.complete(root)
            })
            .unwrap();
        let FailureEvidenceOutcome::Rooted(rooted) = built else {
            panic!("healthy deferred rejection must retain rich failure, not degrade to fallback")
        };
        let validation = rooted
            .validate_rich_failure(completion.failure_fallback, |session, root| {
                session
                    .visit_sequence(root, |_index, value| value == 7)
                    .unwrap()
            })
            .unwrap();
        let RootedFailureValidation::Valid(validated) = validation else {
            panic!("the same admitted rich evidence must validate")
        };
        let (prepared, cause) = validated
            .try_prepare(final_progress())
            .unwrap()
            .into_parts();
        assert!(matches!(
            cause,
            StoreNeverActivatedCause::Requested(PreflightRejection::Unsupported)
        ));
        let PreparedStoreFailureOutcome::RichFailure(prepared) = prepared else {
            panic!("a clean deferred failure must retain its rich publication")
        };
        let published = prepared.handoff();
        let StoreTerminalCause::Failure(cause) =
            published.ledger_evidence.header().terminal_cause()
        else {
            panic!("the rejected body cannot publish successful evidence")
        };
        assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
        assert_eq!(cause.code(), 11);
        drop(published);
    }
    drop(host);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
