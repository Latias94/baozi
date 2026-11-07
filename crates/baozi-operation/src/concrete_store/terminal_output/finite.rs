//! Finite terminal facts retain the original domain independently of physical output shape.

use super::*;
use crate::ControlCause;
use crate::ledger::finite::FiniteLedgerSnapshot;
use crate::retained::{
    FiniteRetainedOutput, FiniteRetainedOutputPayload, PreparedFiniteRetainedPublication,
    quarantine_coupled_cause_account,
};

type RawPreparedFiniteStore = PreparedFiniteRetainedPublication<
    StoredBlock,
    StoredBlock,
    StoreFallbackEnvelope,
    StorePhaseKey,
    StoreDimensionKey,
>;
type FinitePayload = FiniteRetainedOutputPayload<StoredBlock, StoredBlock, StoreFallbackEnvelope>;

/// Fixed terminal facts and their original runtime vocabulary owner.
///
/// This owner is produced only by qualified finite terminal capture. Early failures retain their
/// actual invocation residue instead of constructing an empty or partial substitute for facts.
pub struct FiniteStoreFacts {
    snapshot: FiniteLedgerSnapshot<StorePhaseKey, StoreDimensionKey>,
    cause: StoreTerminalCause,
    domain: StoreKeyDomain,
    original_freeze_control: Option<ControlCause>,
}

impl FiniteStoreFacts {
    pub(in crate::concrete_store) fn from_snapshot(
        snapshot: FiniteLedgerSnapshot<StorePhaseKey, StoreDimensionKey>,
        cause: StoreTerminalCause,
        domain: StoreKeyDomain,
    ) -> Self {
        Self {
            snapshot,
            cause,
            domain,
            original_freeze_control: None,
        }
    }

    /// Borrows the detailed control cause observed by the original finite freeze, if any.
    pub const fn original_freeze_control(&self) -> Option<&ControlCause> {
        self.original_freeze_control.as_ref()
    }

    fn attach_original_freeze_control(&mut self, control: Option<ControlCause>) {
        if self.original_freeze_control.is_none() {
            self.original_freeze_control = control;
        }
    }

    /// Returns the complete header from the one qualified fixed snapshot.
    pub fn header(&self) -> StoreLedgerEvidenceHeader {
        StoreLedgerEvidenceHeader {
            schema_revision: crate::retained::TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION,
            terminal_cause: self.cause,
            last_completed_phase: self
                .snapshot
                .last_completed_phase()
                .map(StorePhaseKey::semantic_id),
            dimension_count: self.snapshot.dimensions().len(),
            phase_count: self.snapshot.phases().len(),
            current_live_bytes: self.snapshot.current_live_bytes(),
            peak_live_bytes: self.snapshot.peak_live_bytes(),
            active_reservations: self.snapshot.active_reservations(),
            current_administrative_bytes: self.snapshot.current_administrative_bytes(),
            peak_administrative_bytes: self.snapshot.peak_administrative_bytes(),
        }
    }

    /// Returns the actual administrative cover recorded before exclusive retirement, if present.
    pub fn covered_administrative_bytes(&self) -> Option<u64> {
        self.snapshot.covered_administrative_bytes()
    }

    /// Returns a frozen dimension after exact runtime-domain validation.
    pub fn dimension(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<StoreDimensionSnapshot, StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let raw = self
            .snapshot
            .dimension(dimension)
            .copied()
            .ok_or(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
                dimension,
            }))?;
        Ok(StoreDimensionSnapshot::from_raw(dimension, raw))
    }

    /// Returns a frozen dimension by stable semantic identity.
    pub fn dimension_by_semantic(
        &self,
        dimension: StoreDimensionSemanticId,
    ) -> Option<StoreDimensionSnapshot> {
        let dimension = self.domain.try_admit_dimension(dimension)?;
        self.dimension(dimension).ok()
    }

    /// Enumerates a frozen dimension in canonical key order.
    pub fn dimension_at(&self, index: usize) -> Option<StoreDimensionSnapshot> {
        let (dimension, raw) = self.snapshot.dimensions().nth(index)?;
        self.domain
            .owns_dimension(dimension)
            .then(|| StoreDimensionSnapshot::from_raw(dimension, *raw))
    }

    /// Returns a frozen phase after exact runtime-domain validation.
    pub fn phase(&self, phase: StorePhaseKey) -> Result<StorePhaseSnapshot, StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        let raw = self
            .snapshot
            .phase(phase)
            .copied()
            .ok_or(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))?;
        Ok(StorePhaseSnapshot::from_raw(phase, raw))
    }

    /// Returns a frozen phase by stable semantic identity.
    pub fn phase_by_semantic(&self, phase: StorePhaseSemanticId) -> Option<StorePhaseSnapshot> {
        let phase = self.domain.try_admit_phase(phase)?;
        self.phase(phase).ok()
    }

    /// Enumerates a frozen phase in canonical key order.
    pub fn phase_at(&self, index: usize) -> Option<StorePhaseSnapshot> {
        let (phase, raw) = self.snapshot.phases().nth(index)?;
        self.domain
            .owns_phase(phase)
            .then(|| StorePhaseSnapshot::from_raw(phase, *raw))
    }
}

fn into_parts(
    raw: RawPreparedFiniteStore,
    domain: StoreKeyDomain,
    original_freeze_control: Option<ControlCause>,
) -> (FinitePayload, FiniteStoreFacts) {
    let FiniteRetainedOutput {
        payload,
        facts,
        cause,
    } = raw.into_output();
    let cause = match cause {
        TerminalEvidenceCause::Success => StoreTerminalCause::Success,
        TerminalEvidenceCause::Failure(cause) => {
            StoreTerminalCause::Failure(StorePreparationFailure::from_raw(cause))
        }
    };
    let mut facts = FiniteStoreFacts::from_snapshot(facts, cause, domain);
    facts.attach_original_freeze_control(original_freeze_control);
    (payload, facts)
}

/// Finite outcomes of a success request, including qualified terminal downgrades.
pub enum PreparedFiniteStoreSuccessOutcome<CK, CT, EK, ET> {
    /// Both original primary aggregates committed.
    Success(PreparedFiniteStoreSuccess<CK, CT, EK, ET>),
    /// The original evidence aggregate committed with a terminal failure.
    RichFailure(PreparedFiniteStoreFailure<EK, ET>),
    /// The original pre-admitted fallback committed.
    MinimalFallback(PreparedFiniteStoreFallback),
}

/// Finite outcomes of an evidence-bearing failure request.
pub enum PreparedFiniteStoreFailureOutcome<EK, ET> {
    /// The original evidence aggregate committed.
    RichFailure(PreparedFiniteStoreFailure<EK, ET>),
    /// The original pre-admitted fallback committed.
    MinimalFallback(PreparedFiniteStoreFallback),
}

impl<EK, ET> PreparedFiniteStoreFailureOutcome<EK, ET> {
    pub(in crate::concrete_store) fn quarantine_cause_drop_failure(self) {
        let account = match &self {
            Self::RichFailure(prepared) => prepared.outcome_evidence.account(),
            Self::MinimalFallback(prepared) => &prepared.terminal_account,
        };
        quarantine_coupled_cause_account(account);
        std::mem::forget(self);
    }
}

/// Finite outcomes of a success request with no retained primary output.
pub enum PreparedFiniteStoreEvidenceOnlyOutcome {
    /// The fixed facts committed after verified fallback destruction.
    Success(PreparedFiniteStoreEvidenceOnlySuccess),
    /// The original fallback committed after a terminal rejection.
    MinimalFallback(PreparedFiniteStoreFallback),
}

/// Original committed canonical and evidence owners, with one fixed facts owner.
#[must_use = "prepared finite success must be handed off or deliberately dropped"]
pub struct PreparedFiniteStoreSuccess<CK, CT, EK, ET> {
    canonical: PublishedCanonicalStore<CK, CT>,
    outcome_evidence: PublishedOutcomeEvidenceStore<EK, ET>,
    facts: FiniteStoreFacts,
}

impl<CK, CT, EK, ET> PreparedFiniteStoreSuccess<CK, CT, EK, ET> {
    /// Moves committed physical owners and fixed facts without allocation or account sharing.
    pub fn handoff(self) -> PublishedFiniteStoreSuccessParts<CK, CT, EK, ET> {
        PublishedFiniteStoreSuccessParts {
            canonical: self.canonical,
            outcome_evidence: self.outcome_evidence,
            facts: self.facts,
        }
    }
}

/// Original committed rich-failure owner, with one fixed facts owner.
#[must_use = "prepared finite rich failure must be handed off or deliberately dropped"]
pub struct PreparedFiniteStoreFailure<EK, ET> {
    outcome_evidence: PublishedRichFailureStore<EK, ET>,
    facts: FiniteStoreFacts,
}

impl<EK, ET> PreparedFiniteStoreFailure<EK, ET> {
    /// Moves the committed evidence owner and fixed facts without allocation or sharing.
    pub fn handoff(self) -> PublishedFiniteStoreFailureParts<EK, ET> {
        PublishedFiniteStoreFailureParts {
            outcome_evidence: self.outcome_evidence,
            facts: self.facts,
        }
    }
}

/// Original committed fallback and independent terminal account, with fixed facts.
#[must_use = "prepared finite fallback must be handed off or deliberately dropped"]
pub struct PreparedFiniteStoreFallback {
    fallback: PublishedStoreFallbackOutput,
    facts: FiniteStoreFacts,
    terminal_account: PhysicalStorageAccount,
}

impl PreparedFiniteStoreFallback {
    pub(in crate::concrete_store) fn attach_original_freeze_control(
        &mut self,
        control: Option<ControlCause>,
    ) {
        self.facts.attach_original_freeze_control(control);
    }

    pub(in crate::concrete_store) fn quarantine_cause_drop_failure(self) {
        quarantine_coupled_cause_account(&self.terminal_account);
        std::mem::forget(self);
    }

    /// Moves the original fallback, facts and independent account without new admission.
    pub fn handoff(self) -> PublishedFiniteStoreFallbackParts {
        PublishedFiniteStoreFallbackParts {
            fallback: self.fallback,
            facts: self.facts,
            terminal_account: self.terminal_account,
        }
    }
}

/// Qualified fixed facts and the original account after verified fallback destruction.
#[must_use = "prepared finite facts must be handed off or deliberately dropped"]
pub struct PreparedFiniteStoreEvidenceOnlySuccess {
    facts: FiniteStoreFacts,
    terminal_account: PhysicalStorageAccount,
}

impl PreparedFiniteStoreEvidenceOnlySuccess {
    /// Moves the fixed facts and original account without new admission.
    pub fn handoff(self) -> PublishedFiniteStoreEvidenceOnlySuccessParts {
        PublishedFiniteStoreEvidenceOnlySuccessParts {
            facts: self.facts,
            terminal_account: self.terminal_account,
        }
    }
}

/// Exact finite success after pure ownership handoff.
#[must_use = "the parts contain every committed success owner and the original facts domain"]
pub struct PublishedFiniteStoreSuccessParts<CK, CT, EK, ET> {
    /// Original canonical aggregate with its typed root.
    pub canonical: PublishedCanonicalStore<CK, CT>,
    /// Original evidence aggregate with its typed root.
    pub outcome_evidence: PublishedOutcomeEvidenceStore<EK, ET>,
    /// Qualified fixed terminal facts and original key domain.
    pub facts: FiniteStoreFacts,
}

/// Exact finite rich failure after pure ownership handoff.
#[must_use = "the parts contain the committed evidence owner and original facts domain"]
pub struct PublishedFiniteStoreFailureParts<EK, ET> {
    /// Original rich-failure aggregate with its typed root.
    pub outcome_evidence: PublishedRichFailureStore<EK, ET>,
    /// Qualified fixed terminal facts and original key domain.
    pub facts: FiniteStoreFacts,
}

/// Exact finite fallback after pure ownership handoff.
#[must_use = "the parts contain the fallback, fixed facts and original independent account"]
pub struct PublishedFiniteStoreFallbackParts {
    /// Original pre-admitted fallback.
    pub fallback: PublishedStoreFallbackOutput,
    /// Qualified fixed terminal facts and original key domain.
    pub facts: FiniteStoreFacts,
    /// Original independent terminal account, retained through the tableless outcome.
    pub terminal_account: PhysicalStorageAccount,
}

/// Exact finite success with no retained primary output.
#[must_use = "the parts contain fixed facts and the original independent account"]
pub struct PublishedFiniteStoreEvidenceOnlySuccessParts {
    /// Qualified fixed terminal facts and original key domain.
    pub facts: FiniteStoreFacts,
    /// Original independent terminal account, retained after all physical parts were discarded.
    pub terminal_account: PhysicalStorageAccount,
}

pub(in crate::concrete_store) fn classify_success<CK, CT, EK, ET>(
    raw: RawPreparedFiniteStore,
    canonical_root: CanonicalStoreRef<CK, CT>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    domain: StoreKeyDomain,
) -> PreparedFiniteStoreSuccessOutcome<CK, CT, EK, ET> {
    let (payload, facts) = into_parts(raw, domain, None);
    match payload {
        FinitePayload::Success {
            canonical,
            outcome_evidence,
        } => PreparedFiniteStoreSuccessOutcome::Success(PreparedFiniteStoreSuccess {
            canonical: PublishedCanonicalStore {
                raw: canonical,
                root: canonical_root,
            },
            outcome_evidence: PublishedOutcomeEvidenceStore {
                raw: outcome_evidence,
                root: evidence_root,
            },
            facts,
        }),
        FinitePayload::RichFailure(raw) => {
            PreparedFiniteStoreSuccessOutcome::RichFailure(PreparedFiniteStoreFailure {
                outcome_evidence: PublishedRichFailureStore {
                    raw,
                    root: evidence_root,
                },
                facts,
            })
        }
        FinitePayload::MinimalFallback {
            fallback,
            terminal_account,
        } => PreparedFiniteStoreSuccessOutcome::MinimalFallback(PreparedFiniteStoreFallback {
            fallback: PublishedStoreFallbackOutput { raw: fallback },
            facts,
            terminal_account,
        }),
        FinitePayload::EvidenceOnlySuccess { .. } => unreachable_store_state(),
    }
}

pub(in crate::concrete_store) fn classify_frozen_success<CK, CT, EK, ET>(
    raw: RawPreparedFiniteStore,
    canonical_root: CanonicalStoreRef<CK, CT>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    domain: StoreKeyDomain,
) -> PreparedFiniteStoreSuccess<CK, CT, EK, ET> {
    let (payload, facts) = into_parts(raw, domain, None);
    let FinitePayload::Success {
        canonical,
        outcome_evidence,
    } = payload
    else {
        unreachable_store_state()
    };
    PreparedFiniteStoreSuccess {
        canonical: PublishedCanonicalStore {
            raw: canonical,
            root: canonical_root,
        },
        outcome_evidence: PublishedOutcomeEvidenceStore {
            raw: outcome_evidence,
            root: evidence_root,
        },
        facts,
    }
}

pub(in crate::concrete_store) fn classify_failure<EK, ET>(
    raw: RawPreparedFiniteStore,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    domain: StoreKeyDomain,
    original_freeze_control: Option<ControlCause>,
) -> PreparedFiniteStoreFailureOutcome<EK, ET> {
    let (payload, facts) = into_parts(raw, domain, original_freeze_control);
    match payload {
        FinitePayload::RichFailure(raw) => {
            PreparedFiniteStoreFailureOutcome::RichFailure(PreparedFiniteStoreFailure {
                outcome_evidence: PublishedRichFailureStore {
                    raw,
                    root: evidence_root,
                },
                facts,
            })
        }
        FinitePayload::MinimalFallback {
            fallback,
            terminal_account,
        } => PreparedFiniteStoreFailureOutcome::MinimalFallback(PreparedFiniteStoreFallback {
            fallback: PublishedStoreFallbackOutput { raw: fallback },
            facts,
            terminal_account,
        }),
        FinitePayload::Success { .. } | FinitePayload::EvidenceOnlySuccess { .. } => {
            unreachable_store_state()
        }
    }
}

pub(in crate::concrete_store) fn classify_evidence_only(
    raw: RawPreparedFiniteStore,
    domain: StoreKeyDomain,
) -> PreparedFiniteStoreEvidenceOnlyOutcome {
    let (payload, facts) = into_parts(raw, domain, None);
    match payload {
        FinitePayload::EvidenceOnlySuccess { terminal_account } => {
            PreparedFiniteStoreEvidenceOnlyOutcome::Success(
                PreparedFiniteStoreEvidenceOnlySuccess {
                    facts,
                    terminal_account,
                },
            )
        }
        FinitePayload::MinimalFallback {
            fallback,
            terminal_account,
        } => PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(PreparedFiniteStoreFallback {
            fallback: PublishedStoreFallbackOutput { raw: fallback },
            facts,
            terminal_account,
        }),
        FinitePayload::Success { .. } | FinitePayload::RichFailure(_) => unreachable_store_state(),
    }
}

pub(in crate::concrete_store) fn classify_fallback(
    raw: RawPreparedFiniteStore,
    domain: StoreKeyDomain,
) -> PreparedFiniteStoreFallback {
    classify_fallback_with_original_freeze_control(raw, domain, None)
}

pub(in crate::concrete_store) fn classify_fallback_with_original_freeze_control(
    raw: RawPreparedFiniteStore,
    domain: StoreKeyDomain,
    original_freeze_control: Option<ControlCause>,
) -> PreparedFiniteStoreFallback {
    let (payload, facts) = into_parts(raw, domain, original_freeze_control);
    match payload {
        FinitePayload::MinimalFallback {
            fallback,
            terminal_account,
        } => PreparedFiniteStoreFallback {
            fallback: PublishedStoreFallbackOutput { raw: fallback },
            facts,
            terminal_account,
        },
        FinitePayload::Success { .. }
        | FinitePayload::RichFailure(_)
        | FinitePayload::EvidenceOnlySuccess { .. } => unreachable_store_state(),
    }
}

#[cfg(test)]
mod tests;
