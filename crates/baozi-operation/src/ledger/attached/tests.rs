use super::*;

std::thread_local! {
    static PANIC_AFTER_CANONICAL_DETACH: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static PANIC_AFTER_COMMITMENTS: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

pub(super) fn arm_partial_detach_panic() {
    PANIC_AFTER_CANONICAL_DETACH.with(|pending| pending.set(true));
}

pub(super) fn panic_after_canonical_detach() {
    if PANIC_AFTER_CANONICAL_DETACH.with(|pending| pending.replace(false)) {
        panic!("injected unwind after the actual canonical batch detached");
    }
}

pub(super) fn arm_commit_panic() {
    PANIC_AFTER_COMMITMENTS.with(|pending| pending.set(true));
}

pub(super) fn panic_after_commitments() {
    if PANIC_AFTER_COMMITMENTS.with(|pending| pending.replace(false)) {
        panic!("injected unwind after actual terminal commitments were applied");
    }
}

struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Finish,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "finish"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "work"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
}

struct TerminalRoot {
    invocation: crate::retained::InvocationRoot<Owner, Phase, Phase, Unit, Dimension, (), (), ()>,
    baseline: Option<FallbackBatchBaseline<Owner, Phase, Dimension>>,
}

fn ledger() -> Ledger<Owner, Phase, Dimension> {
    let limits = LedgerLimits::new(
        u64::MAX,
        [(
            Dimension::Bytes,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )],
    )
    .expect("terminal test limits");
    Ledger::try_new(limits, &[Phase::Finish]).expect("complete terminal test catalog")
}

impl TerminalRoot {
    fn with_probe() -> (Self, LedgerTestProbe<Phase, Dimension>) {
        let mut ledger = ledger();
        ledger
            .view()
            .try_complete_phase(Phase::Finish)
            .expect("the terminal phase completes before publication");
        let canonical = ledger
            .try_attribution_batch::<CanonicalOutput>(Phase::Finish, Dimension::Bytes, 1, 0)
            .expect("canonical assembly with actual backing");
        let evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Finish,
                Dimension::Bytes,
                1,
                0,
            )
            .expect("evidence assembly with actual backing");
        let fallback = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Finish,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("empty fallback source");
        let numerical_evidence = ledger
            .try_attribution_batch::<OutcomeEvidenceDestination>(
                Phase::Finish,
                Dimension::Bytes,
                0,
                0,
            )
            .expect("empty numerical evidence source");
        let mut baseline = ledger
            .try_fallback_baseline(canonical, evidence, fallback, numerical_evidence)
            .unwrap_or_else(|_| panic!("the actual baseline must bind"));
        assert!(baseline.try_prepare_fallback_sources().is_ok());
        ledger
            .seal_ordinary_work()
            .expect("ordinary work is resolved");
        ledger
            .close_work()
            .expect("work closes before qualification");
        let probe = ledger.test_probe();
        let mut invocation = crate::retained::InvocationRoot::new();
        invocation.install_ledger_for_test(ledger);
        (
            Self {
                invocation,
                baseline: Some(baseline),
            },
            probe,
        )
    }

    fn ready(&mut self) -> FreezeReadyTypedProof<'_, Owner, Phase, Phase, Unit, Dimension> {
        let proof = self
            .baseline
            .as_mut()
            .expect("the root owns its actual baseline")
            .view()
            .finish_prepared_fallback_proof(self.invocation.terminal_loan_for_test(), None, None);
        match proof.try_into_qualified_discards() {
            Ok(QualifiedTypedDiscardFreezeProof::None(ready)) => ready,
            Ok(proof) => {
                proof.retain_fail_closed();
                panic!("this baseline has no discarded physical fields")
            }
            Err(proof) => {
                proof.retain_fail_closed();
                panic!("the exact baseline must qualify")
            }
        }
    }

    fn batches(&mut self) -> TypedTerminalBatches<'_, Owner, Phase, Dimension> {
        let baseline = self
            .baseline
            .as_mut()
            .expect("the root owns its actual baseline");
        TypedTerminalBatches {
            canonical: &mut baseline.canonical_assembly,
            evidence: &mut baseline.evidence_assembly,
            deferred_canonical_storage: None,
            deferred_evidence_storage_first: Some(&mut baseline.evidence),
            deferred_evidence_storage_second: Some(&mut baseline.fallback),
            discard_canonical: None,
            discard_evidence: None,
        }
    }
}

impl Drop for TerminalRoot {
    fn drop(&mut self) {
        self.invocation.cleanup_owned_state();
        if self.invocation.must_retain_domain() {
            std::mem::forget(self.baseline.take());
        }
    }
}

fn live_state(probe: &LedgerTestProbe<Phase, Dimension>) -> (u64, u64, u64) {
    (
        probe.current_live_bytes(),
        probe.active_reservations(),
        probe.current_administrative_bytes(),
    )
}

#[test]
fn mid_commit_unwind_keeps_the_actual_dynamic_cover_installed_before_proof_drop() {
    let (mut root, _) = TerminalRoot::with_probe();
    let ready = root.ready();
    let last_completed_phase = ready.last_completed_phase();
    assert!(ready.seal_to_root().is_ok());
    let invocation = root.invocation.take_terminal_invocation_for_test();
    let mut proof = invocation
        .try_detach_typed(last_completed_phase, root.batches())
        .unwrap_or_else(|_| panic!("the real dynamic terminal cover detaches"));
    let before = {
        let cover = proof.cover.as_ref().unwrap();
        (
            cover.state.dimensions.as_ptr(),
            cover.state.phases.as_ptr(),
            cover.state.phase_states.as_ptr(),
            cover.state.current_live_bytes,
            cover.state.active_reservations,
            cover.state.current_administrative_bytes,
        )
    };
    assert!(before.3 > 0 && before.4 > 0 && before.5 > 0);
    arm_commit_panic();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| proof.commit_inner()));
    assert!(result.is_err());
    assert!(proof.projection.is_some());
    let retained = proof
        .cover
        .as_ref()
        .expect("an interrupted commit keeps its actual cover");
    assert_eq!(
        (
            retained.state.dimensions.as_ptr(),
            retained.state.phases.as_ptr(),
            retained.state.phase_states.as_ptr(),
            retained.state.current_live_bytes,
            retained.state.active_reservations,
            retained.state.current_administrative_bytes,
        ),
        before,
    );
    drop(proof);
}

#[test]
fn unresolved_shared_recovery_drop_retains_original_storage_while_a_token_is_live() {
    let mut owner = ledger();
    let token = owner
        .reserve(Phase::Finish, Dimension::Bytes, 37)
        .expect("a real escaping token promotes the original storage");
    let probe = owner.test_probe();
    let before = live_state(&probe);
    assert_eq!((before.0, before.1), (37, 1));
    let shared = match std::mem::replace(&mut owner.state, DynamicStorage::Inline(None)) {
        DynamicStorage::Shared(shared) => shared,
        DynamicStorage::Inline(_) => panic!("the live token retains shared storage"),
    };
    let identity = Arc::downgrade(&shared);
    let original_owners = Arc::strong_count(&shared);
    let mut recovery = SharedStorageRecovery::new(shared);
    assert!(matches!(
        recovery.recover(),
        Err(SharedStorageRecoveryError::SharedOwner)
    ));
    assert_eq!(live_state(&probe), before);
    drop(recovery);

    let retained = identity
        .upgrade()
        .expect("abandoning recovery retains its original shell");
    assert_eq!(Arc::strong_count(&retained), original_owners + 1);
    assert_eq!(live_state(&probe), before);
    drop(token);
    assert_eq!(
        (probe.current_live_bytes(), probe.active_reservations()),
        (0, 0)
    );
    assert_eq!(probe.current_administrative_bytes(), before.2);
}

#[test]
fn poisoned_exclusive_recovery_preserves_actual_rows_and_retires_only_the_mutex_cover() {
    let mut owner = ledger();
    let token = owner
        .reserve(Phase::Finish, Dimension::Bytes, 53)
        .expect("the original owner admits a real token");
    drop(token);
    let dimension_before = owner.dimension_evidence(Dimension::Bytes).unwrap();
    let shared = match std::mem::replace(&mut owner.state, DynamicStorage::Inline(None)) {
        DynamicStorage::Shared(shared) => shared,
        DynamicStorage::Inline(_) => panic!("the original storage was promoted"),
    };
    let (administrative_before, dimensions, phases, dimension_staging, phase_staging) = {
        let selected = shared.lock().unwrap();
        let state = selected.as_ref().unwrap();
        (
            state.current_administrative_bytes,
            state.dimensions.as_ptr(),
            state.phase_states.as_ptr(),
            state.dimension_snapshot_staging.as_ptr(),
            state.phase_snapshot_staging.as_ptr(),
        )
    };
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _locked = shared.lock().unwrap();
        panic!("poison the actual mutex before exclusive recovery");
    }));
    assert!(poisoned.is_err() && shared.is_poisoned());
    let mut recovery = SharedStorageRecovery::new(shared);
    assert!(matches!(
        recovery.recover(),
        Err(SharedStorageRecoveryError::Poisoned)
    ));
    let state = recovery.state_mut();
    assert_eq!(
        (state.current_live_bytes, state.active_reservations),
        (0, 0)
    );
    assert_eq!(state.dimensions.as_ptr(), dimensions);
    assert_eq!(state.phase_states.as_ptr(), phases);
    assert_eq!(state.dimension_snapshot_staging.as_ptr(), dimension_staging);
    assert_eq!(state.phase_snapshot_staging.as_ptr(), phase_staging);
    assert_eq!(
        state.current_administrative_bytes,
        administrative_before - size_of::<Mutex<Option<LedgerState<Phase, Dimension>>>>() as u64,
    );
    owner.state = DynamicStorage::Inline(recovery.take_state());
    drop(recovery);
    assert_eq!(
        owner.dimension_evidence(Dimension::Bytes),
        Ok(dimension_before)
    );
}

#[test]
fn capture_panic_retains_the_actual_invocation_and_original_staging_backings() {
    let (mut root, probe) = TerminalRoot::with_probe();
    let ready = root.ready();
    let last_completed_phase = ready.last_completed_phase();
    assert!(ready.seal_to_root().is_ok());
    let before = live_state(&probe);
    assert!(before.0 > 0 && before.1 > 0 && before.2 > 0);
    let staging = {
        let storage = probe
            .state
            .upgrade()
            .expect("the actual owner is installed");
        let mut state = lock(&storage);
        let staging = (
            state.dimension_snapshot_staging.as_ptr(),
            state.dimension_snapshot_staging.capacity(),
            state.phase_snapshot_staging.as_ptr(),
            state.phase_snapshot_staging.capacity(),
        );
        // Corrupt only a previously qualified positioned fact. The capture kernel must panic
        // before moving its pre-admitted staging, under the actual invocation's guard.
        state.phases[0].state_position = usize::MAX;
        staging
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let invocation = root.invocation.take_terminal_invocation_for_test();
        let _ = invocation.try_detach_typed(last_completed_phase, root.batches());
    }));
    assert!(result.is_err());
    assert!(probe.is_available());
    assert_eq!(live_state(&probe), before);
    {
        let storage = probe.state.upgrade().expect("the actual owner is retained");
        let state = lock(&storage);
        assert_eq!(
            (
                state.dimension_snapshot_staging.as_ptr(),
                state.dimension_snapshot_staging.capacity(),
                state.phase_snapshot_staging.as_ptr(),
                state.phase_snapshot_staging.capacity(),
            ),
            staging
        );
    }
    drop(root);
    assert!(probe.is_available());
    assert_eq!(live_state(&probe), before);
}

#[test]
fn partial_detach_panic_preserves_live_cover_and_retires_only_completed_source_shells() {
    let (mut root, probe) = TerminalRoot::with_probe();
    let ready = root.ready();
    let last_completed_phase = ready.last_completed_phase();
    assert!(ready.seal_to_root().is_ok());
    let before = live_state(&probe);
    assert!(before.0 > 0 && before.1 > 0 && before.2 > 0);
    PANIC_AFTER_CANONICAL_DETACH.with(|pending| pending.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let invocation = root.invocation.take_terminal_invocation_for_test();
        let _ = invocation.try_detach_typed(last_completed_phase, root.batches());
    }));
    PANIC_AFTER_CANONICAL_DETACH.with(|pending| pending.set(false));
    assert!(result.is_err());
    assert!(probe.is_available());
    assert_eq!(live_state(&probe), before);
    let baseline = root
        .baseline
        .as_ref()
        .expect("the source shells stay in the root");
    assert_eq!(baseline.canonical_assembly.attributions.capacity(), 0);
    assert!(baseline.canonical_assembly.capacity_cover.is_none());
    assert!(baseline.evidence_assembly.attributions.capacity() > 0);
    assert!(baseline.evidence_assembly.capacity_cover.is_some());
    {
        let storage = probe.state.upgrade().expect("the actual owner is retained");
        let state = lock(&storage);
        // Capture moved the original staging before the second detach panicked. Its admission
        // and the completed batch's live cover must remain visible throughout root cleanup.
        assert_eq!(state.dimension_snapshot_staging.capacity(), 0);
        assert_eq!(state.phase_snapshot_staging.capacity(), 0);
    }
    drop(root);
    assert!(probe.is_available());
    assert_eq!(live_state(&probe), before);
}
