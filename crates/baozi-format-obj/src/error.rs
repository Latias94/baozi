use baozi_import::{FormatFailureDetailSink, FormatFailureEvidenceProducer};
use std::fmt;
use std::str::Utf8Error;

/// Stable classification of an OBJ-owned content failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ObjFailureCode {
    InvalidUtf8,
    InvalidStatement,
    InvalidFace,
    InvalidNumber,
    NonFiniteNumber,
    InvalidIndex,
    EmptyGeometry,
    InternalInvariant,
}

impl ObjFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "obj.invalid_utf8",
            Self::InvalidStatement => "obj.invalid_statement",
            Self::InvalidFace => "obj.invalid_face",
            Self::InvalidNumber => "obj.invalid_number",
            Self::NonFiniteNumber => "obj.non_finite_number",
            Self::InvalidIndex => "obj.invalid_index",
            Self::EmptyGeometry => "obj.empty_geometry",
            Self::InternalInvariant => "obj.internal_invariant",
        }
    }
}

/// Synchronous OBJ-owned producer sealed into host-owned bounded evidence by the import scope.
#[derive(Debug)]
pub(crate) struct ObjFailure {
    code: ObjFailureCode,
    line: Option<u32>,
    column: Option<u32>,
    byte_offset: Option<u64>,
    subject: Option<&'static str>,
    reason: Option<&'static str>,
}

impl ObjFailure {
    pub(crate) fn invalid_utf8_at(error: Utf8Error, base_offset: u64) -> Option<Self> {
        let valid_up_to = u64::try_from(error.valid_up_to()).ok()?;
        let byte_offset = base_offset.checked_add(valid_up_to)?;
        Some(Self {
            code: ObjFailureCode::InvalidUtf8,
            line: None,
            column: None,
            byte_offset: Some(byte_offset),
            subject: Some("document"),
            reason: Some("invalid-utf8"),
        })
    }

    pub(crate) const fn invalid_document_byte(reason: &'static str, byte_offset: u64) -> Self {
        Self {
            code: ObjFailureCode::InvalidStatement,
            line: None,
            column: None,
            byte_offset: Some(byte_offset),
            subject: Some("document"),
            reason: Some(reason),
        }
    }

    pub(crate) const fn statement(
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> Self {
        Self::located(
            ObjFailureCode::InvalidStatement,
            subject,
            reason,
            line,
            column,
        )
    }

    pub(crate) const fn face(reason: &'static str, line: u32, column: u32) -> Self {
        Self::located(ObjFailureCode::InvalidFace, "face", reason, line, column)
    }

    pub(crate) const fn invalid_float(subject: &'static str, line: u32, column: u32) -> Self {
        Self::located(
            ObjFailureCode::InvalidNumber,
            subject,
            "invalid-float",
            line,
            column,
        )
    }

    pub(crate) const fn non_finite(subject: &'static str, line: u32, column: u32) -> Self {
        Self::located(
            ObjFailureCode::NonFiniteNumber,
            subject,
            "non-finite",
            line,
            column,
        )
    }

    pub(crate) const fn invalid_index(
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> Self {
        Self::located(ObjFailureCode::InvalidIndex, subject, reason, line, column)
    }

    pub(crate) const fn empty_geometry() -> Self {
        Self {
            code: ObjFailureCode::EmptyGeometry,
            line: None,
            column: None,
            byte_offset: None,
            subject: Some("faces"),
            reason: Some("none-declared"),
        }
    }

    pub(crate) const fn internal(reason: &'static str) -> Self {
        Self {
            code: ObjFailureCode::InternalInvariant,
            line: None,
            column: None,
            byte_offset: None,
            subject: Some("normalized-model"),
            reason: Some(reason),
        }
    }

    const fn located(
        code: ObjFailureCode,
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> Self {
        Self {
            code,
            line: Some(line),
            column: Some(column),
            byte_offset: None,
            subject: Some(subject),
            reason: Some(reason),
        }
    }
}

impl fmt::Display for ObjFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "OBJ content failed with {}", self.code.as_str())?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(formatter, " at line {line}, column {column}")?;
        } else if let Some(offset) = self.byte_offset {
            write!(formatter, " at byte offset {offset}")?;
        }
        Ok(())
    }
}

impl FormatFailureEvidenceProducer for ObjFailure {
    fn code(&self) -> &str {
        self.code.as_str()
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        if let Some(line) = self.line {
            sink.unsigned("line", u64::from(line));
        }
        if let Some(column) = self.column {
            sink.unsigned("column", u64::from(column));
        }
        if let Some(byte_offset) = self.byte_offset {
            sink.unsigned("byte_offset", byte_offset);
        }
        if let Some(subject) = self.subject {
            sink.string("subject", subject);
        }
        if let Some(reason) = self.reason {
            sink.string("reason", reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_codes_are_valid_diagnostic_codes() {
        for code in [
            ObjFailureCode::InvalidUtf8,
            ObjFailureCode::InvalidStatement,
            ObjFailureCode::InvalidFace,
            ObjFailureCode::InvalidNumber,
            ObjFailureCode::NonFiniteNumber,
            ObjFailureCode::InvalidIndex,
            ObjFailureCode::EmptyGeometry,
            ObjFailureCode::InternalInvariant,
        ] {
            assert!(baozi_import::DiagnosticCode::new(code.as_str()).is_ok());
        }
    }

    #[test]
    fn utf8_failure_retains_stable_offset_evidence() {
        let mut bytes = [0_u8];
        bytes[0] = u8::MAX;
        let error = std::str::from_utf8(&bytes).expect_err("fixture is invalid UTF-8");
        let failure = ObjFailure::invalid_utf8_at(error, 4_096)
            .expect("fixture offset remains representable");

        assert_eq!(failure.byte_offset, Some(4_096));
        assert_eq!(failure.subject, Some("document"));
        assert_eq!(failure.reason, Some("invalid-utf8"));
    }

    #[test]
    fn utf8_failure_offset_addition_is_checked() {
        let error = std::str::from_utf8(&[b'a', u8::MAX])
            .expect_err("fixture is invalid UTF-8 after one byte");

        assert!(ObjFailure::invalid_utf8_at(error, u64::MAX).is_none());
    }

    #[test]
    fn number_failure_retains_stable_location_evidence() {
        let failure = ObjFailure::invalid_float("position", 1, 3);

        assert_eq!(failure.code, ObjFailureCode::InvalidNumber);
        assert_eq!(failure.line, Some(1));
        assert_eq!(failure.column, Some(3));
        assert_eq!(failure.subject, Some("position"));
        assert_eq!(failure.reason, Some("invalid-float"));
    }
}
