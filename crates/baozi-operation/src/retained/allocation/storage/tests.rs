use super::*;
use crate::CapacityPlan;
use crate::retained::allocation::test_retained_final_owner_allocator_entries;
use crate::retained::{
    AccountPoisonEvidence, AdmittedRetainedShareWork, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

const BUILD_FAILED: RetainedBuildFailureCode = RetainedBuildFailureCode::new(1);

#[test]
fn shared_typed_table_keeps_one_charge_and_one_backing_after_source_first_drop() {
    let account = account(16_384, 8);
    let (capacity, _) = byte_plan(127);
    let bytes = SharedRetainedStorage::<Vec<u8>>::header_bytes().unwrap()
        + capacity.envelope_delta_bytes().unwrap();
    let mut admission = account
        .try_admit_retained(
            AdmissionCharge::new(bytes, 2),
            AdmittedRetainedShareWork { private: () },
        )
        .unwrap();
    let lease = RetainedStorageLease::from_admission(&mut admission);
    let source = lease.try_bind_shared(try_bytes(capacity).unwrap()).unwrap();
    let derived =
        SharedRetainedStorage::from_allocation(source.allocation().share_after_test_admission());
    assert!(std::ptr::eq(source.as_ref(), derived.as_ref()));
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    drop(source);
    assert_eq!(derived.as_ref(), &vec![7; 127]);
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _derived = derived;
        panic!("ordinary host unwind after typed sharing");
    }));
    assert!(unwind.is_err());
    drop(unwind);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn empty_typed_table_charges_only_its_actual_shared_header() {
    let account = account(16_384, 8);
    let bytes = SharedRetainedStorage::<Vec<u8>>::header_bytes().unwrap();
    let mut admission = account
        .try_admit_retained(
            AdmissionCharge::new(bytes, 1),
            AdmittedRetainedShareWork { private: () },
        )
        .unwrap();
    let storage = RetainedStorageLease::from_admission(&mut admission)
        .try_bind_shared(Vec::<u8>::new())
        .unwrap();
    assert!(storage.as_ref().is_empty());
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    drop(storage);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}

fn account(bytes: u64, allocations: u64) -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(bytes, allocations).unwrap())
        .unwrap()
}

fn byte_plan(capacity: usize) -> (CapacityPlan, RetainedAllocationPlan) {
    let capacity = CapacityPlan::for_type::<u8>(0, 0, capacity).unwrap();
    let bytes = capacity.envelope_delta_bytes().unwrap();
    (
        capacity,
        RetainedAllocationPlan::new(bytes, u64::from(bytes != 0)).unwrap(),
    )
}

fn try_bytes(capacity: CapacityPlan) -> Result<Vec<u8>, RetainedBuildFailureCode> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity.allocator_additional())
        .map_err(|_| BUILD_FAILED)?;
    capacity
        .finish(bytes.len(), bytes.capacity())
        .map_err(|_| BUILD_FAILED)?;
    bytes.resize(capacity.desired_capacity(), 7);
    Ok(bytes)
}

struct ObserveDrop<'a> {
    bytes: Vec<u8>,
    account: &'a PhysicalStorageAccount,
    observed_bytes: &'a AtomicU64,
    drop_count: &'a AtomicU64,
    panic_on_drop: bool,
}

impl Drop for ObserveDrop<'_> {
    fn drop(&mut self) {
        assert!(!self.bytes.is_empty());
        self.observed_bytes.store(
            self.account.snapshot().live_retained_bytes(),
            Ordering::Relaxed,
        );
        self.drop_count.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic_on_drop, "retained storage destructor failure");
    }
}

#[test]
fn ordinary_drop_destroys_storage_before_releasing_its_charge() {
    let account = account(4_096, 8);
    let (capacity, plan) = byte_plan(127);
    let observed = AtomicU64::new(0);
    let drops = AtomicU64::new(0);
    let charge = RetainedStorage::<ObserveDrop<'_>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap();
    let storage = RetainedStorage::try_build(&account, &mut admission, plan, || {
        Ok(ObserveDrop {
            bytes: try_bytes(capacity)?,
            account: &account,
            observed_bytes: &observed,
            drop_count: &drops,
            panic_on_drop: false,
        })
    })
    .unwrap();

    assert!(storage.belongs_to(&account));
    assert_eq!(storage.as_ref().bytes.len(), 127);
    assert_eq!(storage.charged_bytes(), charge.bytes);
    assert_eq!(storage.charged_allocations(), 1);
    drop(storage);

    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(observed.load(Ordering::Relaxed), charge.bytes);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[test]
fn healthy_storage_uses_the_same_release_path_during_host_unwind() {
    let account = account(4_096, 8);
    let (capacity, plan) = byte_plan(255);
    let observed = AtomicU64::new(0);
    let drops = AtomicU64::new(0);
    let charge = RetainedStorage::<ObserveDrop<'_>>::checked_charge(plan).unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut admission = account
            .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
            .unwrap();
        let _storage = RetainedStorage::try_build(&account, &mut admission, plan, || {
            Ok(ObserveDrop {
                bytes: try_bytes(capacity)?,
                account: &account,
                observed_bytes: &observed,
                drop_count: &drops,
                panic_on_drop: false,
            })
        })
        .unwrap();
        panic!("unrelated host unwind");
    }));
    assert!(result.is_err());
    drop(result);

    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(observed.load(Ordering::Relaxed), charge.bytes);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn explicit_cleanup_reports_storage_failure_and_keeps_its_charge() {
    let account = account(4_096, 8);
    let (capacity, plan) = byte_plan(63);
    let observed = AtomicU64::new(0);
    let drops = AtomicU64::new(0);
    let charge = RetainedStorage::<ObserveDrop<'_>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap();
    let storage = RetainedStorage::try_build(&account, &mut admission, plan, || {
        Ok(ObserveDrop {
            bytes: try_bytes(capacity)?,
            account: &account,
            observed_bytes: &observed,
            drop_count: &drops,
            panic_on_drop: true,
        })
    })
    .unwrap();

    assert!(matches!(
        storage.release_before_attribution(),
        FinalOwnerReleaseOutcome::StorageReleaseUnverified(_)
    ));
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(observed.load(Ordering::Relaxed), charge.bytes);
    let retained = account.snapshot();
    assert_eq!(retained.live_retained_bytes(), charge.bytes);
    assert_eq!(retained.live_retained_allocations(), 1);
    assert!(retained.poisoned());
    assert_eq!(
        retained
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
}

#[test]
fn move_and_cleanup_do_not_allocate_a_shared_final_owner() {
    let account = account(4_096, 8);
    let (capacity, plan) = byte_plan(511);
    let charge = RetainedStorage::<Vec<u8>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap();
    let allocator_entries = test_retained_final_owner_allocator_entries();
    let storage =
        RetainedStorage::try_build(&account, &mut admission, plan, || try_bytes(capacity)).unwrap();
    let original_pointer = storage.as_ref().as_ptr();
    let moved = storage;

    assert_eq!(moved.as_ref().as_ptr(), original_pointer);
    assert_eq!(
        test_retained_final_owner_allocator_entries(),
        allocator_entries
    );
    assert!(matches!(
        moved.release_before_attribution(),
        FinalOwnerReleaseOutcome::Clean
    ));
    assert_eq!(
        test_retained_final_owner_allocator_entries(),
        allocator_entries
    );
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn builder_error_rolls_back_admission_without_publishing_storage() {
    let account = account(4_096, 8);
    let (_, plan) = byte_plan(32);
    let charge = RetainedStorage::<Vec<u8>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap();
    let result =
        RetainedStorage::<Vec<u8>>::try_build(&account, &mut admission, plan, || Err(BUILD_FAILED));

    assert!(matches!(
        result,
        Err(RetainedAllocationError::Build(BUILD_FAILED))
    ));
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn builder_panic_keeps_the_admission_and_poisons_the_account() {
    let account = account(4_096, 8);
    let (_, plan) = byte_plan(32);
    let charge = RetainedStorage::<Vec<u8>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap();
    let result = RetainedStorage::<Vec<u8>>::try_build(&account, &mut admission, plan, || {
        panic!("retained builder may have moved storage to another owner")
    });

    assert!(matches!(
        result,
        Err(RetainedAllocationError::BuilderPanicked)
    ));
    let retained = account.snapshot();
    assert_eq!(retained.live_retained_bytes(), charge.bytes);
    assert_eq!(retained.live_retained_allocations(), 1);
    assert!(retained.poisoned());
}

#[test]
fn mismatched_admission_rejects_before_the_builder_runs() {
    let account = account(4_096, 8);
    let (_, plan) = byte_plan(32);
    let charge = RetainedStorage::<Vec<u8>>::checked_charge(plan).unwrap();
    let mut admission = account
        .try_admit_retained(
            AdmissionCharge::new(charge.bytes + 1, charge.allocations),
            AdmittedRetainedShareWork { private: () },
        )
        .unwrap();
    let called = Cell::new(false);
    let result = RetainedStorage::<Vec<u8>>::try_build(&account, &mut admission, plan, || {
        called.set(true);
        Ok(Vec::new())
    });

    assert!(result.is_err());
    assert!(!called.get());
    assert!(account.snapshot().poisoned());
}
