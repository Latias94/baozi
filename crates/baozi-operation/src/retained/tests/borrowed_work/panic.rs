use super::*;
use std::cell::Cell;

fn assert_canonical_panic_containment(
    assembly: &TestPublication<'_, u64, u64>,
    result: Result<
        CanonicalRetainedSlot<u64, CanonicalRootTag>,
        RetainedPublicationError<TestDimension>,
    >,
    storage_before: RetainedPlanningBaseline,
    physical_before: PhysicalStorageAccountSnapshot,
    work_before: DimensionSnapshot,
    reservations_before: u64,
) {
    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(RetainedWriterError::Physical(cause)))
            if cause == RetainedAllocationError::BuilderPanicked && cause.charge_retained()
    ));
    let operation = assembly.loan().operation.as_ref().unwrap();
    assert!(!operation.has_outstanding_work_permit());
    assert!(!operation.planning_matches_baseline(storage_before));
    let work = assembly
        .loan()
        .test_dimension_evidence(TestDimension::Bytes)
        .unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted() + 5, work_before.consumed() + 3)
    );
    let account = assembly.loan().fields.account.as_ref().unwrap();
    let charge = account
        .checked_retained_charge::<u64>(empty_plan())
        .unwrap();
    assert_eq!(
        work.current_live_bytes(),
        work_before.current_live_bytes() + charge.bytes
    );
    assert_eq!(
        assembly.loan().test_ledger_probe().active_reservations(),
        reservations_before + 1
    );
    assert_eq!(assembly.loan().canonical_len(), 0);
    assert_eq!(assembly.loan().canonical_new_blocks(), 0);
    assert_eq!(
        assembly
            .loan()
            .fields
            .canonical_batch
            .as_ref()
            .unwrap()
            .attribution_len(),
        1
    );
    assert!(!assembly.loan().test_storage_graph_is_exact());
    let physical = account.snapshot();
    assert!(physical.poisoned());
    assert_eq!(
        physical
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert_eq!(
        physical.live_retained_bytes(),
        physical_before.live_retained_bytes() + charge.bytes
    );
    assert_eq!(
        physical.live_retained_allocations(),
        physical_before.live_retained_allocations() + charge.allocations
    );
}

#[test]
fn controlled_canonical_body_panic_preserves_prefix_and_retains_charge() {
    for borrowed in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
        let storage_before = assembly
            .loan()
            .operation
            .as_ref()
            .unwrap()
            .planning_baseline();
        let physical_before = account.snapshot();
        let work_before = assembly
            .loan()
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let reservations_before = assembly.loan().test_ledger_probe().active_reservations();
        let shares_before = account.share_call_count();

        let result = assembly
            .loan_mut()
            .try_retain_canonical_controlled::<CanonicalRootTag>(
                TestDimension::Bytes,
                empty_plan(),
                |_emplacer, mut control| {
                    if borrowed {
                        let mut permission = control
                            .try_begin_borrowed(NonZeroU64::new(3).unwrap())
                            .unwrap_or_else(|_| panic!("borrowed panic envelope must be admitted"));
                        assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                        assert_eq!(permission.remaining_units(), 2);
                        panic!("controlled builder body panicked with a borrowed permission");
                    } else {
                        let mut permission = control
                            .try_begin(NonZeroU64::new(3).unwrap())
                            .unwrap_or_else(|_| panic!("panic envelope must be admitted"));
                        assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                        assert_eq!(permission.remaining_units(), 2);
                        panic!("controlled builder body panicked with an owning permission");
                    }
                },
            );

        assert_eq!(account.share_call_count(), shares_before + 1);
        assert_canonical_panic_containment(
            &assembly,
            result,
            storage_before,
            physical_before,
            work_before,
            reservations_before,
        );
        drop(assembly);
        drop(root);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}

struct PanickingCallbackCapture<'a>(&'a Cell<bool>);

impl Drop for PanickingCallbackCapture<'_> {
    fn drop(&mut self) {
        self.0.set(true);
        if !std::thread::panicking() {
            panic!("controlled callback capture destructor panicked");
        }
    }
}

#[test]
fn controlled_canonical_capture_drop_panic_resolves_forgotten_permission_at_host() {
    for borrowed in [false, true] {
        let factory = account_factory(1);
        let account = account(&factory, u64::MAX, 128);
        let mut root = TestInvocationRoot::new();
        let mut assembly = ordinary_assembly(&mut root, &account, OperationControl::default());
        let storage_before = assembly
            .loan()
            .operation
            .as_ref()
            .unwrap()
            .planning_baseline();
        let physical_before = account.snapshot();
        let work_before = assembly
            .loan()
            .test_dimension_evidence(TestDimension::Bytes)
            .unwrap();
        let reservations_before = assembly.loan().test_ledger_probe().active_reservations();
        let shares_before = account.share_call_count();
        let capture_dropped = Cell::new(false);
        let builder_returned_ok = Cell::new(false);
        let returned_ok = &builder_returned_ok;
        let capture = PanickingCallbackCapture(&capture_dropped);

        let result = assembly.loan_mut().try_retain_canonical_controlled(
            TestDimension::Bytes,
            empty_plan(),
            move |emplacer, mut control| {
                let _capture = capture;
                if borrowed {
                    let mut permission = control
                        .try_begin_borrowed(NonZeroU64::new(3).unwrap())
                        .unwrap_or_else(|_| panic!("borrowed capture envelope must be admitted"));
                    assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                    std::mem::forget(permission);
                } else {
                    let mut permission = control
                        .try_begin(NonZeroU64::new(3).unwrap())
                        .unwrap_or_else(|_| panic!("capture envelope must be admitted"));
                    assert!(permission.complete_units(NonZeroU64::MIN).is_ok());
                    std::mem::forget(permission);
                }
                returned_ok.set(true);
                Ok(emplacer.place(11, CanonicalRootTag))
            },
        );

        assert!(builder_returned_ok.get());
        assert!(capture_dropped.get());
        assert_eq!(account.share_call_count(), shares_before + 1);
        assert_canonical_panic_containment(
            &assembly,
            result,
            storage_before,
            physical_before,
            work_before,
            reservations_before,
        );
        drop(assembly);
        drop(root);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 1);
    }
}
