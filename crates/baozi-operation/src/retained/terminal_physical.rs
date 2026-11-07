//! Actual terminal payload owners, independent of the numerical evidence representation.

use super::*;

/// Short access to the original slots; dropping this view never disposes their owners.
pub(super) struct TerminalPhysicalSlots<'loan, P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: Option<&'loan mut Option<DetachedRetainedPart<F>>>,
    pub(super) evidence:
        Option<&'loan mut Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>>,
    pub(super) canonical_table: Option<&'loan mut Option<RetainedTableAllocation<CanonicalBlock>>>,
    pub(super) evidence_table: Option<&'loan mut Option<RetainedTableAllocation<EvidenceBlock>>>,
    pub(super) account: Option<&'loan mut Option<PhysicalStorageAccount>>,
}

pub(super) struct TerminalPhysicalRefs<'loan, P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: Option<&'loan DetachedRetainedPart<F>>,
    pub(super) evidence: Option<&'loan DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    pub(super) canonical_table: Option<&'loan RetainedTableAllocation<CanonicalBlock>>,
    pub(super) evidence_table: Option<&'loan RetainedTableAllocation<EvidenceBlock>>,
    pub(super) account: Option<&'loan PhysicalStorageAccount>,
}

impl<'loan, P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalRefs<'loan, P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn first_account(&self) -> Option<&'loan PhysicalStorageAccount> {
        self.account
            .or_else(|| self.fallback.and_then(DetachedRetainedPart::account_anchor))
            .or_else(|| self.evidence.and_then(DetachedRetainedPart::account_anchor))
            .or_else(|| self.canonical_table.map(|table| table.account()))
            .or_else(|| {
                self.canonical_table.and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| self.evidence_table.map(|table| table.account()))
            .or_else(|| {
                self.evidence_table.and_then(|table| {
                    table
                        .as_ref()
                        .members
                        .first()
                        .map(|member| member.allocation.account())
                })
            })
            .or_else(|| {
                self.fallback
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
            .or_else(|| {
                self.evidence
                    .and_then(DetachedRetainedPart::allocation_anchor)
            })
    }

    pub(super) fn record_new_fault(
        &self,
        reason: AccountPoisonReason,
    ) -> AccountPoisonEventWitness {
        let account = self
            .first_account()
            .unwrap_or_else(|| unreachable_final_owner_state());
        record_account_poison_event(account, reason)
    }

    fn record_edges(&self) {
        if let Some(canonical) = self.canonical_table {
            record_forgotten_table_edges(canonical);
        }
        if let Some(evidence) = self.evidence_table {
            record_forgotten_table_edges(evidence);
        }
        if let Some(fallback) = self.fallback {
            fallback.record_forgotten_physical_edge();
        }
        if let Some(evidence) = self.evidence {
            evidence.record_forgotten_physical_edge();
        }
    }

    pub(super) fn record_forgotten_physical_edges(
        &self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let account = self
            .first_account()
            .unwrap_or_else(|| unreachable_final_owner_state());
        self.record_edges();
        FinalizationContainmentError::new(AccountPoisoned::new(account.snapshot()), reason)
    }
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalSlots<'_, P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn as_read(&self) -> TerminalPhysicalRefs<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalRefs {
            fallback: self.fallback.as_deref().and_then(Option::as_ref),
            evidence: self.evidence.as_deref().and_then(Option::as_ref),
            canonical_table: self.canonical_table.as_deref().and_then(Option::as_ref),
            evidence_table: self.evidence_table.as_deref().and_then(Option::as_ref),
            account: self.account.as_deref().and_then(Option::as_ref),
        }
    }

    pub(super) fn fail_closed_after_recorded_poison(&mut self) {
        if let Some(canonical) = self.canonical_table.as_deref_mut().and_then(Option::take) {
            std::mem::forget(canonical);
        }
        if let Some(evidence) = self.evidence_table.as_deref_mut().and_then(Option::take) {
            std::mem::forget(evidence);
        }
        drop(self.account.as_deref_mut().and_then(Option::take));
        if let Some(mut fallback) = self.fallback.as_deref_mut().and_then(Option::take) {
            fallback.fail_closed_after_recorded_poison();
        }
        if let Some(mut evidence) = self.evidence.as_deref_mut().and_then(Option::take) {
            evidence.fail_closed_after_recorded_poison();
        }
    }

    pub(super) fn contain_new_fault(
        &mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.as_read().record_new_fault(reason);
        self.contain_after_recorded(reason)
    }

    pub(super) fn contain_after_recorded(
        &mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        let cause = self.as_read().record_forgotten_physical_edges(reason);
        self.fail_closed_after_recorded_poison();
        cause
    }

    pub(super) fn quarantine_unresolved(&mut self) {
        let parts = self.as_read();
        if parts.first_account().is_some() {
            parts.record_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            let _ = parts
                .record_forgotten_physical_edges(AccountPoisonReason::OwnershipGraphUnverified);
        } else {
            parts.record_edges();
        }
        self.fail_closed_after_recorded_poison();
    }
}

pub(super) struct TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock> {
    pub(super) fallback: Option<DetachedRetainedPart<F>>,
    pub(super) canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
    pub(super) evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
    pub(super) account: Option<PhysicalStorageAccount>,
}

impl<F, CanonicalBlock, EvidenceBlock> TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock> {
    pub(super) fn is_empty(&self) -> bool {
        self.fallback.is_none()
            && self.canonical_table.is_none()
            && self.evidence_table.is_none()
            && self.account.is_none()
    }

    fn slots<'loan, P, D>(
        &'loan mut self,
        evidence: Option<&'loan mut Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>>,
    ) -> TerminalPhysicalSlots<'loan, P, D, F, CanonicalBlock, EvidenceBlock> {
        TerminalPhysicalSlots {
            fallback: Some(&mut self.fallback),
            evidence,
            canonical_table: Some(&mut self.canonical_table),
            evidence_table: Some(&mut self.evidence_table),
            account: Some(&mut self.account),
        }
    }
}

impl<F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        if self.is_empty() {
            return;
        }
        self.slots::<(), ()>(None).quarantine_unresolved();
    }
}

/// Finite numerical facts live in the invocation's fixed staging, not in a physical allocation.
pub(super) struct FiniteTerminalPhysicalState<F, CanonicalBlock, EvidenceBlock> {
    pub(super) graph: TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock>,
    pub(super) disposition: Option<TerminalDisposition>,
}

impl<F, CanonicalBlock, EvidenceBlock>
    FiniteTerminalPhysicalState<F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn new(
        fallback: DetachedRetainedPart<F>,
        canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
        evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
        account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            graph: TerminalPhysicalGraph {
                fallback: Some(fallback),
                canonical_table,
                evidence_table,
                account: Some(account),
            },
            disposition: None,
        }
    }

    pub(super) fn contain_new_fault(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.graph.slots::<(), ()>(None).contain_new_fault(reason)
    }

    pub(super) fn contain_after_quarantine(
        mut self,
        cause: AccountPoisoned,
    ) -> FinalizationContainmentError {
        let reason = cause.snapshot().first_poison_evidence().map_or(
            AccountPoisonReason::OwnershipGraphUnverified,
            AccountPoisonEvidence::reason,
        );
        self.graph
            .slots::<(), ()>(None)
            .contain_after_recorded(reason)
    }
}

/// Dynamic terminal state adds its real numerical evidence allocation to the physical graph.
/// The same owner remains in the root during loans and moves intact after detach.
pub(super) struct TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock> {
    pub(super) graph: TerminalPhysicalGraph<F, CanonicalBlock, EvidenceBlock>,
    pub(super) evidence: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>,
    pub(super) qualification: Option<TerminalEvidenceFreezeQualification>,
    pub(super) disposition: Option<TerminalDisposition>,
}

impl<P, D, F, CanonicalBlock, EvidenceBlock>
    TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>
{
    pub(super) fn new(
        fallback: DetachedRetainedPart<F>,
        evidence: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>,
        canonical_table: Option<RetainedTableAllocation<CanonicalBlock>>,
        evidence_table: Option<RetainedTableAllocation<EvidenceBlock>>,
        account: Option<PhysicalStorageAccount>,
    ) -> Self {
        Self {
            graph: TerminalPhysicalGraph {
                fallback: Some(fallback),
                canonical_table,
                evidence_table,
                account,
            },
            evidence: Some(evidence),
            qualification: None,
            disposition: None,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.graph.is_empty() && self.evidence.is_none()
    }

    fn slots(&mut self) -> TerminalPhysicalSlots<'_, P, D, F, CanonicalBlock, EvidenceBlock> {
        self.graph.slots(Some(&mut self.evidence))
    }

    pub(super) fn contain_new_fault(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.slots().contain_new_fault(reason)
    }

    pub(super) fn contain_after_recorded(
        mut self,
        reason: AccountPoisonReason,
    ) -> FinalizationContainmentError {
        self.slots().contain_after_recorded(reason)
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
}

impl<P, D, F, CanonicalBlock, EvidenceBlock> Drop
    for TerminalPhysicalState<P, D, F, CanonicalBlock, EvidenceBlock>
{
    fn drop(&mut self) {
        self.slots().quarantine_unresolved();
    }
}
