//! Genuine finite fallback ownership, without a physical numerical-evidence part.

use super::*;
use crate::ledger::{FiniteFallbackBatchBaseline, LedgerReadView};

pub(in crate::retained) struct FiniteFallbackPublicationBaseline<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fallback: Option<DetachedRetainedPart<F>>,
    account: Option<PhysicalStorageAccount>,
    batches: Option<FiniteFallbackBatchBaseline<O, P, D>>,
}

type FiniteFallbackPhysicalLoan<'loan, O, P, D, F> = (
    DetachedRetainedPart<F>,
    PhysicalStorageAccount,
    &'loan mut Option<FiniteFallbackBatchBaseline<O, P, D>>,
);

type FiniteFallbackConstructionFailure<O, P, D, F> = (
    RetainedWriterError<D>,
    RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
    AttributionReservationBatch<O, P, D, CanonicalOutput>,
    AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
    PhysicalStorageAccount,
);

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, F>
    FiniteFallbackPublicationBaseline<O, P, D, F>
{
    /// Rejects before any owner is consumed; the caller receives the original graph.
    pub(in crate::retained) fn try_from_parts_owned(
        fallback: RetainedPart<F, OutcomeEvidenceDestination, O, P, D>,
        canonical_assembly: AttributionReservationBatch<O, P, D, CanonicalOutput>,
        evidence_assembly: AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        account: PhysicalStorageAccount,
    ) -> Result<Self, FiniteFallbackConstructionFailure<O, P, D, F>> {
        if !fallback.has_exact_product_graph() || !fallback.account().is_same_account(&account) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err((
                RetainedWriterError::StorageDropUnverified,
                fallback,
                canonical_assembly,
                evidence_assembly,
                account,
            ));
        }
        // The original owner was just qualified; moving it into the private exact wrapper
        // performs no callback, allocation or repeated graph validation.
        let exact = ExactRetainedPart { inner: fallback };
        let (fallback_batch, fallback) = exact.into_detached();
        Ok(Self {
            fallback: Some(fallback),
            account: Some(account),
            batches: Some(FiniteFallbackBatchBaseline::new_unbound(
                canonical_assembly,
                evidence_assembly,
                fallback_batch,
            )),
        })
    }

    /// Keeps all original physical and numerical owners installed on normal rejection.
    /// The outer entry transfers this owned error after disposing its body captures.
    pub(in crate::retained) fn try_attach_owned(
        mut self,
        ledger: LedgerReadView<'_, O, P, D>,
    ) -> Result<Self, (RetainedWriterError<D>, Self)>
    where
        D: fmt::Debug,
    {
        let bind_result = catch_unwind(AssertUnwindSafe(|| {
            let batches = self
                .batches
                .as_mut()
                .unwrap_or_else(|| unreachable_final_owner_state());
            ledger.try_attach_fallback_rows(batches.view())
        }));
        match bind_result {
            Err(payload) => {
                if let Some(account) = self.first_account() {
                    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                }
                discard_untrusted_panic_payload(payload);
                Err((RetainedWriterError::StorageDropUnverified, self))
            }
            Ok(Err(_cause)) => {
                let poisoned = self
                    .account()
                    .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                Err((RetainedWriterError::AccountPoisoned(poisoned), self))
            }
            Ok(Ok(())) => Ok(self),
        }
    }

    /// Scalar observation retained for the original cleanup fixtures, not production entry.
    #[cfg(test)]
    pub(in crate::retained) fn try_attach(
        self,
        ledger: LedgerReadView<'_, O, P, D>,
    ) -> Result<Self, RetainedWriterError<D>>
    where
        D: fmt::Debug,
    {
        self.try_attach_owned(ledger)
            .map_err(|(cause, mut baseline)| {
                if matches!(&cause, RetainedWriterError::StorageDropUnverified) {
                    baseline.contain_after_recorded_poison();
                    cause
                } else if baseline.try_discard_all().is_ok() {
                    cause
                } else {
                    RetainedWriterError::StorageDropUnverified
                }
            })
    }

    pub(in crate::retained) fn account(&self) -> &PhysicalStorageAccount {
        self.account
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    #[cfg(test)]
    pub(in crate::retained) fn test_fallback_allocation(&self) -> &RetainedAllocation<F> {
        self.fallback
            .as_ref()
            .expect("the original finite fallback remains installed")
            .allocation()
    }

    pub(in crate::retained) fn with_batches<R>(
        &self,
        inspect: impl FnOnce(&FiniteFallbackBatchBaseline<O, P, D>) -> R,
    ) -> Option<R> {
        self.batches.as_ref().map(inspect)
    }

    pub(in crate::retained) fn batch_read(
        &self,
    ) -> Option<crate::ledger::FallbackBatchReadView<'_, O, P, D>> {
        self.batches
            .as_ref()
            .map(FiniteFallbackBatchBaseline::as_read)
    }

    pub(in crate::retained) fn terminal_batches(
        &mut self,
    ) -> Option<crate::ledger::FallbackBatchView<'_, O, P, D>> {
        self.batches.as_mut().map(FiniteFallbackBatchBaseline::view)
    }

    pub(super) fn physical_parts_are_installed(&self) -> bool {
        self.fallback.is_some() && self.account.is_some() && self.batches.is_some()
    }

    pub(in crate::retained) fn try_replace_canonical_assembly(
        &mut self,
        replacement: AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> Result<(), RetainedWriterError<D>> {
        if !self.has_exact_product_graph() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedWriterError::StorageDropUnverified);
        }
        let Some(batches) = self.batches.as_mut() else {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedWriterError::StorageDropUnverified);
        };
        match batches.view().replace_empty_canonical_assembly(replacement) {
            Ok(previous) => {
                drop(previous);
                Ok(())
            }
            Err(replacement) => {
                drop(replacement);
                self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
                Err(RetainedWriterError::StorageDropUnverified)
            }
        }
    }

    pub(in crate::retained) fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .as_ref()
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::account_anchor)
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
    }

    pub(in crate::retained) fn take_parts(
        &mut self,
    ) -> (
        DetachedRetainedPart<F>,
        PhysicalStorageAccount,
        FiniteFallbackBatchBaseline<O, P, D>,
    ) {
        (
            self.fallback
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            self.account
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            self.batches
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
        )
    }

    pub(in crate::retained) fn take_physical_and_lend_batches(
        &mut self,
    ) -> FiniteFallbackPhysicalLoan<'_, O, P, D, F> {
        if self.fallback.is_none() || self.account.is_none() || self.batches.is_none() {
            unreachable_final_owner_state();
        }
        let fallback = self
            .fallback
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let account = self
            .account
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        (fallback, account, &mut self.batches)
    }

    fn has_exact_product_graph(&self) -> bool {
        let (Some(fallback), Some(account), Some(_)) =
            (&self.fallback, &self.account, &self.batches)
        else {
            return false;
        };
        fallback.has_exact_owner() && fallback.account().is_same_account(account)
    }

    pub(in crate::retained) fn record_forgotten_physical_edges(&self) {
        if let Some(fallback) = &self.fallback {
            fallback.record_forgotten_physical_edge();
        }
    }

    pub(in crate::retained) fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
        }
        std::mem::forget(self.batches.take());
        drop(self.account.take());
    }

    fn contain_after_recorded_poison(&mut self) {
        self.record_forgotten_physical_edges();
        self.fail_closed_after_recorded_poison();
    }

    fn new_fault_and_fail_closed(&mut self, reason: AccountPoisonReason) {
        if let Some(account) = self.first_account() {
            let _ = account.record_poison(reason);
        }
        self.contain_after_recorded_poison();
    }

    fn try_discard_all(&mut self) -> Result<(), ()> {
        if !self.has_exact_product_graph() {
            self.new_fault_and_fail_closed(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(());
        }
        let (fallback_account, _) =
            match discard_baseline_physical_parts::<F, P, D>(&mut self.fallback, None) {
                Ok(accounts) => accounts,
                Err(failure) => {
                    if failure.cause == RetainedPartCleanupError::MissingFaultAnchor
                        && let Some(account) = failure
                            .released_account
                            .as_ref()
                            .or_else(|| self.first_account())
                    {
                        let _ =
                            account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    }
                    drop(failure.released_account);
                    self.contain_after_recorded_poison();
                    return Err(());
                }
            };
        // Physical destruction is established before retiring the original batch/account cover.
        drop(self.batches.take());
        drop(fallback_account);
        drop(self.account.take());
        drop(self.fallback.take());
        Ok(())
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord, F> Drop
    for FiniteFallbackPublicationBaseline<O, P, D, F>
{
    fn drop(&mut self) {
        if self.fallback.is_none() && self.account.is_none() && self.batches.is_none() {
            return;
        }
        let _ = self.try_discard_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
    use crate::ledger::{
        AttributionBatchPlan, DimensionPolicy, FiniteStartupLedgerPlanParts, OutputCommitment,
        StartupAccountShareEnvelope, StartupRetainedAttribution,
    };
    use crate::retained::startup::FiniteStartupCrossPlanePlan;
    use crate::retained::{
        PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits, RetainedAllocationPlan,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Owner;
    impl OperationOwner for Owner {}

    struct Payload(Arc<AtomicUsize>);

    impl Drop for Payload {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    type Baseline = FiniteFallbackPublicationBaseline<Owner, u8, u8, Payload>;

    fn account() -> PhysicalStorageAccount {
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap();
        factory
            .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 1).unwrap())
            .unwrap()
    }

    fn baseline(
        account: &PhysicalStorageAccount,
        drops: &Arc<AtomicUsize>,
    ) -> (FiniteLedger<Owner, u8, u8>, Baseline) {
        let policies = [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
            let policy = if dimension == 6 {
                DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX)
            } else {
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            };
            (dimension, policy)
        });
        let limits = FiniteLedgerLimits::try_new(u64::MAX, u64::MAX, policies).unwrap();
        let mut ledger = FiniteLedger::<Owner, u8, u8>::try_new(limits, [1, 2, 3]).unwrap();
        let allocation = RetainedAllocationPlan::new(0, 0).unwrap();
        let charge = account
            .checked_retained_charge::<Payload>(allocation)
            .unwrap();
        let parts = FiniteStartupLedgerPlanParts {
            canonical_assembly: AttributionBatchPlan::try_new(1, 1, 2, 1).unwrap(),
            evidence_assembly: AttributionBatchPlan::try_new(2, 2, 2, 1).unwrap(),
            fallback: AttributionBatchPlan::try_new(3, 3, 1, 1).unwrap(),
            fallback_attribution: StartupRetainedAttribution::new(3, 4, charge.bytes),
            account_shares: StartupAccountShareEnvelope::three_identical([(6, 1), (6, 2)]),
            fallback_commitment: OutputCommitment::new(6, 4),
        };
        let ready = account
            .try_admit_operation_start()
            .unwrap()
            .try_prepare_finite_startup_cross_plane(
                &mut ledger.view(),
                FiniteStartupCrossPlanePlan::try_new(parts, charge).unwrap(),
            )
            .unwrap_or_else(|_| panic!("the real finite graph must enter Ready"));
        let baseline = ready
            .into_retained_guard::<Payload>(allocation)
            .try_build(|| Ok(Payload(Arc::clone(drops))))
            .unwrap();
        (ledger, baseline)
    }

    #[test]
    fn owned_attachment_rejection_outlives_original_ledger_without_disposing_physical_members() {
        let account = account();
        let drops = Arc::new(AtomicUsize::new(0));
        let (ledger, baseline) = baseline(&account, &drops);
        let (foreign, foreign_baseline) =
            self::baseline(&self::account(), &Arc::new(AtomicUsize::new(0)));
        let probe = ledger.test_shared_probe().unwrap();
        let payload = std::ptr::from_ref(baseline.test_fallback_allocation().as_ref());
        let cover = (probe.current_live_bytes(), probe.active_reservations());
        let physical = account.snapshot();

        let (cause, failed) = match baseline.try_attach_owned(foreign.as_read()) {
            Ok(_) => panic!("a foreign ledger cannot attach the original finite baseline"),
            Err(failed) => failed,
        };
        assert!(matches!(cause, RetainedWriterError::AccountPoisoned(_)));
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(
            std::ptr::from_ref(failed.test_fallback_allocation().as_ref()),
            payload
        );
        assert!(failed.account().is_same_account(&account));
        assert_eq!(
            (probe.current_live_bytes(), probe.active_reservations()),
            cover
        );
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            physical.live_retained_bytes()
        );
        drop(ledger);
        drop(foreign_baseline);
        drop(foreign);
        assert!(probe.is_available());
        assert_eq!(
            (probe.current_live_bytes(), probe.active_reservations()),
            cover
        );
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        let first_poison = account.snapshot().first_poison_evidence();

        // Exact pre-Ready owners may still dispose verified physical storage after poison.
        drop(failed);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
        assert!(!probe.is_available());
    }

    #[test]
    fn owned_constructor_rejection_returns_original_fallback_and_batch_cover_after_source_ends() {
        let account = account();
        let foreign_account = self::account();
        let drops = Arc::new(AtomicUsize::new(0));
        let (ledger, mut source) = baseline(&account, &drops);
        let probe = ledger.test_shared_probe().unwrap();
        let payload = std::ptr::from_ref(source.test_fallback_allocation().as_ref());
        let cover = (probe.current_live_bytes(), probe.active_reservations());
        let (mut detached, terminal_account, batches) = source.take_parts();
        let (canonical, evidence, fallback_batch) = batches.into_parts();
        let fallback = RetainedPart::from_complete(
            detached.allocation.take().unwrap(),
            fallback_batch,
            detached.account.take().unwrap(),
        );
        let failure =
            match Baseline::try_from_parts_owned(fallback, canonical, evidence, foreign_account) {
                Ok(_) => panic!("a foreign terminal account must reject before owner extraction"),
                Err(failure) => failure,
            };
        drop(source);
        drop(detached);
        drop(terminal_account);
        drop(ledger);

        let (cause, fallback, canonical, evidence, foreign_account) = failure;
        assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(std::ptr::from_ref(fallback.allocation().as_ref()), payload);
        assert!(fallback.account().is_same_account(&account));
        assert!(foreign_account.snapshot().poisoned());
        assert!(!account.snapshot().poisoned());
        assert_eq!(
            (probe.current_live_bytes(), probe.active_reservations()),
            cover
        );
        assert_eq!(account.snapshot().live_retained_allocations(), 1);
        assert_eq!(account.share_call_count(), 3);

        drop(fallback);
        drop(canonical);
        drop(evidence);
        drop(foreign_account);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        assert!(!probe.is_available());
    }
}
