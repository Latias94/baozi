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

/// A bounded, session-authenticated fact about semantic loss or recovery.
#[derive(Debug)]
pub struct ImportLossEvidence {
    kind: ImportLossKind,
    code: DiagnosticCode,
    source: AssetPath,
    raw_range: DiagnosticByteRange,
    disposition: ImportLossDisposition,
    reservation: Option<ImportReservation>,
}

impl ImportLossEvidence {
    pub(crate) const fn new(
        kind: ImportLossKind,
        code: DiagnosticCode,
        source: AssetPath,
        raw_range: DiagnosticByteRange,
        disposition: ImportLossDisposition,
        reservation: ImportReservation,
    ) -> Self {
        Self {
            kind,
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

    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    pub const fn source(&self) -> &AssetPath {
        &self.source
    }

    pub const fn raw_range(&self) -> DiagnosticByteRange {
        self.raw_range
    }

    pub const fn disposition(&self) -> ImportLossDisposition {
        self.disposition
    }

    pub(crate) fn reservation_count(&self) -> usize {
        usize::from(self.reservation.is_some())
    }

    pub(crate) fn drain_reservation(&mut self, target: &mut Vec<ImportReservation>) {
        if let Some(reservation) = self.reservation.take() {
            target.push(reservation);
        }
    }

    pub(crate) fn release_reservation(&mut self) {
        drop(self.reservation.take());
    }
}
