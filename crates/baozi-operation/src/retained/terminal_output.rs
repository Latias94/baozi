#[cfg(test)]
use super::unreachable_final_owner_state;
use super::{
    AccountPoisonReason, DimensionSnapshot, FallbackHeaderSnapshot, PhaseSnapshot,
    PhysicalStorageAccount, PublishedCanonicalAggregate, PublishedOutcomeEvidenceAggregate,
    PublishedPart, RetainedSlotIdentity, TerminalEvidenceCause, TerminalEvidenceKey,
    TerminalLedgerEvidence,
};

pub(super) enum RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F> {
    Success {
        canonical: PublishedCanonicalAggregate<CanonicalBlock>,
        outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        canonical_root: RetainedSlotIdentity,
        outcome_evidence_root: RetainedSlotIdentity,
        account: PhysicalStorageAccount,
    },
    RichFailure {
        outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
        outcome_evidence_root: RetainedSlotIdentity,
        account: PhysicalStorageAccount,
    },
    MinimalFallback {
        part: PublishedPart<F>,
    },
}

/// Prepared terminal publication after one atomic ledger transition.
///
/// Success freezes canonical and evidence destinations, rich failure freezes evidence only, and
/// minimal fallback freezes the pre-admitted fallback/evidence baseline. Handoff is a pure
/// allocation-free ownership move.
#[must_use = "prepared publication owns terminal output and frozen evidence until handoff"]
pub(crate) struct PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
    pub(super) payload: RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact terminal classification consumed before any domain prepared wrapper is constructed.
pub(crate) enum PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, P, D> {
    Success(PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>),
    RichFailure(PreparedRetainedRichFailure<EvidenceBlock, P, D>),
    MinimalFallback(PreparedRetainedFallback<F, P, D>),
}

/// Exact terminal outcome for a requested success that publishes no retained primary output.
pub(crate) enum PreparedRetainedEvidenceOnlyOutcome<F, P, D> {
    Success(PreparedRetainedEvidenceOnlySuccess<P, D>),
    MinimalFallback(PreparedRetainedFallback<F, P, D>),
}

/// Exact prepared success. Its retained roots remain opaque until handoff.
pub(crate) struct PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D> {
    pub(super) canonical: PublishedCanonicalAggregate<CanonicalBlock>,
    pub(super) outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    pub(super) canonical_root: RetainedSlotIdentity,
    pub(super) outcome_evidence_root: RetainedSlotIdentity,
    pub(super) account: PhysicalStorageAccount,
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact prepared rich failure. Its retained evidence remains opaque until handoff.
pub(crate) struct PreparedRetainedRichFailure<EvidenceBlock, P, D> {
    pub(super) outcome_evidence: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    pub(super) outcome_evidence_root: RetainedSlotIdentity,
    pub(super) account: PhysicalStorageAccount,
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact prepared minimal fallback. Its payload remains opaque until handoff.
pub(crate) struct PreparedRetainedFallback<F, P, D> {
    pub(super) part: PublishedPart<F>,
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

/// Exact prepared success for an operation that published no retained primary output.
pub(crate) struct PreparedRetainedEvidenceOnlySuccess<P, D> {
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

impl<CanonicalBlock, EvidenceBlock, F, P, D>
    PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Consumes the unclassified private owner into one exact prepared terminal state.
    pub fn into_outcome(
        self,
    ) -> PreparedRetainedPublicationOutcome<CanonicalBlock, EvidenceBlock, F, P, D> {
        let Self { payload, evidence } = self;
        match payload {
            RetainedPublicationPayload::Success {
                canonical,
                outcome_evidence,
                canonical_root,
                outcome_evidence_root,
                account,
            } => PreparedRetainedPublicationOutcome::Success(PreparedRetainedSuccess {
                canonical,
                outcome_evidence,
                canonical_root,
                outcome_evidence_root,
                account,
                evidence,
            }),
            RetainedPublicationPayload::RichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
            } => PreparedRetainedPublicationOutcome::RichFailure(PreparedRetainedRichFailure {
                outcome_evidence,
                outcome_evidence_root,
                account,
                evidence,
            }),
            RetainedPublicationPayload::MinimalFallback { part } => {
                PreparedRetainedPublicationOutcome::MinimalFallback(PreparedRetainedFallback {
                    part,
                    evidence,
                })
            }
        }
    }
}

/// Canonical owning half ready to move into a domain-owned validated Scene wrapper.
pub(crate) struct PublishedCanonicalOutput<CanonicalBlock> {
    pub(super) aggregate: PublishedCanonicalAggregate<CanonicalBlock>,
    pub(super) root: RetainedSlotIdentity,
    pub(super) account: PhysicalStorageAccount,
}

impl<CanonicalBlock> PublishedCanonicalOutput<CanonicalBlock> {
    pub fn aggregate(&self) -> &PublishedCanonicalAggregate<CanonicalBlock> {
        &self.aggregate
    }

    #[cfg(test)]
    pub fn root(&self) -> &CanonicalBlock {
        match self.aggregate.inner.member(self.root) {
            Some(root) => root,
            None => unreachable_final_owner_state(),
        }
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }
}

/// Outcome-evidence owning half ready to move into a domain report.
pub(crate) struct PublishedOutcomeEvidenceOutput<EvidenceBlock> {
    pub(super) aggregate: PublishedOutcomeEvidenceAggregate<EvidenceBlock>,
    pub(super) root: RetainedSlotIdentity,
}

/// Evidence-only terminal owner with its frozen outer failure cause and account lineage.
pub(crate) struct PublishedRichFailureOutput<EvidenceBlock> {
    pub(super) outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
    pub(super) account: PhysicalStorageAccount,
}

impl<EvidenceBlock> PublishedRichFailureOutput<EvidenceBlock> {
    pub fn outcome_evidence(&self) -> &PublishedOutcomeEvidenceOutput<EvidenceBlock> {
        &self.outcome_evidence
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        &self.account
    }
}

impl<EvidenceBlock> PublishedOutcomeEvidenceOutput<EvidenceBlock> {
    pub fn aggregate(&self) -> &PublishedOutcomeEvidenceAggregate<EvidenceBlock> {
        &self.aggregate
    }

    #[cfg(test)]
    pub fn root(&self) -> &EvidenceBlock {
        match self.aggregate.inner.member(self.root) {
            Some(root) => root,
            None => unreachable_final_owner_state(),
        }
    }
}

/// Frozen destination-aware ledger evidence moved into a domain report.
pub(crate) struct PublishedLedgerEvidence<P, D> {
    pub(super) evidence: PublishedPart<TerminalLedgerEvidence<P, D>>,
}

impl<P, D> PublishedLedgerEvidence<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    pub fn header(&self) -> Option<FallbackHeaderSnapshot<P>> {
        self.evidence.allocation().as_ref().header()
    }

    pub fn terminal_cause(&self) -> Option<TerminalEvidenceCause> {
        self.header().map(FallbackHeaderSnapshot::terminal_cause)
    }

    pub fn dimension(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.evidence.allocation().as_ref().dimension(dimension)
    }

    pub fn phase(&self, phase: P) -> Option<PhaseSnapshot> {
        self.evidence.allocation().as_ref().phase(phase)
    }

    pub fn dimension_key_at(&self, index: usize) -> Option<D> {
        self.evidence.allocation().as_ref().dimension_key_at(index)
    }

    pub fn phase_key_at(&self, index: usize) -> Option<P> {
        self.evidence.allocation().as_ref().phase_key_at(index)
    }
}

/// Pre-admitted fallback owner after a failed terminal publication.
pub(crate) struct PublishedFallbackOutput<F> {
    pub(super) part: PublishedPart<F>,
}

impl<F> PublishedFallbackOutput<F> {
    pub fn value(&self) -> &F {
        self.part.allocation().as_ref()
    }

    pub fn account(&self) -> &PhysicalStorageAccount {
        self.part.account()
    }
}

/// Exact published success owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, P, D> {
    pub(crate) canonical: PublishedCanonicalOutput<CanonicalBlock>,
    pub(crate) outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

/// Exact published rich-failure owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedRichFailureParts<EvidenceBlock, P, D> {
    pub(crate) outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

/// Exact published minimal-fallback owners produced directly by pure handoff.
pub(crate) struct PublishedRetainedFallbackParts<F, P, D> {
    pub(crate) fallback: PublishedFallbackOutput<F>,
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

/// Exact published success for an operation that produced only frozen ledger evidence.
pub(crate) struct PublishedRetainedEvidenceOnlySuccessParts<P, D> {
    pub(crate) ledger_evidence: PublishedLedgerEvidence<P, D>,
}

impl<CanonicalBlock, EvidenceBlock, P, D>
    PreparedRetainedSuccess<CanonicalBlock, EvidenceBlock, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Performs a pure ownership handoff directly into exact success owners.
    pub fn handoff(self) -> PublishedRetainedSuccessParts<CanonicalBlock, EvidenceBlock, P, D> {
        let Self {
            canonical,
            outcome_evidence,
            canonical_root,
            outcome_evidence_root,
            account,
            evidence,
        } = self;
        let ledger_evidence = PublishedLedgerEvidence { evidence };
        PublishedRetainedSuccessParts {
            canonical: PublishedCanonicalOutput {
                aggregate: canonical,
                root: canonical_root,
                account,
            },
            outcome_evidence: PublishedOutcomeEvidenceOutput {
                aggregate: outcome_evidence,
                root: outcome_evidence_root,
            },
            ledger_evidence,
        }
    }
}

impl<EvidenceBlock, P, D> PreparedRetainedRichFailure<EvidenceBlock, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Retains the complete physical graph after its coupled cause failed to drop.
    pub(crate) fn quarantine_cause_drop_failure(self) {
        let _ = self
            .account
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        std::mem::forget(self);
    }

    /// Performs a pure ownership handoff directly into exact rich-failure owners.
    pub fn handoff(self) -> PublishedRetainedRichFailureParts<EvidenceBlock, P, D> {
        let Self {
            outcome_evidence,
            outcome_evidence_root,
            account,
            evidence,
        } = self;
        PublishedRetainedRichFailureParts {
            outcome_evidence: PublishedRichFailureOutput {
                outcome_evidence: PublishedOutcomeEvidenceOutput {
                    aggregate: outcome_evidence,
                    root: outcome_evidence_root,
                },
                account,
            },
            ledger_evidence: PublishedLedgerEvidence { evidence },
        }
    }
}

impl<F, P, D> PreparedRetainedFallback<F, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Retains payload and ledger evidence using the already-owned account anchor.
    pub(crate) fn quarantine_cause_drop_failure(self) {
        let _ = self
            .part
            .account()
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        std::mem::forget(self);
    }

    /// Performs a pure ownership handoff directly into exact fallback owners.
    pub fn handoff(self) -> PublishedRetainedFallbackParts<F, P, D> {
        let Self { part, evidence } = self;
        PublishedRetainedFallbackParts {
            fallback: PublishedFallbackOutput { part },
            ledger_evidence: PublishedLedgerEvidence { evidence },
        }
    }
}

impl<P, D> PreparedRetainedEvidenceOnlySuccess<P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    /// Performs a pure ownership handoff into frozen ledger evidence.
    pub fn handoff(self) -> PublishedRetainedEvidenceOnlySuccessParts<P, D> {
        PublishedRetainedEvidenceOnlySuccessParts {
            ledger_evidence: PublishedLedgerEvidence {
                evidence: self.evidence,
            },
        }
    }
}
