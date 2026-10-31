use std::fmt;

use baozi_operation::StoreLedgerEvidence;

use super::runtime::{RuntimeDimension, dimension_semantic};

#[derive(Debug, PartialEq, Eq)]
struct Counters {
    live: u64,
    peak: u64,
    reservations: u64,
    attempted: u64,
    consumed: u64,
    staging: u64,
    canonical: u64,
    evidence: u64,
    peak_evidence: u64,
    published_evidence: u64,
    terminal_records: u64,
}

/// Bounded, lifetime-owned resource evidence for an invocation.
///
/// Equality compares the public counters. The private frozen ledger remains owned until drop.
pub struct PostProcessResourceEvidence {
    counters: Counters,
    _ledger: StoreLedgerEvidence,
}

impl PostProcessResourceEvidence {
    pub(crate) fn from_store(raw: StoreLedgerEvidence) -> Self {
        let header = raw.header();
        let dimension = |kind| {
            raw.dimension_by_semantic(dimension_semantic(kind))
                .expect("the frozen direct-operation dimension catalog is complete")
        };
        let ordinary = dimension(RuntimeDimension::Work);
        let terminal = dimension(RuntimeDimension::FailureTerminalWork);
        let counters = Counters {
            live: header.current_live_bytes(),
            peak: header.peak_live_bytes(),
            reservations: header.active_reservations(),
            attempted: ordinary.attempted() + terminal.attempted(),
            consumed: ordinary.consumed() + terminal.consumed(),
            staging: dimension(RuntimeDimension::ScratchBytes).peak_live_bytes(),
            canonical: dimension(RuntimeDimension::CanonicalBytes).committed_canonical_output(),
            evidence: header.current_evidence_bytes(),
            peak_evidence: header.peak_evidence_bytes(),
            published_evidence: dimension(RuntimeDimension::OutcomeEvidenceBytes)
                .published_outcome_evidence(),
            terminal_records: dimension(RuntimeDimension::TerminalEvidenceRecords)
                .published_outcome_evidence(),
        };
        Self {
            counters,
            _ledger: raw,
        }
    }

    /// Returns operation-live bytes at the terminal boundary.
    pub const fn current_live_bytes(&self) -> u64 {
        self.counters.live
    }

    /// Returns the invocation's operation-live high-water mark.
    pub const fn peak_live_bytes(&self) -> u64 {
        self.counters.peak
    }

    /// Returns live linear reservations at the terminal boundary.
    pub const fn active_reservations(&self) -> u64 {
        self.counters.reservations
    }

    /// Returns cumulative admitted work, including protected terminal work.
    pub const fn attempted_work_items(&self) -> u64 {
        self.counters.attempted
    }

    /// Returns cumulative completed work, including protected terminal work.
    pub const fn consumed_work_items(&self) -> u64 {
        self.counters.consumed
    }

    /// Returns the temporary staging capacity high-water mark.
    pub const fn peak_staging_bytes(&self) -> u64 {
        self.counters.staging
    }

    /// Returns newly committed canonical capacity.
    pub const fn committed_canonical_bytes(&self) -> u64 {
        self.counters.canonical
    }

    /// Returns retained ledger evidence capacity at the terminal boundary.
    pub const fn current_evidence_bytes(&self) -> u64 {
        self.counters.evidence
    }

    /// Returns the ledger evidence capacity high-water mark.
    pub const fn peak_evidence_bytes(&self) -> u64 {
        self.counters.peak_evidence
    }

    /// Returns newly published domain outcome-evidence capacity, including retained remaps.
    ///
    /// Publication transfers this capacity out of the operation's live reservations. The returned
    /// value is the terminal publication total; it is not a counter that changes as owners drop.
    /// Terminal ledger records and fallback envelopes are counted separately.
    pub const fn published_outcome_evidence_bytes(&self) -> u64 {
        self.counters.published_evidence
    }

    /// Returns the number of published terminal ledger records and fallback envelopes.
    pub const fn published_terminal_evidence_records(&self) -> u64 {
        self.counters.terminal_records
    }
}

impl fmt::Debug for PostProcessResourceEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessResourceEvidence")
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PostProcessResourceEvidence {
    fn eq(&self, other: &Self) -> bool {
        self.counters == other.counters
    }
}

impl Eq for PostProcessResourceEvidence {}
