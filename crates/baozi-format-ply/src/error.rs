use baozi_import::{FormatFailureDetailSink, FormatFailureSource};
use std::fmt;
use std::str::Utf8Error;

/// Stable classification for failures owned by the PLY grammar and semantic parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlyFailureCode {
    InvalidHeader,
    UnsupportedEncoding,
    InvalidSchema,
    InvalidRecord,
    InvalidScalar,
    InvalidTopology,
    TruncatedPayload,
    InternalInvariant,
}

impl PlyFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidHeader => "ply.invalid_header",
            Self::UnsupportedEncoding => "ply.unsupported_encoding",
            Self::InvalidSchema => "ply.invalid_schema",
            Self::InvalidRecord => "ply.invalid_record",
            Self::InvalidScalar => "ply.invalid_scalar",
            Self::InvalidTopology => "ply.invalid_topology",
            Self::TruncatedPayload => "ply.truncated_payload",
            Self::InternalInvariant => "ply.internal_invariant",
        }
    }
}

/// Retained PLY-owned source and bounded evidence producer sealed by the selected import scope.
///
/// Resource, allocation, cancellation, IO, and scene-validation failures are
/// deliberately not represented here; those remain owned by their scopes.
#[derive(Debug)]
pub struct PlyFailure {
    code: PlyFailureCode,
    message: &'static str,
    context: Option<&'static str>,
    byte_offset: Option<u64>,
    expected: Option<u64>,
    actual: Option<u64>,
    source: Option<PlyFailureSource>,
}

#[derive(Debug)]
enum PlyFailureSource {
    Utf8(Utf8Error),
}

impl fmt::Display for PlyFailureSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlyFailureSource {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Utf8(error) => Some(error),
        }
    }
}

impl PlyFailure {
    pub const fn code(&self) -> PlyFailureCode {
        self.code
    }

    pub const fn context(&self) -> Option<&'static str> {
        self.context
    }

    pub const fn byte_offset(&self) -> Option<u64> {
        self.byte_offset
    }

    pub const fn expected(&self) -> Option<u64> {
        self.expected
    }

    pub const fn actual(&self) -> Option<u64> {
        self.actual
    }

    pub(crate) const fn new(code: PlyFailureCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            context: None,
            byte_offset: None,
            expected: None,
            actual: None,
            source: None,
        }
    }

    pub(crate) const fn with_context(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
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

    pub(crate) fn with_utf8_source(mut self, source: Utf8Error) -> Self {
        self.source = Some(PlyFailureSource::Utf8(source));
        self
    }
}

impl fmt::Display for PlyFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for PlyFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &dyn std::error::Error)
    }
}

impl FormatFailureSource for PlyFailure {
    fn code(&self) -> &'static str {
        self.code.as_str()
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        if let Some(context) = self.context {
            sink.string("context", context);
        }
        if let Some(offset) = self.byte_offset {
            sink.unsigned("byte_offset", offset);
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
    fn invalid_utf8_failure_preserves_its_standard_source_before_sealing() {
        let mut bytes = [0_u8];
        bytes[0] = u8::MAX;
        let error =
            std::str::from_utf8(&bytes).expect_err("fixture contains an invalid UTF-8 lead byte");
        let expected = error.to_string();
        let failure = PlyFailure::new(PlyFailureCode::InvalidHeader, "invalid header text")
            .with_utf8_source(error);

        let source_wrapper =
            std::error::Error::source(&failure).expect("PLY failure must retain its typed source");
        assert_eq!(source_wrapper.to_string(), expected);
        source_wrapper
            .downcast_ref::<PlyFailureSource>()
            .expect("first source hop must be the PLY-owned typed wrapper");

        let utf8_source = source_wrapper
            .source()
            .expect("typed wrapper must retain the standard UTF-8 source");
        let utf8_error = utf8_source
            .downcast_ref::<std::str::Utf8Error>()
            .expect("second source hop must expose std::str::Utf8Error");
        assert_eq!(utf8_error.valid_up_to(), error.valid_up_to());
        assert_eq!(utf8_error.error_len(), error.error_len());
        assert_eq!(utf8_error.to_string(), expected);
    }
}
