use super::*;
use crate::concrete_store::tests::{
    TestActivationDescription, TestOwner, TestProgress, TestUnit, test_account, test_factory,
    test_operation_plan,
};
use crate::{
    AccountPoisonReason, DimensionPolicy, OperationControl, PhysicalStorageAccount,
    PhysicalStorageAccountSnapshot,
};
use std::cell::{Cell, RefCell};

type Host = CanonicalInvocationHost<
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    (),
    (),
    (),
    ScalarSequence<u8>,
    (),
    ScalarSequence<u8>,
    (),
>;

type Candidate<'host> = CanonicalCandidateStoreSession<
    'host,
    TestOwner,
    TestProgress,
    TestUnit,
    (),
    (),
    ScalarSequence<u8>,
    (),
    ScalarSequence<u8>,
    (),
>;

fn validated_candidate<'host>(
    host: &'host mut Host,
    account: &PhysicalStorageAccount,
) -> (
    Candidate<'host>,
    CanonicalCauseSlot<'host, CanonicalFreezeInterruption>,
) {
    let finish = host
        .run_body(|parts, disposition| {
            let (ledger, start, keys) = test_operation_plan(u64::MAX, u64::MAX, u64::MAX, 1);
            let (_, _, terminal) = start.keys_for_start_validation();
            let (domain, _, _) = ledger.into_parts();
            let requirement =
                RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(1, 1, 1)
                    .unwrap();
            let failure = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
                terminal,
                1,
                1,
                1,
                512 + requirement.attempted_work_limit(),
                512 + requirement.consumed_work_limit(),
            )
            .unwrap();
            let ledger = crate::concrete_store::StoreLedgerPlan::try_new(
                domain,
                crate::ledger::LedgerLimits::new(
                    u64::MAX,
                    [
                        (
                            keys.dimension,
                            DimensionPolicy::reservable_bytes(256, 256, u64::MAX, u64::MAX)
                                .with_published_outcome_evidence_limit(u64::MAX),
                        ),
                        (
                            keys.work_dimension,
                            DimensionPolicy::reservable_bytes(256, 256, u64::MAX, u64::MAX),
                        ),
                        (terminal, requirement.dimension_policy()),
                    ],
                )
                .unwrap(),
                &[keys.phase],
            )
            .unwrap();
            let start = crate::concrete_store::RetainedStoreStartPlan::new(
                RetainedStoreWriterPlan::new(
                    keys.phase,
                    keys.dimension,
                    keys.dimension,
                    RetainedStoreBatchLimits::new(32, 8).unwrap(),
                ),
                crate::concrete_store::RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
                crate::concrete_store::StoreFallbackEnvelope::new(91),
                keys.dimension,
                1,
                keys.dimension,
                1,
                failure,
            );
            let operation = CanonicalBodyOperation::try_start(
                parts.startup,
                OperationControl::default(),
                ledger,
                account,
                start,
            )
            .unwrap();
            let owner = operation
                .into_session_outcome_owner(
                    CanonicalActivationSessionPolicy::new(
                        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
                        RetainedStoreWriterRole::new(keys.phase, keys.dimension, keys.dimension),
                        CanonicalMaterializationRole::new(keys.work_dimension, keys.dimension),
                        CanonicalValidationRole::new(
                            keys.phase,
                            keys.work_dimension,
                            keys.dimension,
                        ),
                        keys.dimension,
                        keys.dimension,
                    ),
                    RetainedStoreBatchLimits::new(2, 2).unwrap(),
                    CanonicalFailureRequirements::new(CanonicalTableShape::new(1, 1).unwrap()),
                )
                .unwrap();
            let success = CanonicalSuccessRequirements::new(
                CanonicalTableShape::new(1, 1).unwrap(),
                2,
                1,
                NonZeroU64::new(2).unwrap(),
            )
            .unwrap();
            let mut description = Some(TestActivationDescription::new(success, ()));
            let activated = owner
                .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                    let ((), completion) = planning
                        .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                        .unwrap();
                    let permit =
                        match planning.seal_activation(description.take().unwrap(), completion) {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                    permit.activate(|mut lane, _plan| {
                        let root = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                (),
                                |writer| writer.push(7),
                            )
                            .unwrap();
                        lane.complete::<_, (), _, _>(root, ())
                    })
                })
                .unwrap();
            let CanonicalActivationOutcome::Activated(activated) = activated else {
                panic!("the concrete canonical root must activate");
            };
            let validated = activated
                .validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(TestProgress::Build, TestUnit::Items),
                    |lane, _output| {
                        let root = lane
                            .try_retain_sequence(
                                SequencePlan::<u8>::new(1).unwrap(),
                                (),
                                |writer| writer.push(9),
                            )
                            .unwrap();
                        lane.complete(root)
                    },
                    |planning, _output| {
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
                                Ok(CanonicalValidationDecision::<_, ()>::accept(()))
                            },
                        )
                    },
                )
                .unwrap();
            let CanonicalCentralValidationOutcome::Candidate(candidate) = validated else {
                panic!("both concrete roots must have a valid success closure");
            };
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
        panic!("the original validated candidate must resume after body cleanup");
    };
    (candidate.state, completion.generation.interruption)
}

struct CleanupProbe<'scope> {
    label: &'static str,
    account: &'scope PhysicalStorageAccount,
    before: PhysicalStorageAccountSnapshot,
    recorded: &'scope Cell<Option<PhysicalStorageAccountSnapshot>>,
    events: &'scope RefCell<Vec<&'static str>>,
    panic_on_drop: bool,
}

impl Drop for CleanupProbe<'_> {
    fn drop(&mut self) {
        let snapshot = self.account.snapshot();
        assert!(snapshot.poisoned());
        assert!(snapshot.poison_event_count() > 0);
        assert_eq!(snapshot.first_poison_evidence().unwrap().event_ordinal(), 1);
        assert_eq!(
            snapshot.live_retained_bytes(),
            self.before.live_retained_bytes()
        );
        assert_eq!(
            snapshot.live_retained_allocations(),
            self.before.live_retained_allocations()
        );
        assert_eq!(
            snapshot.first_poison_evidence().unwrap().reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        if let Some(recorded) = self.recorded.get() {
            assert_eq!(snapshot, recorded);
        } else {
            self.recorded.set(Some(snapshot));
        }
        self.events.borrow_mut().push(self.label);
        if self.panic_on_drop {
            panic!("the proof destructor fails after raw freeze containment");
        }
    }
}

#[test]
fn raw_freeze_mismatch_preserves_recorded_error_through_panicking_payload_cleanup() {
    let factory = test_factory();
    let account = test_account(&factory);
    let foreign_factory = test_factory();
    let foreign_account = test_account(&foreign_factory);
    let mut host = Host::new();
    let mut foreign_host = Host::new();
    let recorded = Cell::new(None);
    let events = RefCell::new(Vec::new());
    {
        let (candidate, interruption) = validated_candidate(&mut host, &account);
        let (foreign_candidate, _foreign_interruption) =
            validated_candidate(&mut foreign_host, &foreign_account);
        let CanonicalCandidateStoreSession {
            owner,
            destinations,
            _failure_work: failure_work,
        } = candidate;
        let (proof, owner) = owner.into_parts();
        let (output, validated) = owner.into_parts();
        let identity = output.identity();
        drop((proof, output));
        let (raw, closure, canonical_root, evidence_root, domain) = validated.into_storage_parts();
        assert_eq!(closure.canonical_member_count(), 1);
        assert_eq!(closure.outcome_evidence_member_count(), 1);
        let _ = closure;

        let (foreign_proof, foreign_owner) = foreign_candidate.owner.into_parts();
        let (foreign_output, foreign_validated) = foreign_owner.into_parts();
        let (foreign_raw, foreign_closure, _, _, foreign_domain) =
            foreign_validated.into_storage_parts();
        assert_eq!(foreign_closure.canonical_member_count(), 1);
        assert_eq!(foreign_closure.outcome_evidence_member_count(), 1);
        drop((foreign_proof, foreign_output, foreign_raw, foreign_domain));

        let before = account.snapshot();
        assert!(!before.poisoned());
        assert!(before.live_retained_bytes() > 0);
        assert!(before.live_retained_allocations() > 0);
        let mut output = Some(CleanupProbe {
            label: "output",
            account: &account,
            before,
            recorded: &recorded,
            events: &events,
            panic_on_drop: false,
        });
        let mut proof = Some(CleanupProbe {
            label: "proof",
            account: &account,
            before,
            recorded: &recorded,
            events: &events,
            panic_on_drop: true,
        });
        let order = Cell::new(payload_owner::PayloadOrder::ProofBeforeOutput);
        assert!(interruption.is_vacant_for(identity));
        let candidate = CanonicalCandidateStoreSession {
            owner: PayloadOwner::new(
                PayloadLoan::new(&mut proof, &order, identity),
                PayloadOwner::new(
                    PayloadLoan::new(&mut output, &order, identity),
                    ValidatedStoreSuccess::from_storage_parts(
                        raw,
                        foreign_closure,
                        canonical_root,
                        evidence_root,
                        domain,
                    ),
                ),
            ),
            destinations,
            _failure_work: failure_work,
        };
        let error = catch_unwind(AssertUnwindSafe(|| {
            match issue_dynamic(
                candidate,
                interruption,
                FinalProgress::new(TestProgress::Build, TestUnit::Items, 2),
            ) {
                Err(error) => error,
                Ok(_) => panic!("a foreign success closure cannot issue a freeze permit"),
            }
        }))
        .expect(
            "cleanup must preserve the recorded raw error without a second account-anchor lookup",
        );

        assert!(output.is_none());
        assert!(proof.is_none());
        assert_eq!(*events.borrow(), ["proof", "output"]);
        let recorded = recorded
            .get()
            .expect("raw containment precedes payload cleanup");
        assert_eq!(
            error.cause.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(error.cause.account_poisoned().snapshot(), recorded);
        assert_eq!(account.snapshot(), recorded);
    }
    drop(host);
    drop(foreign_host);
    assert!(!foreign_account.snapshot().poisoned());
    assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot(), recorded.get().unwrap());
    assert_eq!(*events.borrow(), ["proof", "output"]);
}
