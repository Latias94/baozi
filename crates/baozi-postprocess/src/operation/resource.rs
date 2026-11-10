use std::fmt;

use baozi_operation::{FiniteStoreFacts, PhysicalStorageAccount, StoreDimensionSnapshot};

use super::runtime::{RuntimeDimension, dimension_semantic};

/// Bounded resource facts owned independently of the invocation and its Host.
///
/// Equality compares the public counters. Tableless evidence keeps its actual account alive.
pub struct PostProcessResourceEvidence {
    facts: baozi_operation::FiniteStoreFacts,
    _terminal_account: Option<baozi_operation::PhysicalStorageAccount>,
}

impl PostProcessResourceEvidence {
    /// Identifies the semantic vocabulary used by the action counters and budgets.
    pub const fn work_action_contract(&self) -> &'static str {
        super::runtime::WORK_ACTION_CONTRACT
    }

    pub(crate) fn from_facts(facts: FiniteStoreFacts) -> Self {
        Self {
            facts,
            _terminal_account: None,
        }
    }

    pub(crate) fn from_evidence_only(
        facts: FiniteStoreFacts,
        account: PhysicalStorageAccount,
    ) -> Self {
        Self {
            facts,
            _terminal_account: Some(account),
        }
    }

    fn dimension(&self, kind: RuntimeDimension) -> StoreDimensionSnapshot {
        self.facts
            .dimension_by_semantic(dimension_semantic(kind))
            .expect("the frozen direct-operation dimension catalog is complete")
    }

    fn counter_values(&self) -> [u64; 11] {
        [
            self.current_live_bytes(),
            self.peak_live_bytes(),
            self.active_reservations(),
            self.attempted_work_actions(),
            self.consumed_work_actions(),
            self.peak_staging_bytes(),
            self.committed_canonical_bytes(),
            self.current_administrative_bytes(),
            self.peak_administrative_bytes(),
            self.published_outcome_evidence_bytes(),
            self.published_terminal_evidence_records(),
        ]
    }

    /// Returns operation-live bytes at the terminal boundary.
    pub fn current_live_bytes(&self) -> u64 {
        self.facts.header().current_live_bytes()
    }

    /// Returns the invocation's operation-live high-water mark.
    pub fn peak_live_bytes(&self) -> u64 {
        self.facts.header().peak_live_bytes()
    }

    /// Returns live linear reservations at the terminal boundary.
    pub fn active_reservations(&self) -> u64 {
        self.facts.header().active_reservations()
    }

    /// Returns representable attempted semantic actions, including denied admission and
    /// protected terminal work. Delegated core/share actions retain their defined units.
    pub fn attempted_work_actions(&self) -> u64 {
        self.dimension(RuntimeDimension::Work).attempted()
            + self
                .dimension(RuntimeDimension::FailureTerminalWork)
                .attempted()
    }

    /// Returns normally returned semantic actions, including semantic-error returns and
    /// protected terminal work. It does not count successful normal tuples or control credits.
    pub fn consumed_work_actions(&self) -> u64 {
        self.dimension(RuntimeDimension::Work).consumed()
            + self
                .dimension(RuntimeDimension::FailureTerminalWork)
                .consumed()
    }

    /// Returns the temporary staging capacity high-water mark.
    pub fn peak_staging_bytes(&self) -> u64 {
        self.dimension(RuntimeDimension::ScratchBytes)
            .peak_live_bytes()
    }

    /// Returns newly committed canonical capacity.
    pub fn committed_canonical_bytes(&self) -> u64 {
        self.dimension(RuntimeDimension::CanonicalBytes)
            .committed_canonical_output()
    }

    /// Returns retained ledger evidence capacity at the terminal boundary.
    pub fn current_administrative_bytes(&self) -> u64 {
        self.facts.header().current_administrative_bytes()
    }

    /// Returns the ledger evidence capacity high-water mark.
    pub fn peak_administrative_bytes(&self) -> u64 {
        self.facts.header().peak_administrative_bytes()
    }

    /// Returns newly published domain outcome-evidence capacity, including retained remaps.
    ///
    /// Publication transfers this capacity out of the operation's live reservations. The returned
    /// value is the terminal publication total; it is not a counter that changes as owners drop.
    /// Terminal ledger records and fallback envelopes are counted separately.
    pub fn published_outcome_evidence_bytes(&self) -> u64 {
        self.dimension(RuntimeDimension::OutcomeEvidenceBytes)
            .published_outcome_evidence()
    }

    /// Returns the number of published terminal ledger records and fallback envelopes.
    pub fn published_terminal_evidence_records(&self) -> u64 {
        self.dimension(RuntimeDimension::TerminalEvidenceRecords)
            .published_outcome_evidence()
    }
}

impl fmt::Debug for PostProcessResourceEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessResourceEvidence")
            .field("work_action_contract", &self.work_action_contract())
            .field("counters", &self.counter_values())
            .finish_non_exhaustive()
    }
}

impl PartialEq for PostProcessResourceEvidence {
    fn eq(&self, other: &Self) -> bool {
        self.counter_values() == other.counter_values()
    }
}

impl Eq for PostProcessResourceEvidence {}
