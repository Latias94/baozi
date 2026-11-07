use super::*;
use crate::ledger::finite::{FiniteDetachedTypedFreezeProof, FiniteTypedFreezeDetachError};
use crate::ledger::{
    AttributionBatchPlan, AttributionReservationBatch, CanonicalOutput,
    FiniteFallbackBatchBaseline, QualifiedTypedDiscardFreezeProof, TypedFreezeDetachErrorKind,
    TypedTerminalBatches,
};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Phase;
impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "finite-detach"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

type Invocation = FiniteOperationInvocation<Owner, u8, Phase, Unit, u8>;
type Baseline = FiniteFallbackBatchBaseline<Owner, u8, u8>;
type Proof = FiniteDetachedTypedFreezeProof<Owner, u8, u8>;
type Failure = FiniteTypedFreezeDetachError<Owner, u8, Phase, Unit, u8>;

struct Root {
    common: Option<Invocation>,
    baseline: Option<Baseline>,
    contained: bool,
}

impl Root {
    fn new(attribution_slots: usize) -> Self {
        let mut common = FiniteOperationInvocation::try_new(
            OperationControl::default(),
            FiniteLedgerLimits::try_new(
                u64::MAX,
                u64::MAX,
                [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
                    (
                        dimension,
                        DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                            .with_published_outcome_evidence_limit(u64::MAX),
                    )
                }),
            )
            .unwrap(),
            [3_u8, 1, 2],
            CanonicalLineageAuthority::fresh(),
        )
        .unwrap();
        let raw = common.terminal_ledger_mut();
        raw.view().charge_attempted(1, 3).unwrap();
        raw.view().charge_consumed(1, 2).unwrap();
        raw.view().try_complete_phase(1).unwrap();
        let canonical: AttributionReservationBatch<_, _, _, CanonicalOutput> = raw
            .view()
            .try_attribution_batch_from_plan(
                AttributionBatchPlan::try_new(1, 1, attribution_slots, 0).unwrap(),
            )
            .unwrap();
        let evidence = raw
            .view()
            .try_attribution_batch_from_plan(
                AttributionBatchPlan::try_new(1, 1, attribution_slots, 0).unwrap(),
            )
            .unwrap();
        let fallback = raw
            .view()
            .try_attribution_batch_from_plan(AttributionBatchPlan::try_new(1, 1, 0, 0).unwrap())
            .unwrap();
        let mut baseline = Baseline::new_unbound(canonical, evidence, fallback);
        raw.as_read()
            .try_attach_fallback_rows(baseline.view())
            .unwrap();
        assert_eq!(common.view().close_terminal_owners(), None);
        Self {
            common: Some(common),
            baseline: Some(baseline),
            contained: false,
        }
    }

    fn probe(&self) -> Option<crate::ledger::LedgerTestProbe<u8, u8>> {
        self.common
            .as_ref()
            .unwrap()
            .coordinator
            .retained_ledger()
            .test_shared_probe()
    }

    fn seal(&mut self) -> AttachedTerminalStamp {
        let raw = self.common.as_ref().unwrap().coordinator.retained_ledger();
        let baseline = self.baseline.as_mut().unwrap();
        assert!(baseline.as_read().numerical_evidence.is_none());
        raw.as_read()
            .try_bind_fallback_discard_rows(baseline.view(), None, None)
            .unwrap();
        assert!(!self.common.as_ref().unwrap().lifecycle.terminal_outstanding);
        let proof = baseline.view().finish_prepared_fallback_proof(
            InvocationSlot::Finite(&mut self.common).into_terminal(&mut self.contained),
            None,
            None,
        );
        let ready = match proof.try_into_qualified_discards() {
            Ok(QualifiedTypedDiscardFreezeProof::None(ready)) => ready,
            Ok(proof) => {
                proof.retain_fail_closed();
                panic!("the actual no-discard baseline must become Ready");
            }
            Err(proof) => {
                proof.retain_fail_closed();
                panic!("the actual finite work/storage/batch graph must qualify");
            }
        };
        ready
            .seal_to_root()
            .unwrap_or_else(|_| panic!("the actual finite root records Ready"))
    }

    fn detach(&mut self, stamp: AttachedTerminalStamp) -> Result<Proof, Failure> {
        let common = self.common.as_mut().unwrap();
        let identity = common.coordinator.retained_ledger().as_read().identity();
        let decision = common
            .lifecycle
            .take_attached_decision(stamp, identity)
            .expect("the concrete root consumes its original Ready decision once");
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            decision.layout,
            crate::ledger::TypedTerminalLayout::Fallback
        );
        assert!(!decision.roles.deferred_canonical && !decision.roles.deferred_evidence_first);
        assert!(decision.roles.deferred_evidence_second);
        self.contained = true;
        let common = self.common.take().unwrap();
        let baseline = self.baseline.as_mut().unwrap().view();
        common.try_detach_typed(
            decision.last_completed_phase,
            TypedTerminalBatches {
                canonical: baseline.canonical_assembly,
                evidence: baseline.evidence_assembly,
                deferred_canonical_storage: None,
                deferred_evidence_storage_first: None,
                deferred_evidence_storage_second: Some(baseline.fallback),
                discard_canonical: None,
                discard_evidence: None,
            },
        )
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if self.contained
            || self
                .common
                .as_ref()
                .is_some_and(|common| common.lifecycle.terminal_outstanding)
        {
            std::mem::forget(self.baseline.take());
            std::mem::forget(self.common.take());
        }
    }
}

#[test]
fn actual_finite_ready_detaches_inline_and_promoted_without_dynamic_projection() {
    for slots in [0, 1] {
        let mut root = Root::new(slots);
        let probe = root.probe();
        assert_eq!(probe.is_some(), slots != 0);
        let known = root
            .common
            .as_ref()
            .unwrap()
            .known_administrative_bytes()
            .unwrap();
        let stamp = root.seal();
        let mut proof = root
            .detach(stamp)
            .unwrap_or_else(|_| panic!("the original finite graph detaches"));
        assert!(root.common.is_none());
        let projection = proof.terminal_projection();
        assert_eq!(projection.dimensions().len(), 6);
        assert_eq!(projection.phases().len(), 3);
        assert_eq!(projection.last_completed_phase(), Some(1));
        let dimension = projection.dimension(1).unwrap();
        assert_eq!((dimension.attempted(), dimension.consumed()), (3, 2));
        assert_eq!(
            (
                projection.current_live_bytes(),
                projection.active_reservations()
            ),
            (0, 0)
        );
        assert_eq!(projection.current_administrative_bytes(), known);
        assert_eq!(projection.covered_administrative_bytes(), Some(known));
        assert!(probe.as_ref().is_none_or(|probe| !probe.is_available()));
        let (facts, terminal) = proof.commit_account_bound_for_test();
        assert_eq!(facts.last_completed_phase(), Some(1));
        assert_eq!(terminal.current_live_bytes, 0);
        assert_eq!(terminal.active_reservations, 0);
        assert_eq!(terminal.current_administrative_bytes, 0);
        assert!(terminal.published);
    }
}

#[test]
fn finite_shared_owner_refusal_transfers_original_invocation_and_all_detached_batches() {
    let mut root = Root::new(1);
    let probe = root.probe().unwrap();
    let stamp = root.seal();
    let identity = root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .as_read()
        .identity();
    let shared = root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .test_shared_owner()
        .expect("the test keeps one additional actual owner");
    let before = (
        probe.current_live_bytes(),
        probe.active_reservations(),
        probe.current_administrative_bytes(),
    );
    let failure = root
        .detach(stamp)
        .err()
        .expect("the original extra owner refuses exclusive detach");
    assert_eq!(
        failure.kind(),
        TypedFreezeDetachErrorKind::SharedLedgerOwner
    );
    assert_eq!(
        failure
            .test_invocation()
            .coordinator
            .retained_ledger()
            .as_read()
            .identity(),
        identity
    );
    let retained = failure
        .test_invocation()
        .coordinator
        .retained_ledger()
        .test_shared_owner()
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&shared, &retained));
    drop(retained);
    assert_eq!(
        (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes()
        ),
        before
    );
    drop(root);
    drop(failure);
    drop(shared);
    assert!(probe.is_available());
    assert_eq!(
        (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes()
        ),
        before
    );
}

#[test]
fn finite_mid_commit_unwind_keeps_inline_and_promoted_cover_in_the_original_proof() {
    for slots in [0, 1] {
        let mut root = Root::new(slots);
        let stamp = root.seal();
        let mut proof = root
            .detach(stamp)
            .unwrap_or_else(|_| panic!("the original finite cover detaches"));
        let before = proof.test_cover_observation().unwrap();
        let facts = *proof.terminal_projection();
        Proof::test_arm_commit_panic();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            proof.commit_account_bound_for_test()
        }));
        assert!(result.is_err());
        assert_eq!(proof.test_cover_observation(), Some(before));
        assert_eq!(proof.terminal_projection(), &facts);
        drop(proof);
    }
}

#[test]
fn finite_capture_and_partial_detach_panics_retain_actual_promoted_cover() {
    for partial_detach in [false, true] {
        let mut root = Root::new(1);
        let probe = root.probe().unwrap();
        let stamp = root.seal();
        let before = (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes(),
        );
        if partial_detach {
            root.common
                .as_mut()
                .unwrap()
                .test_panic_after_next_canonical_detach();
        } else {
            root.common
                .as_mut()
                .unwrap()
                .terminal_ledger_mut()
                .test_corrupt_storage_phase_position();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.detach(stamp)));
        assert!(result.is_err());
        assert!(root.common.is_none());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            before
        );
        let baseline = root.baseline.as_ref().unwrap().as_read();
        assert_eq!(
            baseline.canonical_assembly.account_capacity_bytes() == 0,
            partial_detach
        );
        assert!(baseline.evidence_assembly.account_capacity_bytes() > 0);
        drop(root);
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            before
        );
    }
}
