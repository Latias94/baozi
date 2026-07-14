use crate::config::ImportLossKind;
use crate::diagnostic::{DiagnosticByteRange, DiagnosticCode};
use crate::ledger::ImportReservation;
use baozi_io::AssetPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportLossDisposition {
    Applied,
    RejectedByConformance,
    RejectedByLossPolicy,
}

/// The typed semantic object affected by one retained loss fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImportLossSubject {
    FormatRecovery,
    SourceScopeDetails,
}

/// A bounded, session-authenticated fact about semantic loss or recovery.
#[derive(Debug)]
pub struct ImportLossEvidence {
    kind: ImportLossKind,
    subject: ImportLossSubject,
    code: DiagnosticCode,
    source: AssetPath,
    raw_range: Option<DiagnosticByteRange>,
    disposition: ImportLossDisposition,
    reservation: Option<ImportReservation>,
}

impl ImportLossEvidence {
    pub(crate) const fn new(
        kind: ImportLossKind,
        subject: ImportLossSubject,
        code: DiagnosticCode,
        source: AssetPath,
        raw_range: Option<DiagnosticByteRange>,
        disposition: ImportLossDisposition,
        reservation: ImportReservation,
    ) -> Self {
        Self {
            kind,
            subject,
            code,
            source,
            raw_range,
            disposition,
            reservation: Some(reservation),
        }
    }

    pub const fn kind(&self) -> ImportLossKind {
        self.kind
    }

    pub const fn subject(&self) -> ImportLossSubject {
        self.subject
    }

    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    pub const fn source(&self) -> &AssetPath {
        &self.source
    }

    /// Returns the exact source range when this loss is localized to raw bytes.
    ///
    /// Document-level subjects such as source-scope detail collapse return `None`.
    pub const fn raw_range(&self) -> Option<DiagnosticByteRange> {
        self.raw_range
    }

    pub const fn disposition(&self) -> ImportLossDisposition {
        self.disposition
    }

    pub(crate) fn reservation_count(&self) -> usize {
        usize::from(self.reservation.is_some())
    }

    pub(crate) fn move_reservation_into(
        &mut self,
        target: &mut crate::metered_storage::AccountedReservationVec,
    ) {
        assert!(
            target.has_slots(self.reservation_count()),
            "loss-evidence publication slots must be admitted before moving a token"
        );
        if let Some(reservation) = self.reservation.take() {
            target.push_reserved_infallible(reservation);
        }
    }

    pub(crate) fn release_reservation(&mut self) {
        drop(self.reservation.take());
    }
}
