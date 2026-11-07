//! Finite terminal output consumes the original physical graph and returns its fixed facts.

use super::physical::{FiniteAccountBoundJointCommit, FiniteCommitFailure};
use super::*;
use crate::ledger::finite::FiniteLedgerSnapshot;

pub(in crate::retained) enum FiniteRetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F> {
    Publication(RetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>),
    EvidenceOnlySuccess,
}

/// The independent finite account remains an anchor even after every physical part is discarded.
pub(crate) struct PreparedFiniteRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D> {
    pub(in crate::retained) payload:
        FiniteRetainedPublicationPayload<CanonicalBlock, EvidenceBlock, F>,
    pub(in crate::retained) facts: FiniteLedgerSnapshot<P, D>,
    pub(in crate::retained) cause: TerminalEvidenceCause,
    pub(in crate::retained) terminal_account: Option<PhysicalStorageAccount>,
}

type FiniteRetainedPreparationResult<O, P, D, F, CB, EB> =
    Result<PreparedFiniteRetainedPublication<CB, EB, F, P, D>, FiniteCommitFailure<O, P, D>>;

/// Concrete published owners after finite commit, with no dynamic numerical evidence.
pub(crate) enum FiniteRetainedOutputPayload<CB, EB, F> {
    Success {
        canonical: PublishedCanonicalOutput<CB>,
        outcome_evidence: PublishedOutcomeEvidenceOutput<EB>,
    },
    RichFailure(PublishedRichFailureOutput<EB>),
    MinimalFallback {
        fallback: PublishedFallbackOutput<F>,
        terminal_account: PhysicalStorageAccount,
    },
    EvidenceOnlySuccess {
        terminal_account: PhysicalStorageAccount,
    },
}

pub(crate) struct FiniteRetainedOutput<CB, EB, F, P, D> {
    pub(crate) payload: FiniteRetainedOutputPayload<CB, EB, F>,
    pub(crate) facts: FiniteLedgerSnapshot<P, D>,
    pub(crate) cause: TerminalEvidenceCause,
}

impl<CB, EB, F, P, D> PreparedFiniteRetainedPublication<CB, EB, F, P, D> {
    /// Moves original committed physical owners and fixed facts without admission or allocation.
    pub(crate) fn into_output(self) -> FiniteRetainedOutput<CB, EB, F, P, D> {
        let Self {
            payload,
            facts,
            cause,
            terminal_account,
        } = self;
        let payload = match payload {
            FiniteRetainedPublicationPayload::Publication(
                RetainedPublicationPayload::Success {
                    canonical,
                    outcome_evidence,
                    canonical_root,
                    outcome_evidence_root,
                    account,
                },
            ) => FiniteRetainedOutputPayload::Success {
                canonical: PublishedCanonicalOutput {
                    aggregate: canonical,
                    root: canonical_root,
                    account,
                },
                outcome_evidence: PublishedOutcomeEvidenceOutput {
                    aggregate: outcome_evidence,
                    root: outcome_evidence_root,
                },
            },
            FiniteRetainedPublicationPayload::Publication(
                RetainedPublicationPayload::RichFailure {
                    outcome_evidence,
                    outcome_evidence_root,
                    account,
                },
            ) => FiniteRetainedOutputPayload::RichFailure(PublishedRichFailureOutput {
                outcome_evidence: PublishedOutcomeEvidenceOutput {
                    aggregate: outcome_evidence,
                    root: outcome_evidence_root,
                },
                account,
            }),
            FiniteRetainedPublicationPayload::Publication(
                RetainedPublicationPayload::MinimalFallback { part },
            ) => FiniteRetainedOutputPayload::MinimalFallback {
                fallback: PublishedFallbackOutput { part },
                terminal_account: terminal_account
                    .unwrap_or_else(|| unreachable_final_owner_state()),
            },
            FiniteRetainedPublicationPayload::EvidenceOnlySuccess => {
                FiniteRetainedOutputPayload::EvidenceOnlySuccess {
                    terminal_account: terminal_account
                        .unwrap_or_else(|| unreachable_final_owner_state()),
                }
            }
        };
        FiniteRetainedOutput {
            payload,
            facts,
            cause,
        }
    }
}

impl<F, CB, EB> FiniteTerminalPhysicalState<F, CB, EB> {
    fn has_ready_shape(&self) -> bool {
        let graph = &self.graph;
        if graph.account.is_none() {
            return false;
        }
        match self.disposition {
            Some(TerminalDisposition::Success { .. }) => {
                graph
                    .fallback
                    .as_ref()
                    .is_some_and(DetachedRetainedPart::is_empty)
                    && graph.canonical_table.is_some()
                    && graph.evidence_table.is_some()
            }
            Some(TerminalDisposition::RichFailure { .. }) => {
                graph
                    .fallback
                    .as_ref()
                    .is_some_and(DetachedRetainedPart::is_empty)
                    && graph.canonical_table.is_none()
                    && graph.evidence_table.is_some()
            }
            Some(TerminalDisposition::MinimalFallback { .. }) => {
                graph
                    .fallback
                    .as_ref()
                    .is_some_and(DetachedRetainedPart::has_exact_owner)
                    && graph.canonical_table.is_none()
                    && graph.evidence_table.is_none()
            }
            Some(TerminalDisposition::EvidenceOnlySuccess) => {
                graph
                    .fallback
                    .as_ref()
                    .is_some_and(DetachedRetainedPart::is_empty)
                    && graph.canonical_table.is_none()
                    && graph.evidence_table.is_none()
            }
            None => false,
        }
    }

    fn into_prepared<P, D>(
        mut self,
        facts: FiniteLedgerSnapshot<P, D>,
    ) -> PreparedFiniteRetainedPublication<CB, EB, F, P, D> {
        let account = self
            .graph
            .account
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let disposition = self
            .disposition
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let (payload, cause, terminal_account) = match disposition {
            TerminalDisposition::Success {
                canonical_root,
                evidence_root,
                set_id,
            } => (
                FiniteRetainedPublicationPayload::Publication(
                    RetainedPublicationPayload::Success {
                        canonical: PublishedCanonicalAggregate::from_table(
                            self.graph
                                .canonical_table
                                .take()
                                .unwrap_or_else(|| unreachable_final_owner_state()),
                            set_id,
                        ),
                        outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(
                            self.graph
                                .evidence_table
                                .take()
                                .unwrap_or_else(|| unreachable_final_owner_state()),
                            set_id,
                        ),
                        canonical_root,
                        outcome_evidence_root: evidence_root,
                        account,
                    },
                ),
                TerminalEvidenceCause::Success,
                None,
            ),
            TerminalDisposition::RichFailure {
                evidence_root,
                set_id,
                cause,
            } => (
                FiniteRetainedPublicationPayload::Publication(
                    RetainedPublicationPayload::RichFailure {
                        outcome_evidence: PublishedOutcomeEvidenceAggregate::from_table(
                            self.graph
                                .evidence_table
                                .take()
                                .unwrap_or_else(|| unreachable_final_owner_state()),
                            set_id,
                        ),
                        outcome_evidence_root: evidence_root,
                        account,
                    },
                ),
                TerminalEvidenceCause::Failure(cause),
                None,
            ),
            TerminalDisposition::MinimalFallback { cause } => (
                FiniteRetainedPublicationPayload::Publication(
                    RetainedPublicationPayload::MinimalFallback {
                        part: PublishedPart::from_exact_owner(
                            self.graph
                                .fallback
                                .take()
                                .unwrap_or_else(|| unreachable_final_owner_state()),
                        ),
                    },
                ),
                TerminalEvidenceCause::Failure(cause),
                Some(account),
            ),
            TerminalDisposition::EvidenceOnlySuccess => (
                FiniteRetainedPublicationPayload::EvidenceOnlySuccess,
                TerminalEvidenceCause::Success,
                Some(account),
            ),
        };
        // Non-minimal rails keep only the empty original fallback shell after acknowledgement.
        drop(self.graph.fallback.take());
        PreparedFiniteRetainedPublication {
            payload,
            facts,
            cause,
            terminal_account,
        }
    }
}

impl<F, CB, EB> AccountBoundPublicationManifest for FiniteTerminalPhysicalState<F, CB, EB> {
    fn publication_account(&self) -> &PhysicalStorageAccount {
        self.graph
            .account
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
    }

    fn contain_uncommitted(self, reason: AccountPoisonReason) -> FinalizationContainmentError {
        self.contain_new_fault(reason)
    }

    fn contain_after_quarantine(self, quarantine: AccountPoisoned) -> FinalizationContainmentError {
        FiniteTerminalPhysicalState::contain_after_quarantine(self, quarantine)
    }
}

impl<O, P, D, F, CB, EB>
    FiniteAccountBoundJointCommit<FiniteTerminalPhysicalState<F, CB, EB>, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(in crate::retained) fn try_into_prepared(
        mut self,
    ) -> FiniteRetainedPreparationResult<O, P, D, F, CB, EB> {
        if !self
            .physical
            .as_ref()
            .unwrap_or_else(|| unreachable_final_owner_state())
            .has_ready_shape()
        {
            let cause = self
                .physical
                .take()
                .unwrap_or_else(|| unreachable_final_owner_state())
                .contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(self.into_error(cause));
        }
        let (physical, facts) = self.try_commit()?;
        Ok(physical.into_prepared(facts))
    }
}
