# Constructing an invocation through its final handoff

These examples are external consumers of the public integration facades. Each
block supplies its own owner, progress vocabulary, catalog, account, and plans.
The positive examples execute the real retained-store implementation, including
terminal preparation and handoff.

## An unchanged operation finishes outside its body

The body records an unchanged disposition. Only the resumed operation completion
can prepare and hand off the evidence-only success.

```rust
# use std::num::{NonZeroU32, NonZeroU64};
# use baozi_operation::__retained_store_integration::activation::{
#     CanonicalBodyOperation, CanonicalInvocationHost, CanonicalInvocationOutcome,
#     CanonicalOperationPublication,
# };
# use baozi_operation::__retained_store_integration::authority::{
#     PreparedStoreEvidenceOnlyOutcome, RetainedStoreSharePlan, RetainedStoreStartPlan,
#     RetainedStoreWriterPlan,
# };
# use baozi_operation::{
#     DimensionPolicy, FinalProgress, LedgerLimits, OperationControl, OperationOwner,
#     PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
#     PhysicalStorageAccountLimits, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
#     RetainedStoreFailureTerminalWorkPlan, StoreDimensionSemanticId, StoreFallbackEnvelope,
#     StoreKeyDomain, StoreLedgerPlan, StorePhaseSemanticId, StoreSemanticCatalogId,
#     StoreTerminalCause,
# };
# struct Owner;
# impl OperationOwner for Owner {}
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Phase { Build }
# impl ProgressPhase for Phase {
#     fn order(self) -> u32 { 0 }
#     fn stable_id(self) -> &'static str { "build" }
# }
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Unit { Items }
# impl ProgressUnit for Unit {
#     fn stable_id(self) -> &'static str { "items" }
# }
let factory = PhysicalStorageAccountFactory::try_new(
    PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
).unwrap();
let account = factory.try_create_account(
    PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap(),
).unwrap();
let catalog = StoreSemanticCatalogId::new(*b"baozi-doc-store!", NonZeroU32::MIN);
let domain = StoreKeyDomain::try_new(catalog).unwrap();
let phase = domain.try_admit_phase(
    StorePhaseSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let ordinary = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let protected = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::new(2).unwrap()),
).unwrap();
let failure_work = RetainedStoreFailureTerminalWorkPlan::new(
    protected, 1, u64::MAX, u64::MAX,
).unwrap();
let policy = DimensionPolicy::reservable_bytes(
    failure_work.ordinary_attempted_work_limit(),
    failure_work.ordinary_consumed_work_limit(), u64::MAX, u64::MAX,
).with_published_outcome_evidence_limit(u64::MAX);
let limits = LedgerLimits::new(u64::MAX, [
    (ordinary, policy), (protected, failure_work.requirement().dimension_policy()),
]).unwrap();
let ledger = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
let start = RetainedStoreStartPlan::new(
    RetainedStoreWriterPlan::new(
        phase, ordinary, ordinary, RetainedStoreBatchLimits::new(32, 8).unwrap(),
    ),
    RetainedStoreSharePlan::new(ordinary, ordinary), StoreFallbackEnvelope::new(91),
    ordinary, 1, ordinary, 1, failure_work,
);
let mut startup = Some((ledger, start));
let mut host = CanonicalInvocationHost::<Owner, Phase, Unit, (), ()>::new();
let finish = host.run_body(|parts, disposition| {
    let (ledger, start) = startup.take().unwrap();
    let operation = CanonicalBodyOperation::try_start(
        parts.startup, OperationControl::default(), ledger, &account, start,
    ).unwrap();
    operation.request_unchanged(disposition).unwrap();
}).unwrap();
let finish = finish.admit_terminal_catalog(
    &[phase.semantic_id()], &[ordinary.semantic_id(), protected.semantic_id()],
).unwrap();
let CanonicalInvocationOutcome::Operation(operation) = finish.resume().outcome else {
    panic!("the host must restore the requested operation");
};
let publication = operation.finish(FinalProgress::new(Phase::Build, Unit::Items, 0)).unwrap();
let CanonicalOperationPublication::Unchanged(PreparedStoreEvidenceOnlyOutcome::Success(prepared)) =
    publication else { panic!("the unchanged operation must succeed"); };
let published = prepared.handoff();
assert!(matches!(published.ledger_evidence.header().terminal_cause(), StoreTerminalCause::Success));
drop(published);
drop(host);
assert_eq!(account.snapshot().live_retained_bytes(), 0);
assert_eq!(account.snapshot().live_retained_allocations(), 0);
assert!(!account.snapshot().poisoned());
```

The following block repeats the same construction and final consumer. Its only
forbidden expression attempts preparation on the body operation. The branch is
not executed; its early return prevents a second use of a consumed operation if
that method were available. Removing the branch leaves the positive lifecycle.

```compile_fail,E0599
# use std::num::{NonZeroU32, NonZeroU64};
# use baozi_operation::__retained_store_integration::activation::{
#     CanonicalBodyOperation, CanonicalInvocationHost, CanonicalInvocationOutcome,
#     CanonicalOperationPublication,
# };
# use baozi_operation::__retained_store_integration::authority::{
#     PreparedStoreEvidenceOnlyOutcome, RetainedStoreSharePlan, RetainedStoreStartPlan,
#     RetainedStoreWriterPlan,
# };
# use baozi_operation::{
#     DimensionPolicy, FinalProgress, LedgerLimits, OperationControl, OperationOwner,
#     PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
#     PhysicalStorageAccountLimits, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
#     RetainedStoreFailureTerminalWorkPlan, StoreDimensionSemanticId, StoreFallbackEnvelope,
#     StoreKeyDomain, StoreLedgerPlan, StorePhaseSemanticId, StoreSemanticCatalogId,
#     StoreTerminalCause,
# };
# struct Owner;
# impl OperationOwner for Owner {}
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Phase { Build }
# impl ProgressPhase for Phase {
#     fn order(self) -> u32 { 0 }
#     fn stable_id(self) -> &'static str { "build" }
# }
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Unit { Items }
# impl ProgressUnit for Unit {
#     fn stable_id(self) -> &'static str { "items" }
# }
let factory = PhysicalStorageAccountFactory::try_new(
    PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
).unwrap();
let account = factory.try_create_account(
    PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap(),
).unwrap();
let catalog = StoreSemanticCatalogId::new(*b"baozi-doc-store!", NonZeroU32::MIN);
let domain = StoreKeyDomain::try_new(catalog).unwrap();
let phase = domain.try_admit_phase(
    StorePhaseSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let ordinary = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let protected = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::new(2).unwrap()),
).unwrap();
let failure_work = RetainedStoreFailureTerminalWorkPlan::new(
    protected, 1, u64::MAX, u64::MAX,
).unwrap();
let policy = DimensionPolicy::reservable_bytes(
    failure_work.ordinary_attempted_work_limit(),
    failure_work.ordinary_consumed_work_limit(), u64::MAX, u64::MAX,
).with_published_outcome_evidence_limit(u64::MAX);
let limits = LedgerLimits::new(u64::MAX, [
    (ordinary, policy), (protected, failure_work.requirement().dimension_policy()),
]).unwrap();
let ledger = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
let start = RetainedStoreStartPlan::new(
    RetainedStoreWriterPlan::new(
        phase, ordinary, ordinary, RetainedStoreBatchLimits::new(32, 8).unwrap(),
    ),
    RetainedStoreSharePlan::new(ordinary, ordinary), StoreFallbackEnvelope::new(91),
    ordinary, 1, ordinary, 1, failure_work,
);
let mut startup = Some((ledger, start));
let mut host = CanonicalInvocationHost::<Owner, Phase, Unit, (), ()>::new();
let finish = host.run_body(|parts, disposition| {
    let (ledger, start) = startup.take().unwrap();
    let operation = CanonicalBodyOperation::try_start(
        parts.startup, OperationControl::default(), ledger, &account, start,
    ).unwrap();
    if false {
        let _ = operation.prepare_evidence_only_success(FinalProgress::new(Phase::Build, Unit::Items, 0));
        return;
    }
    operation.request_unchanged(disposition).unwrap();
}).unwrap();
let finish = finish.admit_terminal_catalog(
    &[phase.semantic_id()], &[ordinary.semantic_id(), protected.semantic_id()],
).unwrap();
let CanonicalInvocationOutcome::Operation(operation) = finish.resume().outcome else {
    panic!("the host must restore the requested operation");
};
let publication = operation.finish(FinalProgress::new(Phase::Build, Unit::Items, 0)).unwrap();
let CanonicalOperationPublication::Unchanged(PreparedStoreEvidenceOnlyOutcome::Success(prepared)) =
    publication else { panic!("the unchanged operation must succeed"); };
let published = prepared.handoff();
assert!(matches!(published.ledger_evidence.header().terminal_cause(), StoreTerminalCause::Success));
drop(published);
drop(host);
assert_eq!(account.snapshot().live_retained_bytes(), 0);
assert_eq!(account.snapshot().live_retained_allocations(), 0);
assert!(!account.snapshot().poisoned());
```

## A constructed candidate reaches joint publication

This consumer also implements a small trusted producer description through the
public producer facade. Its request identity binds the failure envelope and
success facts to the same description. Semantic work precedes activation; central
validation visits the actual canonical and evidence roots before accepting a
proof. The final consumer resumes that exact candidate, issues a freeze permit,
prepares under its joint seal, and hands off both owning stores.

```rust
# use std::num::{NonZeroU32, NonZeroU64};
# use baozi_operation::__retained_store_integration::activation::{
#     CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalActivationSessionPolicy,
#     CanonicalBodyOperation, CanonicalCauseSlot, CanonicalCentralValidationOutcome,
#     CanonicalFailureRequirements, CanonicalFreezeInterruption, CanonicalFreezePermitIssue,
#     CanonicalInvocationHost, CanonicalInvocationOutcome, CanonicalJointSuccess,
#     CanonicalMaterializationRole, CanonicalValidationDecision, CanonicalValidationProgress,
#     CanonicalValidationRole, RetainedStoreWriterRole,
# };
# use baozi_operation::__retained_store_integration::authority::{
#     RetainedStoreSharePlan, RetainedStoreStartPlan, RetainedStoreWriterPlan,
# };
# use baozi_operation::__retained_store_integration::producer::{
#     CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
#     CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
#     mint_canonical_activation_request,
# };
# use baozi_operation::{
#     DimensionPolicy, FinalProgress, LedgerLimits, OperationControl, OperationOwner,
#     PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
#     PhysicalStorageAccountLimits, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
#     RetainedStoreFailureTerminalWorkPlan, ScalarSequence, SequencePlan,
#     StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan,
#     StorePhaseSemanticId, StoreSemanticCatalogId, StoreTerminalCause,
# };
# struct Owner;
# impl OperationOwner for Owner {}
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Phase { Build }
# impl ProgressPhase for Phase {
#     fn order(self) -> u32 { 0 }
#     fn stable_id(self) -> &'static str { "build" }
# }
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Unit { Items }
# impl ProgressUnit for Unit {
#     fn stable_id(self) -> &'static str { "items" }
# }
# #[derive(Clone, Copy)]
# struct CanonicalTag;
# #[derive(Clone, Copy)]
# struct EvidenceTag;
# struct Description {
#     request: CanonicalActivationRequest,
#     success: CanonicalSuccessRequirements,
# }
# impl CanonicalActivationDescription for Description {
#     type Execution = ();
#     fn seal_activation<'scope>(
#         self, sealer: CanonicalActivationPlanSealer<'scope>,
#     ) -> CanonicalSealedActivationPlan<'scope, ()> {
#         sealer.seal(self.request, self.success, ())
#     }
# }
# fn body_interruption_slot<'slot>() -> CanonicalCauseSlot<'slot, CanonicalFreezeInterruption> {
#     panic!("a body has no final interruption slot")
# }
let factory = PhysicalStorageAccountFactory::try_new(
    PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
).unwrap();
let account = factory.try_create_account(
    PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap(),
).unwrap();
let catalog = StoreSemanticCatalogId::new(*b"baozi-doc-store!", NonZeroU32::MIN);
let domain = StoreKeyDomain::try_new(catalog).unwrap();
let phase = domain.try_admit_phase(
    StorePhaseSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let ordinary = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let protected = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::new(2).unwrap()),
).unwrap();
let failure_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
    protected, 1, 64, 64, u64::MAX, u64::MAX,
).unwrap();
let policy = DimensionPolicy::reservable_bytes(
    failure_work.ordinary_attempted_work_limit(),
    failure_work.ordinary_consumed_work_limit(), u64::MAX, u64::MAX,
).with_published_outcome_evidence_limit(u64::MAX);
let limits = LedgerLimits::new(u64::MAX, [
    (ordinary, policy), (protected, failure_work.requirement().dimension_policy()),
]).unwrap();
let ledger = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
let start = RetainedStoreStartPlan::new(
    RetainedStoreWriterPlan::new(
        phase, ordinary, ordinary, RetainedStoreBatchLimits::new(32, 8).unwrap(),
    ),
    RetainedStoreSharePlan::new(ordinary, ordinary), StoreFallbackEnvelope::new(91),
    ordinary, 1, ordinary, 1, failure_work,
);
type Host = CanonicalInvocationHost<
    Owner, Phase, Unit, u8, u8, (), (), (),
    ScalarSequence<u8>, CanonicalTag, ScalarSequence<u8>, EvidenceTag,
>;
let mut host = Host::new();
let mut startup = Some((ledger, start));
let finish = host.run_body(|parts, disposition| {
    let (ledger, start) = startup.take().unwrap();
    let operation = CanonicalBodyOperation::try_start(
        parts.startup, OperationControl::default(), ledger, &account, start,
    ).unwrap();
    let (identity, request) = mint_canonical_activation_request().unwrap();
    let failure = CanonicalFailureRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
    ).with_request_identity(identity);
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(), 2, 1, NonZeroU64::new(2).unwrap(),
    ).unwrap().with_request_identity(identity);
    let role = RetainedStoreWriterRole::new(phase, ordinary, ordinary);
    let policy = CanonicalActivationSessionPolicy::new(
        role, role, CanonicalMaterializationRole::new(ordinary, ordinary),
        CanonicalValidationRole::new(phase, ordinary, ordinary), ordinary, ordinary,
    );
    let owner = operation.into_session_outcome_owner(
        policy, RetainedStoreBatchLimits::new(2, 2).unwrap(), failure,
    ).unwrap();
    let mut description = Some(Description { request, success });
    let activation = owner.plan_and_activate_scoped(
        parts.output, parts.activation, |mut planning| {
            let ((), completion) = planning.with_semantic_work(|semantic| {
                semantic.run_unit(|| ()).unwrap();
                Ok::<(), ()>(())
            }).unwrap();
            match planning.seal_activation(description.take().unwrap(), completion) {
                CanonicalActivationSeal::Sealed(permit) => permit.activate(|mut lane, _| {
                    let root = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(), CanonicalTag,
                        |writer| writer.push(7),
                    ).unwrap();
                    lane.complete::<_, (), _, _>(root, 7_u8)
                }),
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        },
    ).unwrap();
    let CanonicalActivationOutcome::Activated(activated) = activation else {
        panic!("the sealed description must activate its canonical root");
    };
    let validation = activated.validate_centrally(
        parts.proof, parts.validation, CanonicalValidationProgress::new(Phase::Build, Unit::Items),
        |lane, output| {
            assert_eq!(*output, 7);
            let root = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(), EvidenceTag, |writer| writer.push(9),
            ).unwrap();
            lane.complete(root)
        },
        |planning, output| {
            assert_eq!(*output, 7);
            planning.validate_with(|_| Ok(()), |session, canonical, evidence, _| {
                assert!(session.visit_canonical_sequence(canonical, 1, |_, value| value == 7)?);
                assert!(session.visit_outcome_evidence_sequence(evidence, 1, |_, value| value == 9)?);
                Ok(CanonicalValidationDecision::<u8, ()>::accept(9))
            })
        },
    ).unwrap();
    let CanonicalCentralValidationOutcome::Candidate(candidate) = validation else {
        panic!("both exact roots must validate before a candidate exists");
    };
    assert_eq!((*candidate.output(), *candidate.proof()), (7, 9));
    candidate.request_finish(disposition).unwrap();
}).unwrap();
let finish = finish.admit_terminal_catalog(
    &[phase.semantic_id()], &[ordinary.semantic_id(), protected.semantic_id()],
).unwrap();
let (candidate, generation) = {
    let completion = finish.resume();
    let candidate = match completion.outcome {
        CanonicalInvocationOutcome::Candidate(candidate) => candidate,
        _ => panic!("resume must restore the actual validated candidate"),
    };
    (candidate, completion.generation)
};
assert_eq!((*candidate.output(), *candidate.proof()), (7, 9));
let permit = {
    let issue = candidate.try_issue_freeze_permit(
        generation.interruption, FinalProgress::new(Phase::Build, Unit::Items, 2),
    ).unwrap();
    match issue {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        _ => panic!("the final candidate must remain admissible"),
    }
};
let CanonicalJointSuccess { output, proof, store } =
    permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
assert_eq!((output, proof), (7, 9));
let published = store.handoff();
assert_eq!(published.canonical.root().get(0), Some(7));
assert_eq!(published.outcome_evidence.root().get(0), Some(9));
assert!(matches!(published.ledger_evidence.header().terminal_cause(), StoreTerminalCause::Success));
assert_eq!(published.ledger_evidence.dimension(ordinary).unwrap().committed_canonical_output(), 1);
drop(published);
drop(host);
assert_eq!(account.snapshot().live_retained_bytes(), 0);
assert_eq!(account.snapshot().live_retained_allocations(), 0);
assert!(!account.snapshot().poisoned());
```

The next block has the same producer, construction, validator, and final consumer.
Only the added body call is forbidden. Its explicitly typed slot helper diverges
instead of fabricating a permission, and is present in both blocks. The branch is
never executed; if the raw candidate exposed this method, the slot and progress
arguments would type-check, and its early return would avoid any subsequent use
of the consumed candidate. The positive example obtains its real final slot from
`resume`, after body cleanup.

```compile_fail,E0599
# use std::num::{NonZeroU32, NonZeroU64};
# use baozi_operation::__retained_store_integration::activation::{
#     CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalActivationSessionPolicy,
#     CanonicalBodyOperation, CanonicalCauseSlot, CanonicalCentralValidationOutcome,
#     CanonicalFailureRequirements, CanonicalFreezeInterruption, CanonicalFreezePermitIssue,
#     CanonicalInvocationHost, CanonicalInvocationOutcome, CanonicalJointSuccess,
#     CanonicalMaterializationRole, CanonicalValidationDecision, CanonicalValidationProgress,
#     CanonicalValidationRole, RetainedStoreWriterRole,
# };
# use baozi_operation::__retained_store_integration::authority::{
#     RetainedStoreSharePlan, RetainedStoreStartPlan, RetainedStoreWriterPlan,
# };
# use baozi_operation::__retained_store_integration::producer::{
#     CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
#     CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
#     mint_canonical_activation_request,
# };
# use baozi_operation::{
#     DimensionPolicy, FinalProgress, LedgerLimits, OperationControl, OperationOwner,
#     PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
#     PhysicalStorageAccountLimits, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
#     RetainedStoreFailureTerminalWorkPlan, ScalarSequence, SequencePlan,
#     StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan,
#     StorePhaseSemanticId, StoreSemanticCatalogId, StoreTerminalCause,
# };
# struct Owner;
# impl OperationOwner for Owner {}
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Phase { Build }
# impl ProgressPhase for Phase {
#     fn order(self) -> u32 { 0 }
#     fn stable_id(self) -> &'static str { "build" }
# }
# #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
# enum Unit { Items }
# impl ProgressUnit for Unit {
#     fn stable_id(self) -> &'static str { "items" }
# }
# #[derive(Clone, Copy)]
# struct CanonicalTag;
# #[derive(Clone, Copy)]
# struct EvidenceTag;
# struct Description {
#     request: CanonicalActivationRequest,
#     success: CanonicalSuccessRequirements,
# }
# impl CanonicalActivationDescription for Description {
#     type Execution = ();
#     fn seal_activation<'scope>(
#         self, sealer: CanonicalActivationPlanSealer<'scope>,
#     ) -> CanonicalSealedActivationPlan<'scope, ()> {
#         sealer.seal(self.request, self.success, ())
#     }
# }
# fn body_interruption_slot<'slot>() -> CanonicalCauseSlot<'slot, CanonicalFreezeInterruption> {
#     panic!("a body has no final interruption slot")
# }
let factory = PhysicalStorageAccountFactory::try_new(
    PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
).unwrap();
let account = factory.try_create_account(
    PhysicalStorageAccountLimits::new(u64::MAX, 256).unwrap(),
).unwrap();
let catalog = StoreSemanticCatalogId::new(*b"baozi-doc-store!", NonZeroU32::MIN);
let domain = StoreKeyDomain::try_new(catalog).unwrap();
let phase = domain.try_admit_phase(
    StorePhaseSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let ordinary = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN),
).unwrap();
let protected = domain.try_admit_dimension(
    StoreDimensionSemanticId::new(catalog, NonZeroU64::new(2).unwrap()),
).unwrap();
let failure_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
    protected, 1, 64, 64, u64::MAX, u64::MAX,
).unwrap();
let policy = DimensionPolicy::reservable_bytes(
    failure_work.ordinary_attempted_work_limit(),
    failure_work.ordinary_consumed_work_limit(), u64::MAX, u64::MAX,
).with_published_outcome_evidence_limit(u64::MAX);
let limits = LedgerLimits::new(u64::MAX, [
    (ordinary, policy), (protected, failure_work.requirement().dimension_policy()),
]).unwrap();
let ledger = StoreLedgerPlan::try_new(domain, limits, &[phase]).unwrap();
let start = RetainedStoreStartPlan::new(
    RetainedStoreWriterPlan::new(
        phase, ordinary, ordinary, RetainedStoreBatchLimits::new(32, 8).unwrap(),
    ),
    RetainedStoreSharePlan::new(ordinary, ordinary), StoreFallbackEnvelope::new(91),
    ordinary, 1, ordinary, 1, failure_work,
);
type Host = CanonicalInvocationHost<
    Owner, Phase, Unit, u8, u8, (), (), (),
    ScalarSequence<u8>, CanonicalTag, ScalarSequence<u8>, EvidenceTag,
>;
let mut host = Host::new();
let mut startup = Some((ledger, start));
let finish = host.run_body(|parts, disposition| {
    let (ledger, start) = startup.take().unwrap();
    let operation = CanonicalBodyOperation::try_start(
        parts.startup, OperationControl::default(), ledger, &account, start,
    ).unwrap();
    let (identity, request) = mint_canonical_activation_request().unwrap();
    let failure = CanonicalFailureRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(),
    ).with_request_identity(identity);
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(1, 1).unwrap(), 2, 1, NonZeroU64::new(2).unwrap(),
    ).unwrap().with_request_identity(identity);
    let role = RetainedStoreWriterRole::new(phase, ordinary, ordinary);
    let policy = CanonicalActivationSessionPolicy::new(
        role, role, CanonicalMaterializationRole::new(ordinary, ordinary),
        CanonicalValidationRole::new(phase, ordinary, ordinary), ordinary, ordinary,
    );
    let owner = operation.into_session_outcome_owner(
        policy, RetainedStoreBatchLimits::new(2, 2).unwrap(), failure,
    ).unwrap();
    let mut description = Some(Description { request, success });
    let activation = owner.plan_and_activate_scoped(
        parts.output, parts.activation, |mut planning| {
            let ((), completion) = planning.with_semantic_work(|semantic| {
                semantic.run_unit(|| ()).unwrap();
                Ok::<(), ()>(())
            }).unwrap();
            match planning.seal_activation(description.take().unwrap(), completion) {
                CanonicalActivationSeal::Sealed(permit) => permit.activate(|mut lane, _| {
                    let root = lane.try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(), CanonicalTag,
                        |writer| writer.push(7),
                    ).unwrap();
                    lane.complete::<_, (), _, _>(root, 7_u8)
                }),
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        },
    ).unwrap();
    let CanonicalActivationOutcome::Activated(activated) = activation else {
        panic!("the sealed description must activate its canonical root");
    };
    let validation = activated.validate_centrally(
        parts.proof, parts.validation, CanonicalValidationProgress::new(Phase::Build, Unit::Items),
        |lane, output| {
            assert_eq!(*output, 7);
            let root = lane.try_retain_sequence(
                SequencePlan::<u8>::new(1).unwrap(), EvidenceTag, |writer| writer.push(9),
            ).unwrap();
            lane.complete(root)
        },
        |planning, output| {
            assert_eq!(*output, 7);
            planning.validate_with(|_| Ok(()), |session, canonical, evidence, _| {
                assert!(session.visit_canonical_sequence(canonical, 1, |_, value| value == 7)?);
                assert!(session.visit_outcome_evidence_sequence(evidence, 1, |_, value| value == 9)?);
                Ok(CanonicalValidationDecision::<u8, ()>::accept(9))
            })
        },
    ).unwrap();
    let CanonicalCentralValidationOutcome::Candidate(candidate) = validation else {
        panic!("both exact roots must validate before a candidate exists");
    };
    assert_eq!((*candidate.output(), *candidate.proof()), (7, 9));
    if false {
        let _ = candidate.try_issue_freeze_permit(
            body_interruption_slot(), FinalProgress::new(Phase::Build, Unit::Items, 2),
        );
        return;
    }
    candidate.request_finish(disposition).unwrap();
}).unwrap();
let finish = finish.admit_terminal_catalog(
    &[phase.semantic_id()], &[ordinary.semantic_id(), protected.semantic_id()],
).unwrap();
let (candidate, generation) = {
    let completion = finish.resume();
    let candidate = match completion.outcome {
        CanonicalInvocationOutcome::Candidate(candidate) => candidate,
        _ => panic!("resume must restore the actual validated candidate"),
    };
    (candidate, completion.generation)
};
assert_eq!((*candidate.output(), *candidate.proof()), (7, 9));
let permit = {
    let issue = candidate.try_issue_freeze_permit(
        generation.interruption, FinalProgress::new(Phase::Build, Unit::Items, 2),
    ).unwrap();
    match issue {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        _ => panic!("the final candidate must remain admissible"),
    }
};
let CanonicalJointSuccess { output, proof, store } =
    permit.bind_joint_commit(|joint, seal| joint.prepare(seal).unwrap());
assert_eq!((output, proof), (7, 9));
let published = store.handoff();
assert_eq!(published.canonical.root().get(0), Some(7));
assert_eq!(published.outcome_evidence.root().get(0), Some(9));
assert!(matches!(published.ledger_evidence.header().terminal_cause(), StoreTerminalCause::Success));
assert_eq!(published.ledger_evidence.dimension(ordinary).unwrap().committed_canonical_output(), 1);
drop(published);
drop(host);
assert_eq!(account.snapshot().live_retained_bytes(), 0);
assert_eq!(account.snapshot().live_retained_allocations(), 0);
assert!(!account.snapshot().poisoned());
```
