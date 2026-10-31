use baozi_import::{FormatFailureDetailSink, FormatFailureEvidenceProducer};

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

/// Synchronous PLY-owned evidence producer sealed by the selected import scope.
///
/// Resource, allocation, cancellation, IO, and scene-validation failures are
/// deliberately not represented here; those remain owned by their scopes.
#[derive(Debug)]
pub(crate) struct PlyFailure {
    code: PlyFailureCode,
    message: &'static str,
    context: Option<&'static str>,
    byte_offset: Option<u64>,
}

impl PlyFailure {
    pub(crate) const fn new(code: PlyFailureCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            context: None,
            byte_offset: None,
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
}

impl FormatFailureEvidenceProducer for PlyFailure {
    fn code(&self) -> &str {
        self.code.as_str()
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        sink.string("reason", self.message);
        if let Some(context) = self.context {
            sink.string("context", context);
        }
        if let Some(offset) = self.byte_offset {
            sink.unsigned("byte_offset", offset);
        }
    }
}
