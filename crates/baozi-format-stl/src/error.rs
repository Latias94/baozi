use baozi_import::{FormatFailureDetailSink, FormatFailureSource};
use std::error::Error;
use std::fmt;
use std::num::ParseFloatError;
use std::str::Utf8Error;

/// Stable classification of an STL-owned content failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StlFailureCode {
    InvalidUtf8,
    NonAscii,
    InvalidControl,
    InvalidStructure,
    InvalidNumber,
    NonFiniteNumber,
    EmptyGeometry,
    BinarySizeMismatch,
    InternalInvariant,
}

impl StlFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "stl.invalid_utf8",
            Self::NonAscii => "stl.non_ascii",
            Self::InvalidControl => "stl.invalid_control",
            Self::InvalidStructure => "stl.invalid_structure",
            Self::InvalidNumber => "stl.invalid_number",
            Self::NonFiniteNumber => "stl.non_finite_number",
            Self::EmptyGeometry => "stl.empty_geometry",
            Self::BinarySizeMismatch => "stl.binary_size_mismatch",
            Self::InternalInvariant => "stl.internal_invariant",
        }
    }
}

#[derive(Debug)]
enum StlFailureSource {
    Utf8(Utf8Error),
    Float(ParseFloatError),
}

impl fmt::Display for StlFailureSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(error) => error.fmt(formatter),
            Self::Float(error) => error.fmt(formatter),
        }
    }
}

impl Error for StlFailureSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Utf8(error) => Some(error),
            Self::Float(error) => Some(error),
        }
    }
}

/// Retained STL-owned source and bounded evidence producer sealed by the selected import scope.
#[derive(Debug)]
pub struct StlFailure {
    code: StlFailureCode,
    line: Option<u32>,
    column: Option<u32>,
    byte_offset: Option<u64>,
    subject: Option<&'static str>,
    reason: Option<&'static str>,
    expected: Option<u64>,
    actual: Option<u64>,
    source: Option<StlFailureSource>,
}

impl StlFailure {
    pub const fn code(&self) -> StlFailureCode {
        self.code
    }

    pub const fn line(&self) -> Option<u32> {
        self.line
    }

    pub const fn column(&self) -> Option<u32> {
        self.column
    }

    pub const fn byte_offset(&self) -> Option<u64> {
        self.byte_offset
    }

    pub const fn subject(&self) -> Option<&'static str> {
        self.subject
    }

    pub const fn reason(&self) -> Option<&'static str> {
        self.reason
    }

    pub const fn expected(&self) -> Option<u64> {
        self.expected
    }

    pub const fn actual(&self) -> Option<u64> {
        self.actual
    }

    pub(crate) const fn content(
        code: StlFailureCode,
        subject: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            code,
            line: None,
            column: None,
            byte_offset: None,
            subject: Some(subject),
            reason: Some(reason),
            expected: None,
            actual: None,
            source: None,
        }
    }

    pub(crate) const fn located(
        code: StlFailureCode,
        subject: &'static str,
        reason: &'static str,
        line: u32,
        column: u32,
    ) -> Self {
        let mut failure = Self::content(code, subject, reason);
        failure.line = Some(line);
        failure.column = Some(column);
        failure
    }

    pub(crate) const fn at_byte(mut self, offset: u64) -> Self {
        self.byte_offset = Some(offset);
        self
    }

    pub(crate) const fn with_counts(mut self, expected: u64, actual: u64) -> Self {
        self.expected = Some(expected);
        self.actual = Some(actual);
        self
    }

    pub(crate) fn invalid_utf8_at(source: Utf8Error, byte_offset: u64) -> Self {
        let mut failure = Self::content(
            StlFailureCode::InvalidUtf8,
            "ascii-document",
            "invalid-utf8",
        )
        .at_byte(byte_offset);
        failure.source = Some(StlFailureSource::Utf8(source));
        failure
    }

    pub(crate) const fn non_ascii(offset: u64, reason: &'static str) -> Self {
        Self::content(StlFailureCode::NonAscii, "ascii-document", reason).at_byte(offset)
    }

    pub(crate) const fn invalid_control(offset: u64, reason: &'static str) -> Self {
        Self::content(StlFailureCode::InvalidControl, "ascii-document", reason).at_byte(offset)
    }

    pub(crate) fn invalid_float(line: u32, column: u32, source: ParseFloatError) -> Self {
        let mut failure = Self::located(
            StlFailureCode::InvalidNumber,
            "float",
            "invalid-float",
            line,
            column,
        );
        failure.source = Some(StlFailureSource::Float(source));
        failure
    }
}

impl fmt::Display for StlFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "STL content failed with {}", self.code.as_str())?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(formatter, " at line {line}, column {column}")?;
        } else if let Some(offset) = self.byte_offset {
            write!(formatter, " at byte offset {offset}")?;
        }
        Ok(())
    }
}

impl Error for StlFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as &dyn Error)
    }
}

impl FormatFailureSource for StlFailure {
    fn code(&self) -> &'static str {
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
        if let Some(expected) = self.expected {
            sink.unsigned("expected", expected);
        }
        if let Some(actual) = self.actual {
            sink.unsigned("actual", actual);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_codes_are_valid_diagnostic_codes() {
        for code in [
            StlFailureCode::InvalidUtf8,
            StlFailureCode::NonAscii,
            StlFailureCode::InvalidControl,
            StlFailureCode::InvalidStructure,
            StlFailureCode::InvalidNumber,
            StlFailureCode::NonFiniteNumber,
            StlFailureCode::EmptyGeometry,
            StlFailureCode::BinarySizeMismatch,
            StlFailureCode::InternalInvariant,
        ] {
            assert!(baozi_import::DiagnosticCode::new(code.as_str()).is_ok());
        }
    }

    #[test]
    fn invalid_utf8_failure_retains_typed_source_and_stable_details() {
        let bytes = vec![0xff];
        let source = std::str::from_utf8(&bytes).expect_err("fixture must be invalid UTF-8");
        let failure = StlFailure::invalid_utf8_at(source, 37);

        assert_eq!(failure.code(), StlFailureCode::InvalidUtf8);
        assert_eq!(failure.byte_offset(), Some(37));
        assert_eq!(failure.subject(), Some("ascii-document"));
        assert_eq!(failure.reason(), Some("invalid-utf8"));
        assert!(
            Error::source(&failure)
                .and_then(Error::source)
                .and_then(|source| source.downcast_ref::<Utf8Error>())
                .is_some()
        );
    }
}
