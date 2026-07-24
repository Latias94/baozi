use super::*;

#[test]
fn explicit_limits_reject_zero_and_plans_reject_mismatched_nested_evidence() {
    assert_eq!(
        PhysicalStorageAccountFactoryLimits::new(0, 1),
        Err(PhysicalStorageLimitError::ZeroByteLimit)
    );
    assert_eq!(
        PhysicalStorageAccountLimits::new(1, 0),
        Err(PhysicalStorageLimitError::ZeroAllocationLimit)
    );
    assert_eq!(
        RetainedAllocationPlan::new(1, 0),
        Err(RetainedAllocationPlanError::MismatchedNestedFootprint {
            bytes: 1,
            allocations: 0,
        })
    );
}

#[test]
fn inline_ceiling_covers_the_complete_extracted_ownership_header() {
    let factory = account_factory(1);
    let account = account(&factory, 16_384, 8);
    let result = account.try_retain_with(empty_plan(), || {
        Ok::<_, RetainedBuildFailureCode>([0_u8; MAX_RETAINED_INLINE_BYTES])
    });

    assert!(matches!(
        result,
        Err(RetainedAllocationError::InlinePayloadTooLarge {
            bytes,
            limit: MAX_RETAINED_INLINE_BYTES,
        }) if bytes > MAX_RETAINED_INLINE_BYTES
    ));
}

#[test]
fn factory_enforces_exact_live_account_and_control_byte_limits() {
    let control_bytes =
        FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
    let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
    let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    let first = factory.try_create_account(account_limits).unwrap();
    assert_eq!(factory.snapshot().live_accounts(), 1);
    assert_eq!(
        factory.snapshot().live_account_control_bytes(),
        control_bytes
    );
    assert!(matches!(
        factory.try_create_account(account_limits),
        Err(PhysicalStorageAccountCreateError::ControlByteLimitExceeded { .. })
            | Err(PhysicalStorageAccountCreateError::LiveAccountLimitExceeded { .. })
    ));

    drop(first);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn account_control_allocation_failure_rolls_back_factory_admission() {
    let factory = account_factory(1);
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    fail_next_final_owner_allocation();
    assert_eq!(
        factory.try_create_account(account_limits).err(),
        Some(PhysicalStorageAccountCreateError::AllocationFailed)
    );
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn last_account_control_owner_releases_factory_admission_during_unwind() {
    let factory = account_factory(1);
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        let _account = factory.try_create_account(account_limits).unwrap();
        panic!("drop the final account control owner during unwind");
    }));

    assert!(result.is_err());
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn concurrent_factory_admission_cannot_oversubscribe_one_account_slot() {
    let control_bytes =
        FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
    let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
    let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();
    let start = StdArc::new(Barrier::new(3));
    let attempts_complete = StdArc::new(Barrier::new(3));
    let release_owner = StdArc::new(Barrier::new(2));
    let successes = StdArc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let factory = factory.clone();
        let start = StdArc::clone(&start);
        let attempts_complete = StdArc::clone(&attempts_complete);
        let release_owner = StdArc::clone(&release_owner);
        let successes = StdArc::clone(&successes);
        threads.push(thread::spawn(move || {
            start.wait();
            let account = factory.try_create_account(account_limits).ok();
            if account.is_some() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempts_complete.wait();
            if let Some(account) = account {
                release_owner.wait();
                drop(account);
            }
        }));
    }

    start.wait();
    attempts_complete.wait();
    assert_eq!(successes.load(Ordering::SeqCst), 1);
    release_owner.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn retained_admission_accepts_exact_limits_and_rejects_excess() {
    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer + 11, 2);
    let plan = RetainedAllocationPlan::new(11, 1).unwrap();

    let retained = account
        .try_retain_with(plan, || Ok::<_, RetainedBuildFailureCode>(7_u64))
        .unwrap();
    assert_eq!(retained.charged_bytes(), outer + 11);
    assert_eq!(retained.charged_allocations(), 2);
    assert_eq!(account.snapshot().live_retained_bytes(), outer + 11);
    assert!(matches!(
        account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(9_u64)
        }),
        Err(RetainedAllocationError::ByteLimitExceeded { .. })
            | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
    ));

    drop(retained);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_error_code_is_returned_after_charge_release() {
    let factory = account_factory(1);
    let account = account(&factory, 1_024, 8);

    let error = account
        .try_retain_with::<u64>(empty_plan(), || Err(BUILD_FAILED))
        .unwrap_err();
    assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_local_storage_drops_before_its_physical_lease() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct BuilderLocalStorage {
        account: PhysicalStorageAccount,
        _bytes: Vec<u8>,
    }

    impl Drop for BuilderLocalStorage {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();
    let error = account
        .try_retain_with::<u64>(plan, || {
            let _storage = BuilderLocalStorage {
                account: account
                    .share_after_test_admission()
                    .unwrap_or_else(|_| panic!("test account share must be admitted")),
                _bytes: vec![0; 64],
            };
            Err(BUILD_FAILED)
        })
        .unwrap_err();

    assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
    assert!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst) > 64);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn every_builder_panic_payload_keeps_its_physical_charge_fail_closed() {
    let factory = account_factory(3);
    let borrowed_account = account(&factory, 4_096, 16);
    let before = borrowed_account.snapshot();

    let borrowed = borrowed_account
        .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
        .unwrap_err();
    assert_eq!(borrowed, RetainedAllocationError::BuilderPanicked);
    let after_borrowed = borrowed_account.snapshot();
    assert!(after_borrowed.live_retained_bytes() > before.live_retained_bytes());
    assert!(after_borrowed.live_retained_allocations() > before.live_retained_allocations());
    assert!(after_borrowed.poisoned());
    assert_eq!(after_borrowed.poison_event_count(), 1);
    assert!(after_borrowed.poison_event_count_exact());
    assert!(after_borrowed.poison_event_domain_valid());
    assert_eq!(
        after_borrowed
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );

    let later_builder_ran = AtomicBool::new(false);
    let later = borrowed_account
        .try_retain_with::<u64>(empty_plan(), || {
            later_builder_ran.store(true, Ordering::SeqCst);
            Ok(7)
        })
        .unwrap_err();
    assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
    assert!(!later_builder_ran.load(Ordering::SeqCst));
    assert_eq!(borrowed_account.snapshot(), after_borrowed);

    let owned_account = account(&factory, 4_096, 16);
    let owned_before = owned_account.snapshot();
    let owned = owned_account
        .try_retain_with::<u64>(empty_plan(), || {
            std::panic::panic_any(String::from("owned test panic"))
        })
        .unwrap_err();
    assert_eq!(owned, RetainedAllocationError::BuilderPanicked);
    let after_owned = owned_account.snapshot();
    assert!(after_owned.live_retained_bytes() > owned_before.live_retained_bytes());
    assert!(after_owned.live_retained_allocations() > owned_before.live_retained_allocations());

    let opaque_account = account(&factory, 4_096, 16);
    let opaque_before = opaque_account.snapshot();
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();
    let opaque = opaque_account
        .try_retain_with::<u64>(plan, || std::panic::panic_any(vec![0_u8; 64]))
        .unwrap_err();
    assert_eq!(opaque, RetainedAllocationError::BuilderPanicked);
    let after_opaque = opaque_account.snapshot();
    assert!(after_opaque.live_retained_bytes() > opaque_before.live_retained_bytes() + 64);
    assert!(after_opaque.live_retained_allocations() > opaque_before.live_retained_allocations());
    drop(borrowed_account);
    drop(owned_account);
    drop(opaque_account);
    assert_eq!(factory.snapshot().live_accounts(), 3);
}

#[test]
fn shared_handles_observe_one_latched_first_event_and_exact_event_count() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let observer = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("observer share must be admitted before poison"));
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("first poison probe must retain"));
    let second = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("second poison probe must retain"));

    drop(first);
    let after_first = account.snapshot();
    assert_eq!(after_first, observer.snapshot());
    assert_eq!(after_first.poison_event_count(), 1);
    assert_eq!(
        after_first
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );

    drop(second);
    let after_second = account.snapshot();
    assert_eq!(after_second, observer.snapshot());
    assert_eq!(after_second.poison_event_count(), 2);
    assert!(after_second.poison_event_count_exact());
    assert!(after_second.poison_event_domain_valid());
    assert_eq!(
        after_second.first_poison_evidence(),
        after_first.first_poison_evidence()
    );
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);

    let before_rejected_share = account.snapshot();
    let rejected_share = account.share_after_test_admission().unwrap_err();
    assert!(matches!(
        rejected_share,
        RetainedAllocationError::AccountPoisoned(_)
    ));
    assert_eq!(account.snapshot(), before_rejected_share);
}

#[test]
fn poison_first_start_rejects_without_builder_or_account_side_effects() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("poison probe must retain"));
    let started = StdArc::new(Barrier::new(2));
    let poisoned = StdArc::new(Barrier::new(2));
    let thread_started = StdArc::clone(&started);
    let thread_poisoned = StdArc::clone(&poisoned);
    let poison_thread = thread::spawn(move || {
        thread_started.wait();
        drop(poisoner);
        thread_poisoned.wait();
    });
    started.wait();
    poisoned.wait();
    poison_thread
        .join()
        .unwrap_or_else(|_| panic!("poison thread must be contained"));

    let before = account.snapshot();
    let builder_ran = AtomicBool::new(false);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            builder_ran.store(true, Ordering::SeqCst);
            Ok(7)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert!(!builder_ran.load(Ordering::SeqCst));
    assert_eq!(account.snapshot(), before);
}

#[test]
fn admission_first_start_panic_after_external_poison_records_distinct_builder_fault() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 8_192, 32);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("startup panic poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("startup worker account share must precede poison"));
    let shares_before_start = account.share_call_count();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        TestOperation::try_start(
            OperationControl::default(),
            limits,
            &worker_account,
            plan,
            || {
                worker_entered.wait();
                worker_continue.wait();
                std::panic::panic_any(vec![0_u8; 8]);
            },
        )
    });

    builder_entered.wait();
    drop(poisoner);
    let after_external_poison = account.snapshot();
    assert!(after_external_poison.poisoned());
    let first_evidence = after_external_poison.first_poison_evidence();
    builder_continue.wait();
    let result = worker
        .join()
        .unwrap_or_else(|_| panic!("startup builder panic must be contained"));
    let after_builder_panic = account.snapshot();

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(
        after_builder_panic.poison_event_count(),
        after_external_poison.poison_event_count() + 1
    );
    assert_eq!(after_builder_panic.first_poison_evidence(), first_evidence);
    assert_eq!(
        after_builder_panic.live_retained_bytes(),
        after_external_poison.live_retained_bytes()
    );
    assert_eq!(
        after_builder_panic.live_retained_allocations(),
        after_external_poison.live_retained_allocations()
    );
    assert_eq!(account.share_call_count(), shares_before_start + 4);
}

#[test]
fn admission_first_physical_retain_completes_then_later_retain_is_rejected() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("physical-race poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("worker account share must precede poison"));
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        worker_account
            .try_retain_with(empty_plan(), || {
                worker_entered.wait();
                worker_continue.wait();
                Ok::<_, RetainedBuildFailureCode>(99_u64)
            })
            .unwrap_or_else(|_| panic!("admission-first retain must complete"))
    });

    builder_entered.wait();
    drop(poisoner);
    let after_poison = account.snapshot();
    assert!(after_poison.poisoned());
    builder_continue.wait();
    let retained = worker
        .join()
        .unwrap_or_else(|_| panic!("retaining worker must finish"));
    assert_eq!(retained.as_ref(), &99);
    assert_eq!(account.snapshot(), after_poison);

    let later_builder_ran = AtomicBool::new(false);
    let later = account
        .try_retain_with(empty_plan(), || {
            later_builder_ran.store(true, Ordering::SeqCst);
            Ok::<_, RetainedBuildFailureCode>(100_u64)
        })
        .unwrap_err();
    assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
    assert!(!later_builder_ran.load(Ordering::SeqCst));
    assert_eq!(account.snapshot(), after_poison);
    drop(retained);
}

#[test]
fn admission_first_builder_error_after_poison_retains_charge_without_rollback_clone() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("builder-error poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("worker account share must precede poison"));
    let shares_before_retain = account.share_call_count();
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        worker_account
            .try_retain_with::<u64>(empty_plan(), || {
                worker_entered.wait();
                worker_continue.wait();
                Err(BUILD_FAILED)
            })
            .expect_err("admission-first builder error must not publish storage")
    });

    builder_entered.wait();
    drop(poisoner);
    let after_poison = account.snapshot();
    assert!(after_poison.poisoned());
    builder_continue.wait();
    let error = worker
        .join()
        .unwrap_or_else(|_| panic!("builder-error worker must finish"));

    assert!(matches!(
        error,
        RetainedAllocationError::AccountPoisonedWithRetainedCharge(_)
    ));
    assert_eq!(account.snapshot(), after_poison);
    assert_eq!(account.share_call_count(), shares_before_retain + 1);
    assert!(matches!(
        account.try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(1_u64)),
        Err(RetainedAllocationError::AccountPoisoned(_))
    ));
}

#[test]
fn admission_first_direct_fallback_ends_in_finalization_containment() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("post-start poison probe must retain"));
    let started = StdArc::new(Barrier::new(2));
    let poisoned = StdArc::new(Barrier::new(2));
    let thread_started = StdArc::clone(&started);
    let thread_poisoned = StdArc::clone(&poisoned);
    let poison_thread = thread::spawn(move || {
        thread_started.wait();
        drop(poisoner);
        thread_poisoned.wait();
    });
    started.wait();
    poisoned.wait();
    poison_thread
        .join()
        .unwrap_or_else(|_| panic!("poison thread must be contained"));
    let before_prepare = account.snapshot();
    let first_evidence = before_prepare.first_poison_evidence();
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let active_before = ledger.active_reservations();
    let live_before = ledger.current_live_bytes();

    let result = operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(91));
    let after_prepare = account.snapshot();
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
                && cause.account_poisoned().snapshot() == after_prepare
    ));
    assert_eq!(
        after_prepare.poison_event_count(),
        before_prepare.poison_event_count() + 2
    );
    assert!(after_prepare.poison_event_count_exact());
    assert!(after_prepare.poison_event_domain_valid());
    assert_eq!(after_prepare.first_poison_evidence(), first_evidence);
    assert_eq!(ledger.active_reservations(), active_before);
    assert_eq!(ledger.current_live_bytes(), live_before);
    assert_eq!(
        after_prepare.live_retained_bytes(),
        before_prepare.live_retained_bytes()
    );
    assert_eq!(
        after_prepare.live_retained_allocations(),
        before_prepare.live_retained_allocations()
    );
}

#[test]
fn poisoned_inherited_share_has_no_lookup_work_clone_or_target_side_effect() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source, source_slot, source_evidence, _source_evidence_slot) =
        publish_scalar_source(&account);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("target publication must open"));
    let source_allocation = source
        .aggregate
        .inner
        .test_source_allocation(source_slot.identity)
        .unwrap_or_else(|| panic!("source slot must resolve before poison"));
    let source_share_calls = source_allocation.share_call_count();
    let target_len = assembly.canonical_len();
    let ledger = assembly.test_ledger_probe();
    let active = ledger.active_reservations();
    let live = ledger.current_live_bytes();
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("share poison probe must retain"));
    drop(poisoner);
    let poisoned = account.snapshot();
    let _ = take_test_member_lookups();

    let result = assembly.try_share_canonical(source.aggregate(), source_slot);
    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(assembly.canonical_len(), target_len);
    assert_eq!(ledger.active_reservations(), active);
    assert_eq!(ledger.current_live_bytes(), live);
    assert_eq!(source_allocation.share_call_count(), source_share_calls);
    assert_eq!(take_test_member_lookups(), 0);
    assert_eq!(account.snapshot(), poisoned);
    drop(source_evidence);
}

#[test]
fn admission_first_inherited_share_completes_then_later_share_is_rejected() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source, source_slot, source_evidence, _source_evidence_slot) =
        publish_scalar_source(&account);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(3, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("admission-first share target must open"));
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("share-race poison probe must retain"));
    let target_account = assembly
        .account
        .as_ref()
        .unwrap_or_else(|| panic!("target retains account"));
    let share_admission = target_account
        .try_admit_account_action()
        .unwrap_or_else(|_| panic!("share action must be admitted before poison"));
    drop(poisoner);
    let poisoned = account.snapshot();
    let source_share_calls = source
        .aggregate
        .inner
        .test_member_share_call_count(source_slot.identity)
        .unwrap_or_else(|| panic!("source slot must resolve"));
    let source_allocation = source
        .aggregate
        .inner
        .test_source_allocation(source_slot.identity)
        .unwrap_or_else(|| panic!("source slot must resolve"));
    let ledger = assembly
        .operation
        .as_ref()
        .unwrap_or_else(|| panic!("target retains operation"))
        .coordinator
        .retained_ledger();
    let identity = try_share_retained_block_after_admission::<
        TestOwner,
        TestPhase,
        TestDimension,
        u64,
        CanonicalRootTag,
    >(
        ledger,
        share_admission,
        assembly.handle_share_work,
        assembly
            .canonical_table
            .as_mut()
            .and_then(PolicyRetainedAllocation::table_mut_during_construction)
            .unwrap_or_else(|| panic!("target retains canonical table")),
        assembly.set_id,
        RetainedMembershipDestination::Canonical,
        0,
        1,
        source_allocation,
    )
    .unwrap_or_else(|_| panic!("admission-first share must complete"));
    assert_eq!(identity.index, 0);
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(source_allocation.share_call_count(), source_share_calls + 1);
    assert_eq!(account.snapshot(), poisoned);

    let before_later = assembly.canonical_len();
    let later = assembly.try_share_canonical(source.aggregate(), source_slot);
    assert!(matches!(
        later,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(assembly.canonical_len(), before_later);
    drop(source_evidence);
}

#[test]
fn prepared_handoff_is_a_pure_move_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("handoff poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let prepared = operation
        .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(92))
        .unwrap_or_else(|_| panic!("healthy account must prepare fallback"));
    drop(poisoner);
    let before_handoff = account.snapshot();

    let published = handoff_for_test(prepared);
    assert_eq!(account.snapshot(), before_handoff);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::MinimalFallback { .. }
    ));
}

#[test]
fn eligibility_first_capability_completes_preparation_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("eligibility race poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let eligibility_entered = StdArc::new(Barrier::new(2));
    let eligibility_resume = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&eligibility_entered);
    let worker_resume = StdArc::clone(&eligibility_resume);
    let worker = thread::spawn(move || {
        install_publication_eligibility_pause(worker_entered, worker_resume);
        operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(93))
    });

    eligibility_entered.wait();
    drop(poisoner);
    let poisoned = account.snapshot();
    assert!(poisoned.poisoned());
    eligibility_resume.wait();

    let prepared = worker
        .join()
        .unwrap_or_else(|_| panic!("eligibility-first worker must remain contained"))
        .unwrap_or_else(|_| panic!("eligibility-first preparation must complete"));
    assert_eq!(account.snapshot(), poisoned);
    let published = handoff_for_test(prepared);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::MinimalFallback { .. }
    ));
}

#[test]
fn primary_eligibility_first_capability_completes_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("primary eligibility race poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("primary eligibility publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("primary eligibility candidate must validate"),
        };
    let fallback_charge = account
        .checked_retained_charge::<u32>(empty_plan())
        .unwrap_or_else(|_| panic!("fallback charge must remain derivable from its admitted plan"));
    let eligibility_entered = StdArc::new(Barrier::new(2));
    let eligibility_resume = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&eligibility_entered);
    let worker_resume = StdArc::clone(&eligibility_resume);
    let worker = thread::spawn(move || {
        install_publication_eligibility_pause(worker_entered, worker_resume);
        validated.try_prepare(final_progress())
    });

    eligibility_entered.wait();
    drop(poisoner);
    let poisoned = account.snapshot();
    assert!(poisoned.poisoned());
    eligibility_resume.wait();

    let prepared = worker
        .join()
        .unwrap_or_else(|_| panic!("primary eligibility worker must remain contained"))
        .unwrap_or_else(|_| panic!("primary eligibility preparation must complete"));
    let after_prepare = account.snapshot();
    assert_eq!(
        after_prepare
            .live_retained_bytes()
            .checked_add(fallback_charge.bytes),
        Some(poisoned.live_retained_bytes())
    );
    assert_eq!(
        after_prepare
            .live_retained_allocations()
            .checked_add(fallback_charge.allocations),
        Some(poisoned.live_retained_allocations())
    );
    assert_eq!(
        after_prepare.total_admitted_retained_bytes(),
        poisoned.total_admitted_retained_bytes()
    );
    assert_eq!(
        after_prepare.total_admitted_retained_allocations(),
        poisoned.total_admitted_retained_allocations()
    );
    assert_eq!(
        after_prepare.first_poison_evidence(),
        poisoned.first_poison_evidence()
    );
    let published = handoff_for_test(prepared);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::Success { .. }
    ));
}

#[test]
fn release_underflow_latches_counter_invariant_without_saturating_event_count() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);

    let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
    let first = account.snapshot();
    assert!(first.poisoned());
    assert_eq!(first.poison_event_count(), 1);
    assert!(first.poison_event_count_exact());
    assert!(!first.poison_event_domain_valid());
    assert_eq!(
        first
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );

    let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
    let second = account.snapshot();
    assert_eq!(second.poison_event_count(), 2);
    assert!(second.poison_event_count_exact());
    assert!(!second.poison_event_domain_valid());
    assert_eq!(
        second.first_poison_evidence(),
        first.first_poison_evidence()
    );
}

#[test]
fn pending_single_rollback_underflow_records_one_in_domain_counter_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let charge = AdmissionCharge::new(1, 1);
    let admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap_or_else(|_| panic!("single pending charge must admit"));
    {
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        state.counters.live_bytes = 0;
        state.counters.live_allocations = 0;
    }

    let rollback =
        admission.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
    assert!(matches!(
        rollback,
        PendingRetainedRollback::ChargeReleaseUnverified(_)
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
}

#[test]
fn pending_pair_rollback_underflow_records_one_in_domain_counter_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("pair charge must qualify"));
    let admissions = PendingStartRetainedAdmissions::try_admit_for_test(&account, pair)
        .unwrap_or_else(|_| panic!("pending pair must admit"));
    {
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        state.counters.live_bytes = 0;
        state.counters.live_allocations = 0;
    }

    let rollback =
        admissions.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
    assert!(matches!(
        rollback,
        PendingRetainedRollback::ChargeReleaseUnverified(_)
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
}

#[test]
fn qualified_startup_capacity_failure_rolls_back_without_poisoning() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("capacity pair charge must qualify"));
    let admissions = PendingStartRetainedAdmissions::try_admit_for_test(&account, pair)
        .unwrap_or_else(|_| panic!("capacity pair must admit"));

    let error = map_qualified_startup_batch_error::<TestDimension>(
        AttributionBatchCreateError::CapacityOverflow,
        |outer_reason| admissions.rollback_with_fault(outer_reason),
    );
    assert!(matches!(error, RetainedWriterError::BatchCapacity));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    assert!(!snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 0);
}

#[test]
fn action_admission_counter_exhaustion_is_a_clean_non_poisoning_rejection() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    lock_recovering_poison(&account.inner.value().linear).admitted_account_actions = u64::MAX;
    let before = account.snapshot();

    let result = account.try_admit_account_action();
    assert!(matches!(
        result,
        Err(AccountActionAdmissionError::AccountingOverflow)
    ));
    assert_eq!(account.snapshot(), before);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn side_owner_escape_keeps_physical_and_operation_attribution_fail_closed() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("side-owner publication must open"));
    let ledger = assembly
        .operation
        .as_ref()
        .expect("side-owner assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let account = assembly
        .account
        .as_ref()
        .expect("side-owner assembly retains account")
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let side_owner = StdArc::new(StdMutex::new(None));
    let escaped = StdArc::clone(&side_owner);
    let active_before = ledger.active_reservations();
    let live_before = ledger.current_live_bytes();
    let physical_before = account.snapshot();
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();

    let result = assembly.try_retain_canonical::<CanonicalRootTag>(plan, move |_emplacer| {
        *lock_recovering_poison(&escaped) = Some(vec![0_u8; 64]);
        panic!("builder escaped side-owned storage")
    });

    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert!(lock_recovering_poison(&side_owner).is_some());
    assert_eq!(ledger.active_reservations(), active_before + 1);
    assert!(ledger.current_live_bytes() > live_before);
    let physical_after = account.snapshot();
    assert!(physical_after.live_retained_bytes() > physical_before.live_retained_bytes() + 64);
    assert!(
        physical_after.live_retained_allocations() > physical_before.live_retained_allocations()
    );
    let retained_live = ledger.current_live_bytes();
    drop(lock_recovering_poison(&side_owner).take());
    assert_eq!(ledger.current_live_bytes(), retained_live);
    drop(assembly);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn injected_shared_allocation_failure_does_not_run_the_builder() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct DropProbe {
        account: PhysicalStorageAccount,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let result = account.try_retain_with(empty_plan(), || {
        Ok::<_, RetainedBuildFailureCode>(DropProbe {
            account: account
                .share_after_test_admission()
                .unwrap_or_else(|_| panic!("test account share must be admitted")),
        })
    });

    assert!(matches!(
        result,
        Err(RetainedAllocationError::AllocationFailed)
    ));
    assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn allocation_failure_precedes_payload_construction_and_releases_the_lease() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingDrop;

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let result = catch_unwind(AssertUnwindSafe(|| {
        account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
        })
    }));

    assert!(matches!(
        result,
        Ok(Err(RetainedAllocationError::AllocationFailed))
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn allocation_failure_rolls_back_block_attribution_before_build() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains the operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();

    DROP_COUNT.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let error = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
        })
        .expect_err("injected shared allocation must fail");

    assert!(matches!(
        error,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::AllocationFailed
        ))
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(probe.active_reservations(), before_active);
    assert_eq!(probe.current_live_bytes(), before_live);
    drop(assembly);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn private_shares_charge_once_and_release_only_after_the_final_owner() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(*b"payload!")
        })
        .unwrap();
    let charged = first.charged_bytes();
    assert_eq!(first.share_call_count(), 0);
    let second = first.share_after_test_admission();

    assert!(first.ptr_eq(&second));
    assert_eq!(first.share_call_count(), 1);
    assert_eq!(second.share_call_count(), 1);
    let third = second.share_after_test_admission();
    assert_eq!(first.share_call_count(), 2);
    assert_eq!(second.share_call_count(), 2);
    assert_eq!(third.share_call_count(), 2);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(first);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(second);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(third);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn concurrent_shared_drops_select_exactly_one_final_owner() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct DropCount;

    impl Drop for DropCount {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropCount)
        })
        .unwrap();
    let second = first.share_after_test_admission();
    let third = first.share_after_test_admission();
    let start = StdArc::new(Barrier::new(4));
    let mut threads = Vec::new();
    for retained in [first, second, third] {
        let start = StdArc::clone(&start);
        threads.push(thread::spawn(move || {
            start.wait();
            drop(retained);
        }));
    }

    start.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn final_shared_owner_contains_payload_drop_panic_and_retains_the_lease() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingDrop;

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
        })
        .unwrap();
    let charged = first.charged_bytes();
    let second = first.share_after_test_admission();
    drop(first);

    assert!(catch_unwind(AssertUnwindSafe(|| drop(second))).is_ok());
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn payload_drop_barrier_keeps_headroom_unavailable_until_cleanup_finishes() {
    static ENTERED: Barrier = Barrier::new(2);
    static RELEASE: Barrier = Barrier::new(2);

    struct BlockingDrop;

    impl Drop for BlockingDrop {
        fn drop(&mut self) {
            ENTERED.wait();
            RELEASE.wait();
        }
    }

    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<BlockingDrop>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer, 1);
    let retained = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(BlockingDrop)
        })
        .unwrap();
    let drop_thread = thread::spawn(move || drop(retained));

    ENTERED.wait();
    assert!(matches!(
        account.try_retain_with::<BlockingDrop>(empty_plan(), || {
            panic!("builder must not run before physical admission")
        }),
        Err(RetainedAllocationError::ByteLimitExceeded { .. })
            | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
    ));
    RELEASE.wait();
    drop_thread.join().unwrap();
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    let replacement = account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(()))
        .expect("headroom becomes available after payload cleanup and lease release");
    drop(replacement);
}

#[test]
fn payload_drop_observes_charge_and_factory_outlives_public_account_handle() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct DropProbe {
        account: PhysicalStorageAccount,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    let retained = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropProbe {
                account: account
                    .share_after_test_admission()
                    .unwrap_or_else(|_| panic!("test account share must be admitted")),
            })
        })
        .unwrap();
    let charged = retained.charged_bytes();

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
    drop(retained);
    assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), charged);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn account_identity_is_opaque_and_foreign_accounts_do_not_match() {
    let factory = account_factory(2);
    let first_account = account(&factory, 4_096, 8);
    let second_account = account(&factory, 4_096, 8);
    let retained = first_account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
        .unwrap();

    assert!(retained.belongs_to(&first_account));
    assert!(!retained.belongs_to(&second_account));
    let shared_first = first_account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    assert!(first_account.is_same_account(&shared_first));
    assert!(!first_account.is_same_account(&second_account));
}

#[test]
fn foreign_account_cannot_authorize_retained_attribution_release() {
    let factory = account_factory(2);
    let first_account = account(&factory, 4_096, 8);
    let second_account = account(&factory, 4_096, 8);
    let retained = first_account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
        .unwrap_or_else(|_| panic!("foreign-account probe allocation must retain"));
    let first_before = first_account.snapshot();

    assert!(matches!(
        drop_retained_allocation_before_attribution(retained, &second_account),
        RetainedStorageDropOutcome::ForeignOwnershipFault(_)
    ));
    assert_eq!(
        first_account.snapshot().live_retained_bytes(),
        first_before.live_retained_bytes()
    );
    assert_eq!(
        first_account.snapshot().live_retained_allocations(),
        first_before.live_retained_allocations()
    );
    assert_eq!(second_account.snapshot().live_retained_bytes(), 0);
    drop(first_account);
    drop(second_account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn concurrent_exact_limit_admission_is_linearizable() {
    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer, 1);
    let start = StdArc::new(Barrier::new(3));
    let attempts_complete = StdArc::new(Barrier::new(3));
    let release_owner = StdArc::new(Barrier::new(2));
    let successes = StdArc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    for value in [1_u64, 2_u64] {
        let account = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        let start = StdArc::clone(&start);
        let attempts_complete = StdArc::clone(&attempts_complete);
        let release_owner = StdArc::clone(&release_owner);
        let successes = StdArc::clone(&successes);
        threads.push(thread::spawn(move || {
            start.wait();
            let retained = account
                .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(value))
                .ok();
            if retained.is_some() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempts_complete.wait();
            if let Some(retained) = retained {
                release_owner.wait();
                drop(retained);
            }
        }));
    }

    start.wait();
    attempts_complete.wait();
    assert_eq!(successes.load(Ordering::SeqCst), 1);
    release_owner.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
