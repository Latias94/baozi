use super::ownership::FallbackPublicationBaselineLoan;
use super::terminal_physical::{
    FiniteTerminalPhysicalState, TerminalPhysicalGraph, TerminalPhysicalRefs, TerminalPhysicalSlots,
};
use super::*;
use std::ops::{Deref, DerefMut};

/// A short loan of the original concrete terminal slot, never an owning backend union.
pub(in crate::retained) enum TerminalPhysicalSlot<'inv, P, D, F, CB, EB> {
    Dynamic(&'inv mut Option<TerminalPhysicalState<P, D, F, CB, EB>>),
    Finite(&'inv mut Option<FiniteTerminalPhysicalState<F, CB, EB>>),
}

type TerminalInstallation<'inv, O, P, D, F, CB, EB> = (
    BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CB, EB>,
    crate::ledger::FallbackBatchView<'inv, O, P, D>,
    PrimaryPublicationBatches<'inv, O, P, D>,
);

impl<'inv, P, D, F, CB, EB> TerminalPhysicalSlot<'inv, P, D, F, CB, EB> {
    pub(super) const fn is_finite(&self) -> bool {
        matches!(self, Self::Finite(_))
    }

    /// All physical owners move into their actual terminal slot before qualification lends them.
    pub(super) fn install<O>(
        self,
        baseline: FallbackPublicationBaselineLoan<'inv, O, P, D, F>,
        assembly: Option<&'inv mut PublicationAssemblyFields<O, P, D, CB, EB>>,
    ) -> TerminalInstallation<'inv, O, P, D, F, CB, EB>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        assert_eq!(
            self.is_finite(),
            baseline.is_finite(),
            "the original terminal and baseline backends must match"
        );
        assert!(
            baseline.can_take_physical(),
            "the original baseline must retain all physical owners and batches"
        );
        assert!(
            match &self {
                Self::Dynamic(slot) => slot.is_none(),
                Self::Finite(slot) => slot.is_none(),
            },
            "one physical terminal transition per invocation"
        );
        if let Some(fields) = &assembly {
            assert!(
                fields.account.is_some(),
                "the original assembly retains its account"
            );
        }

        let (canonical_table, evidence_table, assembly_account, primary) = match assembly {
            Some(fields) => (
                fields.canonical_table.take(),
                fields.evidence_table.take(),
                fields.account.take(),
                PrimaryPublicationBatches::new(
                    Some(&mut fields.canonical_batch),
                    Some(&mut fields.evidence_batch),
                ),
            ),
            None => (None, None, None, PrimaryPublicationBatches::new(None, None)),
        };
        match (self, baseline) {
            (Self::Dynamic(slot), FallbackPublicationBaselineLoan::Dynamic(baseline)) => {
                let (fallback, evidence, batches) = baseline.take_physical_and_lend_batches();
                *slot = Some(TerminalPhysicalState::new(
                    fallback,
                    evidence,
                    canonical_table,
                    evidence_table,
                    assembly_account,
                ));
                let physical = slot
                    .as_mut()
                    .expect("the actual dynamic slot was installed")
                    .borrow();
                let batches = batches
                    .as_mut()
                    .expect("the original dynamic batch owner was checked")
                    .view();
                (physical, batches, primary)
            }
            (Self::Finite(slot), FallbackPublicationBaselineLoan::Finite(baseline)) => {
                let (fallback, account, batches) = baseline.take_physical_and_lend_batches();
                *slot = Some(FiniteTerminalPhysicalState::new(
                    fallback,
                    canonical_table,
                    evidence_table,
                    account,
                ));
                // The finite terminal keeps its original independent anchor; the assembly's
                // additional admitted share can retire after that original owner is installed.
                drop(assembly_account);
                let physical = slot
                    .as_mut()
                    .expect("the actual finite slot was installed")
                    .borrow();
                let batches = batches
                    .as_mut()
                    .expect("the original finite batch owner was checked")
                    .view();
                (physical, batches, primary)
            }
            _ => unreachable!("the original backend pair was checked before any owner moved"),
        }
    }
}

pub(super) struct TerminalPartLoan<'inv, T> {
    pub(super) owner: &'inv mut Option<DetachedRetainedPart<T>>,
}

pub(super) enum BorrowedTerminalEvidence<'inv, P, D> {
    Dynamic(TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>),
    Finite,
}

pub(super) enum TerminalEvidenceQualification {
    Dynamic(TerminalEvidenceFreezeQualification),
    Finite,
}

impl TerminalEvidenceQualification {
    pub(super) const fn is_finite(&self) -> bool {
        matches!(self, Self::Finite)
    }
}

impl<'inv, P, D> From<TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>>
    for BorrowedTerminalEvidence<'inv, P, D>
{
    fn from(owner: TerminalPartLoan<'inv, TerminalLedgerEvidence<P, D>>) -> Self {
        Self::Dynamic(owner)
    }
}

impl<P, D> BorrowedTerminalEvidence<'_, P, D> {
    pub(super) const fn is_finite(&self) -> bool {
        matches!(self, Self::Finite)
    }

    pub(super) fn dynamic(&self) -> Option<&DetachedRetainedPart<TerminalLedgerEvidence<P, D>>> {
        match self {
            Self::Dynamic(owner) => Some(owner),
            Self::Finite => None,
        }
    }

    fn account(&self) -> Option<&PhysicalStorageAccount> {
        self.dynamic()
            .and_then(DetachedRetainedPart::account_anchor)
    }

    fn qualifies_for_account(&self, account: &PhysicalStorageAccount) -> bool {
        match self {
            Self::Dynamic(owner) => {
                owner.has_exact_owner() && owner.account().is_same_account(account)
            }
            Self::Finite => true,
        }
    }
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
        let evidence = self.evidence.as_ref().map(|_| ()).map(|()| {
            BorrowedTerminalEvidence::Dynamic(TerminalPartLoan {
                owner: &mut self.evidence,
            })
        });
        BorrowedTerminalPhysicalOwnershipManifest::from_graph(&mut self.graph, evidence)
    }
}

impl<F, CanonicalBlock, EvidenceBlock>
    FiniteTerminalPhysicalState<F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn borrow<P, D>(
        &mut self,
    ) -> BorrowedTerminalPhysicalOwnershipManifest<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        BorrowedTerminalPhysicalOwnershipManifest::from_graph(
            &mut self.graph,
            Some(BorrowedTerminalEvidence::Finite),
        )
    }
}

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn from_graph(
        graph: &'inv mut TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock>,
        evidence: Option<BorrowedTerminalEvidence<'inv, P, D>>,
    ) -> Self {
        Self {
            fallback: graph
                .fallback
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalPartLoan {
                    owner: &mut graph.fallback,
                }),
            evidence,
            canonical_table: graph.canonical_table.as_ref().map(|_| ()).map(|()| {
                TerminalTableLoan {
                    owner: &mut graph.canonical_table,
                }
            }),
            evidence_table: graph
                .evidence_table
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalTableLoan {
                    owner: &mut graph.evidence_table,
                }),
            account: graph
                .account
                .as_ref()
                .map(|_| ())
                .map(|()| TerminalAccountLoan {
                    owner: &mut graph.account,
                }),
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
    pub(super) evidence: Option<BorrowedTerminalEvidence<'inv, P, D>>,
    pub(super) canonical_table: Option<TerminalTableLoan<'inv, CanonicalBlock>>,
    pub(super) evidence_table: Option<TerminalTableLoan<'inv, EvidenceBlock>>,
    pub(super) account: Option<TerminalAccountLoan<'inv>>,
}

pub(super) struct BorrowedExactPreDiscardSuccess<'inv, P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: TerminalPartLoan<'inv, F>,
    pub(super) evidence: BorrowedTerminalEvidence<'inv, P, D>,
    pub(super) canonical_table: TerminalTableLoan<'inv, CanonicalBlock>,
    pub(super) evidence_table: TerminalTableLoan<'inv, EvidenceBlock>,
    pub(super) account: TerminalAccountLoan<'inv>,
    pub(super) canonical_root: RetainedSlotIdentity,
    pub(super) evidence_root: RetainedSlotIdentity,
}

type BorrowedTerminalPhysicalParts<'inv, P, D, F, CanonicalBlock, EvidenceBlock> = (
    TerminalPartLoan<'inv, F>,
    BorrowedTerminalEvidence<'inv, P, D>,
    Option<TerminalTableLoan<'inv, CanonicalBlock>>,
    Option<TerminalTableLoan<'inv, EvidenceBlock>>,
    Option<TerminalAccountLoan<'inv>>,
);

impl<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
    BorrowedTerminalPhysicalOwnershipManifest<'inv, P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn new(
        fallback: impl Into<Option<TerminalPartLoan<'inv, F>>>,
        evidence: impl Into<BorrowedTerminalEvidence<'inv, P, D>>,
        canonical_table: Option<TerminalTableLoan<'inv, CanonicalBlock>>,
        evidence_table: Option<TerminalTableLoan<'inv, EvidenceBlock>>,
        account: Option<TerminalAccountLoan<'inv>>,
    ) -> Self {
        Self {
            fallback: fallback.into(),
            evidence: Some(evidence.into()),
            canonical_table,
            evidence_table,
            account,
        }
    }

    pub(super) fn terminal_account(&self) -> Option<&PhysicalStorageAccount> {
        self.account.as_deref().or_else(|| {
            self.evidence
                .as_ref()
                .and_then(BorrowedTerminalEvidence::account)
        })
    }

    pub(super) fn matches_backend(&self, is_finite: bool) -> bool {
        self.evidence.as_ref().is_some_and(|evidence| {
            evidence.is_finite() == is_finite && (!is_finite || self.account.is_some())
        })
    }

    pub(super) fn first_account(&self) -> Option<&PhysicalStorageAccount> {
        self.as_read().first_account()
    }

    fn as_read(&self) -> TerminalPhysicalRefs<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalRefs {
            fallback: self.fallback.as_deref(),
            evidence: self
                .evidence
                .as_ref()
                .and_then(BorrowedTerminalEvidence::dynamic),
            canonical_table: self.canonical_table.as_deref(),
            evidence_table: self.evidence_table.as_deref(),
            account: self.account.as_deref(),
        }
    }

    fn slots(&mut self) -> TerminalPhysicalSlots<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalSlots {
            fallback: self.fallback.as_mut().map(|part| &mut *part.owner),
            evidence: self.evidence.as_mut().and_then(|evidence| match evidence {
                BorrowedTerminalEvidence::Dynamic(part) => Some(&mut *part.owner),
                BorrowedTerminalEvidence::Finite => None,
            }),
            canonical_table: self.canonical_table.as_mut().map(|table| &mut *table.owner),
            evidence_table: self.evidence_table.as_mut().map(|table| &mut *table.owner),
            account: self.account.as_mut().map(|account| &mut *account.owner),
        }
    }

    pub(super) fn has_exact_fallback_shape(&self) -> bool {
        let (Some(fallback), Some(evidence)) = (self.fallback.as_ref(), self.evidence.as_ref())
        else {
            return false;
        };
        let Some(account) = self.terminal_account() else {
            return false;
        };
        self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && fallback.has_exact_owner()
            && evidence.qualifies_for_account(account)
            && fallback.account().is_same_account(account)
    }

    pub(super) fn record_forgotten_physical_edges(
        &self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.as_read().record_forgotten_physical_edges(reason)
    }

    pub(super) fn record_new_fault(
        &self,
        reason: AccountPoisonReason,
    ) -> AccountPoisonEventWitness {
        self.as_read().record_new_fault(reason)
    }

    fn fail_closed_after_recorded_poison(&mut self) {
        self.slots().fail_closed_after_recorded_poison();
        self.fallback = None;
        self.evidence = None;
        self.canonical_table = None;
        self.evidence_table = None;
        self.account = None;
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
            && evidence.qualifies_for_account(account)
            && fallback.account().is_same_account(account)
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
            && evidence.qualifies_for_account(account)
            && fallback.account().is_same_account(account)
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
        let Some(account) = self.terminal_account() else {
            return false;
        };
        self.canonical_table.is_some() == expect_canonical
            && self.evidence_table.is_some() == expect_evidence
            && self.account.is_some() == (table_expected || evidence.is_finite())
            && fallback.has_exact_owner()
            && evidence.qualifies_for_account(account)
            && fallback.account().is_same_account(account)
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
