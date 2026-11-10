use super::*;
use crate::control::{CancellationOrigin, CancellationToken};
use crate::ledger::WorkLane;
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
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
        "finite-invocation"
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

fn invocation(control: OperationControl<Phase, Unit>) -> Invocation {
    FiniteOperationInvocation::try_new(
        control,
        FiniteLedgerLimits::try_new(
            64,
            0,
            [1_u8, 2, 3, 4, 5, 6]
                .map(|dimension| (dimension, DimensionPolicy::reservable_bytes(32, 32, 0, 64))),
        )
        .unwrap(),
        [3, 1, 2],
        CanonicalLineageAuthority::fresh(),
    )
    .unwrap()
}

fn account() -> PhysicalStorageAccount {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap()
    .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
    .unwrap()
}

#[test]
fn finite_invocation_lends_actual_work_progress_and_lifecycle_debt() {
    let mut invocation = invocation(OperationControl::default());
    let account = account();
    invocation.lifecycle.control_action_debt.set(7);
    invocation
        .preflight_exclusive()
        .unwrap_or_else(|_| panic!("fresh actual invocation is ready"));
    {
        let mut parts = invocation
            .view()
            .into_exclusive_parts(&account, RetainedHandleSharePlan::new(1, 1))
            .unwrap_or_else(|_| panic!("one short loan of the actual finite owner"));
        let layout = parts.work.bind([1]).unwrap();
        parts.work.charge_attempted(layout, [3]).unwrap();
        parts.work.complete_observed(layout, [2]).unwrap();
        parts
            .ordinary
            .before_action(1)
            .unwrap_or_else(|_| panic!("debt admits one action"));
        parts
            .ordinary
            .completed_action(1)
            .unwrap_or_else(|_| panic!("actual debt advances"));
        parts
            .progress
            .report_progress(
                &mut parts.ordinary.poll,
                &mut parts.ordinary.storage,
                Phase,
                Unit,
                2,
                Some(3),
            )
            .unwrap();
    }
    assert_eq!(invocation.lifecycle.control_action_debt.get(), 8);
    assert_eq!(invocation.lifecycle.sticky_terminal_cause.get(), None);
    assert_eq!(
        invocation.lifecycle.execution_state.get(),
        StoreExecutionState::Running
    );
    let evidence = invocation
        .coordinator
        .retained_ledger()
        .as_read()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!((evidence.attempted(), evidence.consumed()), (3, 2));
    assert_eq!(
        invocation.coordinator.reported_progress(Phase, Unit),
        Some((2, Some(3)))
    );
    assert_eq!(invocation.coordinator.known_administrative_bytes(), Some(0));
    invocation
        .preflight_exclusive()
        .unwrap_or_else(|_| panic!("the same owner stays quiescent"));
}

#[test]
fn ordinary_finite_common_loan_keeps_original_slot_identity_when_moved_and_arms_that_same_root() {
    let mut common = Some(invocation(OperationControl::default()));
    let original_slot = std::ptr::from_ref(&common).cast();
    let original_lifecycle = std::ptr::from_ref(&common.as_ref().unwrap().lifecycle);
    let mut contained = false;
    let loan = InvocationLoan {
        common: InvocationSlot::Finite(&mut common),
        contained: &mut contained,
        not_send_or_sync: PhantomData,
    };
    let mut moved = loan;
    assert_eq!(moved.invocation_identity(), original_slot);
    assert_eq!(std::ptr::from_ref(moved.lifecycle()), original_lifecycle);
    moved
        .view()
        .coordinator
        .ledger_mut()
        .charge_attempted(1, 3)
        .unwrap();
    moved
        .view()
        .coordinator
        .ledger_mut()
        .charge_consumed(1, 2)
        .unwrap();
    moved
        .view()
        .coordinator
        .report_progress(Phase, Unit, 2, Some(3))
        .unwrap();
    assert_eq!(moved.recorded_control_cause(), None);
    moved
        .preflight_exclusive()
        .unwrap_or_else(|_| panic!("the same original finite root remains quiescent"));
    assert!(!moved.terminal_is_outstanding());
    assert_eq!(moved.view().close_terminal_owners(), None);
    let terminal = moved.into_terminal();
    assert!(terminal.is_finite());
    assert_eq!(terminal.common.invocation_identity(), original_slot);
    let work = terminal.as_read().dimension_evidence(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    terminal.retain_fail_closed();
    assert!(contained);
    assert!(common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert_eq!(std::ptr::from_ref(&common).cast::<()>(), original_slot);
    assert_eq!(
        std::ptr::from_ref(&common.as_ref().unwrap().lifecycle),
        original_lifecycle
    );
}

#[test]
fn cancelled_checkpoint_preserves_actual_completed_prefix_and_full_debt() {
    let cancellation = CancellationToken::new();
    let mut invocation =
        invocation(OperationControl::default().with_cancellation(cancellation.clone()));
    invocation
        .coordinator
        .retained_ledger_mut()
        .view()
        .charge_attempted(1, 3)
        .unwrap();
    invocation
        .coordinator
        .retained_ledger_mut()
        .view()
        .charge_consumed(1, 2)
        .unwrap();
    invocation
        .lifecycle
        .control_action_debt
        .set(u16::try_from(crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS).unwrap());
    cancellation.cancel();
    let account = account();
    let expected = ControlCause::Cancelled {
        origin: CancellationOrigin::Signal,
    };
    {
        let mut parts = invocation
            .view()
            .into_exclusive_parts(&account, RetainedHandleSharePlan::new(1, 1))
            .unwrap_or_else(|_| panic!("loan creation preserves the existing poll boundary"));
        assert!(
            matches!(parts.ordinary.before_action(1), Err(ExecutionError::Control(cause)) if cause == expected)
        );
    }
    assert_eq!(
        usize::from(invocation.lifecycle.control_action_debt.get()),
        crate::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS
    );
    assert_eq!(
        invocation.lifecycle.sticky_terminal_cause.get(),
        Some(PreparationFailureCode::from_control(&expected))
    );
    assert_eq!(
        invocation.coordinator.recorded_control_cause(),
        Some(expected.clone())
    );
    let evidence = invocation
        .coordinator
        .retained_ledger()
        .as_read()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!((evidence.attempted(), evidence.consumed()), (3, 2));
    assert!(
        matches!(invocation.preflight_exclusive(), Err(ExecutionError::Control(cause)) if cause == expected)
    );
}

#[test]
fn actual_zero_pending_open_frame_is_rejected_and_its_first_cause_survives_resolution() {
    let mut invocation = invocation(OperationControl::default());
    let (layout, mut frame) = {
        let (mut work, _) = invocation
            .coordinator
            .retained_ledger_mut()
            .view()
            .into_internal();
        let layout = work.bind_ordinary([1]).unwrap();
        let frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [0], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    assert!(matches!(
        invocation.preflight_exclusive(),
        Err(ExecutionError::Ledger(
            LedgerCause::OutstandingLinearWorkPermits { count: 1 }
        ))
    ));
    assert_eq!(
        invocation.lifecycle.sticky_terminal_cause.get(),
        Some(PreparationFailureCode::ledger())
    );
    invocation
        .coordinator
        .retained_ledger_mut()
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut frame, false)
        .unwrap();
    assert!(matches!(
        invocation.preflight_exclusive(),
        Err(ExecutionError::Stopped)
    ));
    assert_eq!(
        invocation.lifecycle.sticky_terminal_cause.get(),
        Some(PreparationFailureCode::ledger())
    );
    assert_eq!(invocation.coordinator.known_administrative_bytes(), Some(0));
}
