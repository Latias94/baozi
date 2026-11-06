use super::*;
use std::ops::{Deref, DerefMut};

pub(super) struct TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: Option<DetachedRetainedPart<F>>,
    pub(super) evidence: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    pub(super) canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    pub(super) evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    pub(super) account: Option<PhysicalStorageAccount>,
}

pub(super) struct TerminalPartLoan<'inv, T> {
    pub(super) owner: &'inv mut Option<DetachedRetainedPart<T>>,
}

impl<T> Deref for TerminalPartLoan<'_, T> {
    type Target = DetachedRetainedPart<T>;

    fn deref(&self) -> &Self::Target {
        self.owner
            .as_ref()
            .expect("terminal physical part remains in its root")
    }
}

impl<T> DerefMut for TerminalPartLoan<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.owner
            .as_mut()
            .expect("terminal physical part remains in its root")
    }
}

impl<T> TerminalPartLoan<'_, T> {
    pub(super) fn into_owned(self) -> DetachedRetainedPart<T> {
        self.owner
            .take()
            .expect("detach consumes the original physical part once")
    }
}

pub(super) struct TerminalTableLoan<'inv, Block> {
    pub(super) owner: &'inv mut Option<RetainedTableAllocation<Block>>,
}

impl<Block> Deref for TerminalTableLoan<'_, Block> {
    type Target = RetainedTableAllocation<Block>;

    fn deref(&self) -> &Self::Target {
        self.owner
            .as_ref()
            .expect("terminal table remains in its root")
    }
}

impl<Block> DerefMut for TerminalTableLoan<'_, Block> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.owner
            .as_mut()
            .expect("terminal table remains in its root")
    }
}

impl<Block> TerminalTableLoan<'_, Block> {
    pub(super) fn into_owned(self) -> RetainedTableAllocation<Block> {
        self.owner
            .take()
            .expect("physical cleanup or detach consumes the original table once")
    }

    pub(super) fn retain_fail_closed(self) {
        std::mem::forget(self.owner.take());
    }
}

pub(super) struct TerminalAccountLoan<'inv> {
    pub(super) owner: &'inv mut Option<PhysicalStorageAccount>,
}

impl Deref for TerminalAccountLoan<'_> {
    type Target = PhysicalStorageAccount;

    fn deref(&self) -> &Self::Target {
        self.owner
            .as_ref()
            .expect("terminal account anchor remains in its root")
    }
}

impl TerminalAccountLoan<'_> {
    pub(super) fn into_owned(self) -> PhysicalStorageAccount {
        self.owner
            .take()
            .expect("detach consumes the original account anchor once")
    }

    pub(super) fn retire(self) {
        drop(self.owner.take());
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn borrow(
        &mut self,
    ) -> BorrowedTerminalPhysicalOwnershipManifest<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        BorrowedTerminalPhysicalOwnershipManifest {
            fallback: self
                .fallback
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalPartLoan {
                    owner: &mut self.fallback,
                }),
            evidence: self
                .evidence
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalPartLoan {
                    owner: &mut self.evidence,
                }),
            canonical_table: self.canonical_table.as_ref().map(|_| ()).map(|()| {
                TerminalTableLoan {
                    owner: &mut self.canonical_table,
                }
            }),
            evidence_table: self
                .evidence_table
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalTableLoan {
                    owner: &mut self.evidence_table,
                }),
            account: self
                .account
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalAccountLoan {
                    owner: &mut self.account,
                }),
        }
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        if self.fallback.is_some()
            || self.evidence.is_some()
            || self.canonical_table.is_some()
            || self.evidence_table.is_some()
            || self.account.is_some()
        {
            drop(TerminalPhysicalOwnershipManifest::new(
                self.fallback
                    .take()
                    .unwrap_or_else(DetachedRetainedPart::empty),
                self.evidence
                    .take()
                    .unwrap_or_else(DetachedRetainedPart::empty),
                self.canonical_table.take(),
                self.evidence_table.take(),
                self.account.take(),
            ));
        }
    }
}

pub(super) struct BorrowedTerminalPhysicalOwnershipManifest<
    'inv,
    P,
    D,
    F,
    CanonicalBlock,
    EvidenceBlock,
> {
    pub(super) fallback: Option<TerminalPartLoan<'inv, F>>,
    pub(super) evidence: Option<TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>>,
    pub(super) canonical_table: Option<TerminalTableLoan<'inv, CanonicalBlock>>,
    pub(super) evidence_table: Option<TerminalTableLoan<'inv, EvidenceBlock>>,
    pub(super) account: Option<TerminalAccountLoan<'inv>>,
}

pub(super) struct BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: TerminalPartLoan<'inv, F>,
    pub(super) evidence: TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>,
    pub(super) canonical_table: TerminalTableLoan<'inv, CanonicalBlock>,
    pub(super) evidence_table: TerminalTableLoan<'inv, EvidenceBlock>,
    pub(super) account: TerminalAccountLoan<'inv>,
    pub(super) canonical_root: RetainedSlotIdentity,
    pub(super) evidence_root: RetainedSlotIdentity,
}

type BorrowedTerminalPhysicalParts<'inv, P, D, F, CanonicalBlock, EvidenceBlock> = (
    TerminalPartLoan<'inv, F>,
    TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>,
    Option<TerminalTableLoan<'inv, CanonicalBlock>>,
    Option<TerminalTableLoan<'inv, EvidenceBlock>>,
    Option<TerminalAccountLoan<'inv>>,
);

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn from_owned(
        slot: &'inv mut Option<TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>>,
        fallback: DetachedRetainedPart<F>,
        evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
        evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
        account: Option<PhysicalStorageAccount>,
    ) -> Self {
        assert!(
            slot.is_none(),
            "one physical terminal transition per invocation"
        );
        *slot = Some(TerminalPhysicalState {
            fallback: Some(fallback),
            evidence: Some(evidence),
            canonical_table,
            evidence_table,
            account,
        });
        slot.as_mut()
            .expect("the root owns the actual terminal physical graph")
            .borrow()
    }
}

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn new(
        fallback: impl Into<Option<TerminalPartLoan<'inv, F>>>,
        evidence: TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>,
        canonical_table: Option<TerminalTableLoan<'inv, CanonicalBlock>>,
        evidence_table: Option<TerminalTableLoan<'inv, EvidenceBlock>>,
        account: Option<TerminalAccountLoan<'inv>>,
    ) -> Self {
        Self {
            fallback: fallback.into(),
            evidence: Some(evidence),
            canonical_table,
            evidence_table,
            account,
        }
    }

    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        match self.account.as_ref() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn evidence(&self) -> &DetachedRetainedPart<TerminalLedgerEvidence<P, D>> {
        match self.evidence.as_ref() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account
            .as_deref()
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|part| part.account_anchor())
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.account_anchor())
            })
            .or_else(|| self.canonical_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.canonical_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| self.evidence_table.as_ref().map(|table| table.account()))
            .or_else(|| {
                self.evidence_table.as_ref().and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|part| part.allocation_anchor())
            })
            .or_else(|| {
                self.evidence
                    .as_ref()
                    .and_then(|part| part.allocation_anchor())
            })
    }

    pub(super) fn has_exact_fallback_shape(&self) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self
                .account
                .as_ref()
                .is_none_or(|account| account.is_same_account(evidence.account()))
    }

    pub(super) fn record_forgotten_physical_edges(
        &self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let account = match self.first_account() {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        if let Some(canonical) = self.canonical_table.as_ref() {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.evidence_table.as_ref() {
            record_forgotten_table_edges(evidence);
        }
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence.as_ref() {
            evidence.record_forgotten_physical_edge();
        }
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }

    pub(super) fn record_new_fault(
        &self,
        reason: AccountPoisonReason,
    ) -> AccountPoisonEventWitness {
        let account = match self.account.as_deref().or_else(|| self.first_account()) {
            Some(account) => account,
            None => unreachable_final_owner_state(),
        };
        record_account_poison_event(account, reason)
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(canonical) = self.canonical_table.take() {
            canonical.retain_fail_closed();
        }
        if let Some(evidence) = self.evidence_table.take() {
            evidence.retain_fail_closed();
        }
        if let Some(account) = self.account.take() {
            account.retire();
        }
        if let Some(mut fallback) = self.fallback.take() {
            fallback.fail_closed_after_recorded_poison();
        }
        if let Some(mut evidence) = self.evidence.take() {
            evidence.fail_closed_after_recorded_poison();
        }
    }

    pub(super) fn contain_new_fault(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.record_new_fault(reason);
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    pub(super) fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let cause = self.record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    pub(super) fn contain_after_quarantine(
        self,
        cause: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let reason = cause.snapshot().first_poison_evidence().map_or(
            AccountPoisonReason::OwnershipGraphUnverified,
            AccountPoisonEvidence::reason,
        );
        self.contain_after_recorded(reason)
    }

    pub(super) fn into_parts(
        mut self,
    ) -> BorrowedTerminalPhysicalParts<'inv, P, D, F, CanonicalBlock, EvidenceBlock> {
        let fallback = match self.fallback.take() {
            Some(fallback) => fallback,
            None => unreachable_final_owner_state(),
        };
        let evidence = match self.evidence.take() {
            Some(evidence) => evidence,
            None => unreachable_final_owner_state(),
        };
        (
            fallback,
            evidence,
            self.canonical_table.take(),
            self.evidence_table.take(),
            self.account.take(),
        )
    }

    pub(super) fn qualifies_pre_discard_success(&self) -> bool {
        let (
            Some(fallback),
            Some(evidence),
            Some(canonical),
            Some(outcome_evidence),
            Some(account),
        ) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.canonical_table.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        )
        else {
            return false;
        };
        fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && canonical.belongs_to(account)
            && outcome_evidence.belongs_to(account)
    }

    pub(super) fn qualifies_pre_cleanup_rich(&self, expect_canonical: bool) -> bool {
        let (Some(fallback), Some(evidence), Some(evidence_table), Some(account)) = (
            self.fallback.as_ref(),
            self.evidence.as_ref(),
            self.evidence_table.as_ref(),
            self.account.as_ref(),
        ) else {
            return false;
        };
        self.canonical_table.is_some() == expect_canonical
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(account)
            && evidence.account().is_same_account(account)
            && evidence_table.belongs_to(account)
            && self
                .canonical_table
                .as_ref()
                .is_none_or(|table| table.belongs_to(account))
    }

    pub(super) fn qualifies_pre_cleanup_fallback(
        &self,
        expect_canonical: bool,
        expect_evidence: bool,
    ) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        let table_expected = expect_canonical || expect_evidence;
        self.canonical_table.is_some() == expect_canonical
            && self.evidence_table.is_some() == expect_evidence
            && self.account.is_some() == table_expected
            && fallback.has_exact_owner()
            && evidence.has_exact_owner()
            && fallback.account().is_same_account(evidence.account())
            && self.account.as_ref().is_none_or(|account| {
                fallback.account().is_same_account(account)
                    && self
                        .canonical_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
                    && self
                        .evidence_table
                        .as_ref()
                        .is_none_or(|table| table.belongs_to(account))
            })
    }

    pub(super) fn finish_pre_discard_success(
        mut self,
        canonical_root: RetainedSlotIdentity,
        evidence_root: RetainedSlotIdentity,
    ) -> BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock> {
        BorrowedExactPreDiscardSuccess {
            fallback: self
                .fallback
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence: self
                .evidence
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_table: self
                .canonical_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            evidence_table: self
                .evidence_table
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            account: self
                .account
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state()),
            canonical_root,
            evidence_root,
        }
    }
}

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        if self.first_account().is_none() {
            return;
        }
        self.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
        let _ = self.record_forgotten_physical_edges(AccountPoisonReason::OwnershipGraphUnverified);
        self.fail_closed_after_recorded_poison();
    }
}

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }

    pub(super) fn into_terminal(
        self,
    ) -> BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    {
        BorrowedTerminalPhysicalOwnershipManifest::new(
            self.fallback,
            self.evidence,
            Some(self.canonical_table),
            Some(self.evidence_table),
            Some(self.account),
        )
    }
}
