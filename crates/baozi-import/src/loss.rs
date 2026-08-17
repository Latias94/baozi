use crate::config::ImportLossKind;
use crate::diagnostic::{DiagnosticByteRange, DiagnosticCode};
use crate::store::ImportStoreReservation;
use baozi_io::AssetPath;
use std::fmt;

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
pub struct ImportLossEvidence {
    kind: ImportLossKind,
    subject: ImportLossSubject,
    code: DiagnosticCode,
    source: AssetPath,
    raw_range: Option<DiagnosticByteRange>,
    disposition: ImportLossDisposition,
    reservation: Option<ImportStoreReservation>,
}

impl fmt::Debug for ImportLossEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportLossEvidence")
            .field("kind", &self.kind)
            .field("subject", &self.subject)
            .field("code", &self.code)
            .field("source", &self.source)
            .field("raw_range", &self.raw_range)
            .field("disposition", &self.disposition)
            .field("reservation_live", &self.reservation.is_some())
            .finish()
    }
}

impl ImportLossEvidence {
    pub(crate) const fn new(
        kind: ImportLossKind,
        subject: ImportLossSubject,
        code: DiagnosticCode,
        source: AssetPath,
        raw_range: Option<DiagnosticByteRange>,
        disposition: ImportLossDisposition,
        reservation: ImportStoreReservation,
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

    pub(crate) fn release_reservation(&mut self) {
        drop(self.reservation.take());
    }
}
