use super::*;
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};
use std::sync::{Arc as StdArc, Barrier};
use std::thread;

fn factory(max_live_accounts: u64) -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_live_accounts)
            .unwrap_or_else(|error| panic!("test factory limits must be valid: {error}")),
    )
    .unwrap_or_else(|error| panic!("test factory must be created: {error}"))
}

fn account(
    factory: &PhysicalStorageAccountFactory,
    max_bytes: u64,
    max_allocations: u64,
) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(max_bytes, max_allocations)
                .unwrap_or_else(|error| panic!("test account limits must be valid: {error}")),
        )
        .unwrap_or_else(|error| panic!("test account must be created: {error}"))
}

fn charge(bytes: u64, allocations: u64) -> AdmissionCharge {
    AdmissionCharge::new(bytes, allocations)
}

fn manifest(fallback: AdmissionCharge, evidence: AdmissionCharge) -> StartupPhysicalManifest {
    StartupPhysicalManifest::try_new(fallback, evidence)
        .unwrap_or_else(|error| panic!("test manifest must be valid: {error:?}"))
}

fn shared_account(account: &PhysicalStorageAccount) -> PhysicalStorageAccount {
    PhysicalStorageAccount {
        inner: account.inner.share(),
    }
}

fn preheld_owners(account: &PhysicalStorageAccount) -> PreheldStartupAccountOwners {
    PreheldStartupAccountOwners {
        fallback_account: shared_account(account),
        evidence_account: shared_account(account),
        fallback_lease_owner: shared_account(account),
        evidence_lease_owner: shared_account(account),
    }
}

fn preheld_with_foreign_role(
    account: &PhysicalStorageAccount,
    foreign: &PhysicalStorageAccount,
    foreign_role: StartupAccountOwnerRole,
) -> PreheldStartupAccountOwners {
    let select = |role| {
        if role == foreign_role {
            shared_account(foreign)
        } else {
            shared_account(account)
        }
    };
    PreheldStartupAccountOwners {
        fallback_account: select(StartupAccountOwnerRole::FallbackWitness),
        evidence_account: select(StartupAccountOwnerRole::EvidenceWitness),
        fallback_lease_owner: select(StartupAccountOwnerRole::FallbackLease),
        evidence_lease_owner: select(StartupAccountOwnerRole::EvidenceLease),
    }
}

fn linear_state(account: &PhysicalStorageAccount) -> AccountLinearState {
    *super::super::account::lock_recovering_poison(&account.inner.value().linear)
}

fn mutate_linear_state(
    account: &PhysicalStorageAccount,
    mutate: impl FnOnce(&mut AccountLinearState),
) {
    mutate(&mut super::super::account::lock_recovering_poison(
        &account.inner.value().linear,
    ));
}

fn expect_no_mutation(
    result: Result<ReadyStartupPhysicalAdmission, StartupPhysicalAdmissionFailure>,
) -> StartupPhysicalNoMutation {
    match result {
        Err(StartupPhysicalAdmissionFailure::NoMutation(error)) => error,
        Err(StartupPhysicalAdmissionFailure::Uncertain(_)) => {
            panic!("typed rejection must not become uncertain containment")
        }
        Ok(ready) => {
            std::mem::forget(ready);
            panic!("typed rejection must not commit")
        }
    }
}

#[test]
fn manifest_checks_each_aggregate_counter() {
    assert_eq!(
        StartupPhysicalManifest::try_new(charge(u64::MAX, 1), charge(1, 1)),
        Err(StartupPhysicalManifestError::ByteOverflow)
    );
    assert_eq!(
        StartupPhysicalManifest::try_new(charge(1, u64::MAX), charge(1, 1)),
        Err(StartupPhysicalManifestError::AllocationOverflow)
    );

    let manifest = manifest(charge(11, 2), charge(17, 3));
    assert_eq!(manifest.fallback, charge(11, 2));
    assert_eq!(manifest.evidence, charge(17, 3));
    assert_eq!(manifest.aggregate, charge(28, 5));
}

#[test]
fn poisoned_account_rejects_start_before_any_action_or_share() {
    let factory = factory(1);
    let account = account(&factory, 64, 4);
    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    let before = linear_state(&account);
    let shares_before = account.share_call_count();
    reset_startup_lock_probe();

    let result = account.try_admit_operation_start();

    assert!(matches!(
        result,
        Err(OperationStartAdmissionError::AccountPoisoned(_))
    ));
    assert_eq!(linear_state(&account), before);
    assert_eq!(account.share_call_count(), shares_before);
    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot {
            operation_gate_calls: 1,
            physical_transition_calls: 0,
            current_depth: 0,
            max_depth: 1,
        }
    );
}

#[test]
fn action_counter_overflow_is_a_zero_mutation_rejection() {
    let factory = factory(1);
    let account = account(&factory, 64, 4);
    mutate_linear_state(&account, |state| {
        state.admitted_account_actions = u64::MAX;
    });
    let before = linear_state(&account);

    assert!(matches!(
        account.try_admit_operation_start(),
        Err(OperationStartAdmissionError::AccountingOverflow)
    ));
    assert_eq!(linear_state(&account), before);
}

#[test]
fn foreign_start_token_rejects_without_touching_either_account() {
    let factory = factory(2);
    let target = account(&factory, 64, 4);
    let foreign = account(&factory, 64, 4);
    let token = foreign
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("foreign token must be admitted"));
    let owners = preheld_owners(&target);
    let target_before = linear_state(&target);
    let foreign_before = linear_state(&foreign);

    let error = expect_no_mutation(target.try_admit_startup_physical(
        token,
        manifest(charge(7, 1), charge(9, 1)),
        owners,
    ));

    assert_eq!(error, StartupPhysicalNoMutation::ForeignStartAdmission);
    assert_eq!(linear_state(&target), target_before);
    assert_eq!(linear_state(&foreign), foreign_before);
    drop(target);
    drop(foreign);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn every_foreign_owner_role_is_rejected_before_physical_mutation() {
    for role in [
        StartupAccountOwnerRole::FallbackWitness,
        StartupAccountOwnerRole::EvidenceWitness,
        StartupAccountOwnerRole::FallbackLease,
        StartupAccountOwnerRole::EvidenceLease,
    ] {
        let factory = factory(2);
        let target = account(&factory, 64, 4);
        let foreign = account(&factory, 64, 4);
        let token = target
            .try_admit_operation_start()
            .unwrap_or_else(|_| panic!("target token must be admitted"));
        let owners = preheld_with_foreign_role(&target, &foreign, role);
        let target_before = linear_state(&target);
        let foreign_before = linear_state(&foreign);

        let error = expect_no_mutation(target.try_admit_startup_physical(
            token,
            manifest(charge(7, 1), charge(9, 1)),
            owners,
        ));

        assert_eq!(error, StartupPhysicalNoMutation::ForeignOwner(role));
        assert_eq!(linear_state(&target), target_before);
        assert_eq!(linear_state(&foreign), foreign_before);
        drop(target);
        drop(foreign);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn exact_limits_commit_the_complete_pair_once() {
    let factory = factory(1);
    let account = account(&factory, 28, 5);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let ready = account
        .try_admit_startup_physical(
            token,
            manifest(charge(11, 2), charge(17, 3)),
            preheld_owners(&account),
        )
        .unwrap_or_else(|_| panic!("exact limits must admit"));

    let state = linear_state(&account);
    assert_eq!(state.admitted_account_actions, 1);
    assert_eq!(state.admitted_retained_owners, 2);
    assert_eq!(state.counters.live_bytes, 28);
    assert_eq!(state.counters.live_allocations, 5);
    assert_eq!(state.counters.peak_bytes, 28);
    assert_eq!(state.counters.peak_allocations, 5);
    assert_eq!(state.counters.total_admitted_bytes, 28);
    assert_eq!(state.counters.total_admitted_allocations, 5);
    assert_eq!(ready.manifest.aggregate, charge(28, 5));
    assert!(
        ready
            .owners
            .as_ref()
            .unwrap_or_else(|| panic!("ready owners must remain armed"))
            .fallback_account
            .is_same_account(
                &ready
                    .owners
                    .as_ref()
                    .unwrap_or_else(|| panic!("ready owners must remain armed"))
                    .evidence_lease_owner,
            )
    );
    std::mem::forget(ready);
}

#[test]
fn byte_limit_rejection_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, 27, 5);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(11, 2), charge(17, 3)),
        preheld_owners(&account),
    ));

    assert_eq!(
        error,
        StartupPhysicalNoMutation::ByteLimitExceeded {
            requested: 28,
            limit: 27,
        }
    );
    assert_eq!(linear_state(&account), before);
}

#[test]
fn allocation_limit_rejection_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, 28, 4);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(11, 2), charge(17, 3)),
        preheld_owners(&account),
    ));

    assert_eq!(
        error,
        StartupPhysicalNoMutation::AllocationLimitExceeded {
            requested: 5,
            limit: 4,
        }
    );
    assert_eq!(linear_state(&account), before);
}

#[test]
fn byte_counter_overflow_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, u64::MAX, u64::MAX);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    mutate_linear_state(&account, |state| {
        state.counters.live_bytes = u64::MAX;
    });
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(1, 1), charge(1, 1)),
        preheld_owners(&account),
    ));

    assert_eq!(error, StartupPhysicalNoMutation::AccountingOverflow);
    assert_eq!(linear_state(&account), before);
}

#[test]
fn allocation_counter_overflow_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, u64::MAX, u64::MAX);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    mutate_linear_state(&account, |state| {
        state.counters.live_allocations = u64::MAX;
    });
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(1, 1), charge(1, 1)),
        preheld_owners(&account),
    ));

    assert_eq!(error, StartupPhysicalNoMutation::AccountingOverflow);
    assert_eq!(linear_state(&account), before);
}

#[test]
fn total_admitted_byte_overflow_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, u64::MAX, u64::MAX);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    mutate_linear_state(&account, |state| {
        state.counters.total_admitted_bytes = u64::MAX;
    });
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(1, 1), charge(1, 1)),
        preheld_owners(&account),
    ));

    assert_eq!(error, StartupPhysicalNoMutation::AccountingOverflow);
    assert_eq!(linear_state(&account), before);
}

#[test]
fn total_admitted_allocation_overflow_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, u64::MAX, u64::MAX);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    mutate_linear_state(&account, |state| {
        state.counters.total_admitted_allocations = u64::MAX;
    });
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(1, 1), charge(1, 1)),
        preheld_owners(&account),
    ));

    assert_eq!(error, StartupPhysicalNoMutation::AccountingOverflow);
    assert_eq!(linear_state(&account), before);
}

#[test]
fn retained_owner_overflow_preserves_the_complete_linear_state() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    mutate_linear_state(&account, |state| {
        state.admitted_retained_owners = u64::MAX;
    });
    let before = linear_state(&account);

    let error = expect_no_mutation(account.try_admit_startup_physical(
        token,
        manifest(charge(2, 1), charge(3, 1)),
        preheld_owners(&account),
    ));

    assert_eq!(error, StartupPhysicalNoMutation::RetainedOwnerOverflow);
    assert_eq!(linear_state(&account), before);
}

#[test]
fn poison_after_start_eligibility_does_not_invalidate_the_physical_commit() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let first_poison = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    assert_eq!(first_poison.snapshot().poison_event_count(), 1);

    let ready = account
        .try_admit_startup_physical(
            token,
            manifest(charge(2, 1), charge(3, 1)),
            preheld_owners(&account),
        )
        .unwrap_or_else(|_| panic!("admission-first transition must commit"));

    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 5);
    assert_eq!(snapshot.live_retained_allocations(), 2);
    assert!(snapshot.poisoned());
    std::mem::forget(ready);
}

#[test]
fn panic_before_commit_returns_uncertain_without_a_physical_charge() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    set_startup_transition_panic(StartupTransitionPanicPoint::BeforeCommit);

    let uncertain = match account.try_admit_startup_physical(
        token,
        manifest(charge(2, 1), charge(3, 1)),
        preheld_owners(&account),
    ) {
        Err(StartupPhysicalAdmissionFailure::Uncertain(uncertain)) => uncertain,
        Err(StartupPhysicalAdmissionFailure::NoMutation(_)) => {
            panic!("injected panic must not become a typed rejection")
        }
        Ok(ready) => {
            std::mem::forget(ready);
            panic!("injected panic must not report success")
        }
    };

    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    assert!(snapshot.poisoned());
    assert_eq!(
        uncertain.poison.reason,
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(uncertain.manifest.aggregate, charge(5, 2));
    assert!(
        uncertain
            .owners
            .as_ref()
            .unwrap_or_else(|| panic!("uncertain owners must remain armed"))
            .fallback_account
            .is_same_account(&account)
    );
    drop(uncertain);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn panic_after_commit_returns_uncertain_with_the_complete_charge() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    set_startup_transition_panic(StartupTransitionPanicPoint::AfterCommit);

    let uncertain = match account.try_admit_startup_physical(
        token,
        manifest(charge(2, 1), charge(3, 1)),
        preheld_owners(&account),
    ) {
        Err(StartupPhysicalAdmissionFailure::Uncertain(uncertain)) => uncertain,
        Err(StartupPhysicalAdmissionFailure::NoMutation(_)) => {
            panic!("injected panic must not become a typed rejection")
        }
        Ok(ready) => {
            std::mem::forget(ready);
            panic!("injected panic must not report success")
        }
    };

    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 5);
    assert_eq!(snapshot.live_retained_allocations(), 2);
    assert!(snapshot.poisoned());
    drop(uncertain);
    let after_drop = account.snapshot();
    assert_eq!(after_drop.live_retained_bytes(), 5);
    assert_eq!(after_drop.live_retained_allocations(), 2);
}

#[test]
fn ready_drop_poison_retains_the_complete_charge_and_all_owners() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let ready = account
        .try_admit_startup_physical(
            token,
            manifest(charge(2, 1), charge(3, 1)),
            preheld_owners(&account),
        )
        .unwrap_or_else(|_| panic!("startup physical pair must commit"));

    drop(ready);

    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 5);
    assert_eq!(snapshot.live_retained_allocations(), 2);
    assert!(snapshot.poisoned());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
}

#[test]
fn disarmed_ready_drop_does_not_forge_an_ownership_fault() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let mut ready = account
        .try_admit_startup_physical(
            token,
            manifest(charge(2, 1), charge(3, 1)),
            preheld_owners(&account),
        )
        .unwrap_or_else(|_| panic!("startup physical pair must commit"));
    let owners = ready
        .owners
        .take()
        .unwrap_or_else(|| panic!("ready owners must be extractable exactly once"));
    std::mem::forget(owners);

    drop(ready);

    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 5);
    assert_eq!(snapshot.live_retained_allocations(), 2);
    assert!(!snapshot.poisoned());
}

#[test]
fn transition_consumes_exactly_four_preheld_shares_without_cloning() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    let initial_shares = account.share_call_count();
    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let owners = preheld_owners(&account);
    assert_eq!(account.share_call_count(), initial_shares + 4);
    let shares_before_transition = account.share_call_count();

    let ready = account
        .try_admit_startup_physical(token, manifest(charge(2, 1), charge(3, 1)), owners)
        .unwrap_or_else(|_| panic!("startup physical pair must commit"));

    assert_eq!(account.share_call_count(), shares_before_transition);
    std::mem::forget(ready);
}

#[test]
fn exact_limit_race_commits_zero_or_the_complete_pair() {
    let factory = factory(1);
    let account = account(&factory, 5, 2);
    let barrier = StdArc::new(Barrier::new(2));

    let successes = thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let barrier = StdArc::clone(&barrier);
            let account = &account;
            handles.push(scope.spawn(move || {
                let token = account
                    .try_admit_operation_start()
                    .unwrap_or_else(|_| panic!("both starts must linearize before transition"));
                let owners = preheld_owners(account);
                barrier.wait();
                match account.try_admit_startup_physical(
                    token,
                    manifest(charge(2, 1), charge(3, 1)),
                    owners,
                ) {
                    Ok(ready) => {
                        std::mem::forget(ready);
                        1_u8
                    }
                    Err(StartupPhysicalAdmissionFailure::NoMutation(
                        StartupPhysicalNoMutation::ByteLimitExceeded { .. }
                        | StartupPhysicalNoMutation::AllocationLimitExceeded { .. },
                    )) => 0,
                    Err(StartupPhysicalAdmissionFailure::NoMutation(_)) => {
                        panic!("race must fail only at an exact physical limit")
                    }
                    Err(StartupPhysicalAdmissionFailure::Uncertain(_)) => {
                        panic!("race must not enter uncertain containment")
                    }
                }
            }));
        }
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| panic!("startup race thread must complete"))
            })
            .sum::<u8>()
    });

    assert_eq!(successes, 1);
    let state = linear_state(&account);
    assert_eq!(state.counters.live_bytes, 5);
    assert_eq!(state.counters.live_allocations, 2);
    assert_eq!(state.admitted_retained_owners, 2);
    assert_eq!(state.admitted_account_actions, 2);
}

#[test]
fn startup_uses_two_sequential_account_locks() {
    let factory = factory(1);
    let account = account(&factory, 8, 2);
    reset_startup_lock_probe();

    let token = account
        .try_admit_operation_start()
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let ready = account
        .try_admit_startup_physical(
            token,
            manifest(charge(2, 1), charge(3, 1)),
            preheld_owners(&account),
        )
        .unwrap_or_else(|_| panic!("physical transition must commit"));

    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot {
            operation_gate_calls: 1,
            physical_transition_calls: 1,
            current_depth: 0,
            max_depth: 1,
        }
    );
    std::mem::forget(ready);
}

#[test]
fn startup_cross_plane_rail_has_one_qualified_ledger_path_and_no_legacy_authority() {
    let source = include_str!("../startup.rs");
    assert!(source.contains("plan: StartupCrossPlanePlan<O, P, D>"));
    assert_eq!(
        source
            .matches(".into_admitted_startup_attribution_graph(parts)")
            .count(),
        1
    );
    assert!(source.contains("struct StartupGraphPreholdAuthority<"));
    for forbidden in [
        "allocator",
        "try_reserve",
        "AdmissionLease",
        "PendingRetainedAdmission",
        "ManuallyDrop",
        "unsafe",
    ] {
        assert!(
            !source.contains(forbidden),
            "physical startup rail regained forbidden authority `{forbidden}`"
        );
    }
}

#[test]
fn only_ready_cross_plane_result_has_a_production_split() {
    let source = include_str!("../startup.rs");
    assert!(!source.contains("impl ReadyStartupPhysicalAdmission<'_> {"));
    assert!(!source.contains("impl UncertainStartupPhysicalAdmission {"));
    assert_eq!(source.matches("pub(super) fn split(").count(), 1);
    for forbidden in ["fn accept(", "fn into_parts("] {
        assert!(
            !source.contains(forbidden),
            "startup result regained extraction seam `{forbidden}`"
        );
    }
}

#[test]
fn startup_module_remains_private_to_the_retained_deep_module() {
    let retained_root = include_str!("../../retained.rs");
    let crate_root = include_str!("../../lib.rs");
    assert!(retained_root.contains("\nmod startup;"));
    assert!(!retained_root.contains("pub mod startup;"));
    assert!(!retained_root.contains("pub(crate) mod startup;"));
    assert!(!crate_root.contains("startup::"));
    assert!(!crate_root.contains("StartupPhysicalManifest"));
    assert!(!crate_root.contains("ReadyStartupPhysicalAdmission"));
}
