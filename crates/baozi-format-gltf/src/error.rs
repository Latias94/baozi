use baozi_import::{FormatFailureDetailSink, FormatFailureEvidenceProducer};
use std::fmt;
use std::str::Utf8Error;

// This is the largest inline payload that keeps the synchronous producer below 128 bytes on 64-bit targets.
const MAX_REASON_BYTES: usize = 70;
const FAILURE_CODE_MASK: u8 = 0x0f;
const FAILURE_HAS_INDEX: u8 = 0x10;
const FAILURE_HAS_BYTE_OFFSET: u8 = 0x20;
const REASON_LENGTH_MASK: u8 = 0x7f;
const REASON_TRUNCATED: u8 = 0x80;
const REASON_UNAVAILABLE: u8 = u8::MAX;

/// Stable classification of a glTF-owned content failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GltfFailureCode {
    InvalidUtf8,
    ForbiddenBom,
    InvalidDocument,
    InvalidReference,
    InvalidAccessor,
    UnsupportedFeature,
    ParserPanicked,
    ResourceContract,
    InternalInvariant,
}

impl GltfFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "gltf.invalid_utf8",
            Self::ForbiddenBom => "gltf.forbidden_bom",
            Self::InvalidDocument => "gltf.invalid_document",
            Self::InvalidReference => "gltf.invalid_reference",
            Self::InvalidAccessor => "gltf.invalid_accessor",
            Self::UnsupportedFeature => "gltf.unsupported_feature",
            Self::ParserPanicked => "gltf.parser_panicked",
            Self::ResourceContract => "gltf.resource_contract",
            Self::InternalInvariant => "gltf.internal_invariant",
        }
    }

    const fn bits(self) -> u8 {
        match self {
            Self::InvalidUtf8 => 0,
            Self::ForbiddenBom => 1,
            Self::InvalidDocument => 2,
            Self::InvalidReference => 3,
            Self::InvalidAccessor => 4,
            Self::UnsupportedFeature => 5,
            Self::ParserPanicked => 6,
            Self::ResourceContract => 7,
            Self::InternalInvariant => 8,
        }
    }

    const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::InvalidUtf8,
            1 => Self::ForbiddenBom,
            2 => Self::InvalidDocument,
            3 => Self::InvalidReference,
            4 => Self::InvalidAccessor,
            5 => Self::UnsupportedFeature,
            6 => Self::ParserPanicked,
            7 => Self::ResourceContract,
            _ => Self::InternalInvariant,
        }
    }
}

/// Synchronous glTF-owned producer sealed into host-owned bounded evidence by the import scope.
#[derive(Debug)]
pub(crate) struct GltfFailure {
    metadata: u8,
    subject: Option<&'static str>,
    index: u64,
    byte_offset: u64,
    reason: BoundedReason,
}

impl GltfFailure {
    pub const fn code(&self) -> GltfFailureCode {
        GltfFailureCode::from_bits(self.metadata & FAILURE_CODE_MASK)
    }

    pub const fn index(&self) -> Option<u64> {
        if self.metadata & FAILURE_HAS_INDEX != 0 {
            Some(self.index)
        } else {
            None
        }
    }

    pub const fn byte_offset(&self) -> Option<u64> {
        if self.metadata & FAILURE_HAS_BYTE_OFFSET != 0 {
            Some(self.byte_offset)
        } else {
            None
        }
    }

    pub(crate) fn invalid_document(reason: impl fmt::Display) -> Self {
        Self::new(GltfFailureCode::InvalidDocument, None, None, reason)
    }

    pub(crate) fn invalid_utf8(source: Utf8Error, byte_offset: u64) -> Self {
        let mut failure = Self::new(
            GltfFailureCode::InvalidUtf8,
            Some("json-document"),
            None,
            format_args!("glTF JSON contains invalid UTF-8: {source}"),
        );
        failure.set_byte_offset(byte_offset);
        failure
    }

    pub(crate) fn forbidden_bom(byte_offset: u64) -> Self {
        let mut failure = Self::new(
            GltfFailureCode::ForbiddenBom,
            Some("json-document"),
            None,
            "glTF JSON must not begin with a UTF-8 byte order mark",
        );
        failure.set_byte_offset(byte_offset);
        failure
    }

    pub(crate) fn invalid_reference(
        subject: &'static str,
        index: Option<usize>,
        reason: impl fmt::Display,
    ) -> Self {
        Self::new(
            GltfFailureCode::InvalidReference,
            Some(subject),
            index.and_then(|value| u64::try_from(value).ok()),
            reason,
        )
    }

    pub(crate) fn invalid_accessor(index: Option<usize>, reason: impl fmt::Display) -> Self {
        Self::new(
            GltfFailureCode::InvalidAccessor,
            Some("accessor"),
            index.and_then(|value| u64::try_from(value).ok()),
            reason,
        )
    }

    pub(crate) fn unsupported(subject: &'static str, reason: impl fmt::Display) -> Self {
        Self::new(
            GltfFailureCode::UnsupportedFeature,
            Some(subject),
            None,
            reason,
        )
    }

    pub(crate) fn parser_panicked(operation: &'static str) -> Self {
        Self::new(
            GltfFailureCode::ParserPanicked,
            Some(operation),
            None,
            "the glTF backend panicked while processing untrusted input",
        )
    }

    pub(crate) fn internal(reason: impl fmt::Display) -> Self {
        Self::new(
            GltfFailureCode::InternalInvariant,
            Some("canonical-scene"),
            None,
            reason,
        )
    }

    fn new(
        code: GltfFailureCode,
        subject: Option<&'static str>,
        index: Option<u64>,
        reason: impl fmt::Display,
    ) -> Self {
        let (index, index_metadata) = match index {
            Some(index) => (index, FAILURE_HAS_INDEX),
            None => (0, 0),
        };
        Self {
            metadata: code.bits() | index_metadata,
            subject,
            index,
            byte_offset: 0,
            reason: BoundedReason::new(reason),
        }
    }

    fn set_byte_offset(&mut self, byte_offset: u64) {
        self.byte_offset = byte_offset;
        self.metadata |= FAILURE_HAS_BYTE_OFFSET;
    }
}

impl fmt::Display for GltfFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason.as_str())
    }
}

impl FormatFailureEvidenceProducer for GltfFailure {
    fn code(&self) -> &str {
        self.code().as_str()
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        if let Some(subject) = self.subject {
            sink.string("subject", subject);
        }
        if let Some(index) = self.index() {
            sink.unsigned("index", index);
        }
        if let Some(byte_offset) = self.byte_offset() {
            sink.unsigned("byte_offset", byte_offset);
        }
        sink.string("reason", self.reason.as_str());
    }
}

struct BoundedReason {
    bytes: [u8; MAX_REASON_BYTES],
    state: u8,
}

impl BoundedReason {
    fn new(reason: impl fmt::Display) -> Self {
        let mut bounded = Self::empty();
        if fmt::write(&mut bounded, format_args!("{reason}")).is_err() {
            bounded.state = REASON_UNAVAILABLE;
        }
        bounded
    }

    fn empty() -> Self {
        Self {
            bytes: [0; MAX_REASON_BYTES],
            state: 0,
        }
    }

    fn write_sanitized(&mut self, value: &str) {
        if self.is_unavailable() || self.is_truncated() {
            return;
        }
        for character in value.chars() {
            let character = if character.is_control() {
                '\u{fffd}'
            } else {
                character
            };
            let mut encoded = [0; 4];
            let encoded = character.encode_utf8(&mut encoded).as_bytes();
            let end = self.len() + encoded.len();
            if end > MAX_REASON_BYTES {
                self.state |= REASON_TRUNCATED;
                return;
            }
            let start = self.len();
            self.bytes[start..end].copy_from_slice(encoded);
            self.state = end as u8;
        }
    }

    fn as_str(&self) -> &str {
        if self.is_unavailable() {
            return "glTF failure detail unavailable";
        }
        std::str::from_utf8(&self.bytes[..self.len()]).unwrap_or("glTF failure detail unavailable")
    }

    const fn len(&self) -> usize {
        (self.state & REASON_LENGTH_MASK) as usize
    }

    const fn is_truncated(&self) -> bool {
        self.state != REASON_UNAVAILABLE && self.state & REASON_TRUNCATED != 0
    }

    const fn is_unavailable(&self) -> bool {
        self.state == REASON_UNAVAILABLE
    }
}

impl fmt::Write for BoundedReason {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_sanitized(value);
        Ok(())
    }
}

impl fmt::Debug for BoundedReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_is_bounded_and_sanitized() {
        let failure = GltfFailure::invalid_document(format!("line\n{}secret", "x".repeat(600)));
        assert!(failure.reason.as_str().len() <= MAX_REASON_BYTES);
        assert!(!failure.reason.as_str().contains('\n'));
        assert!(failure.reason.as_str().contains('\u{fffd}'));
        assert_eq!(failure.code(), GltfFailureCode::InvalidDocument);
    }

    #[test]
    fn reason_storage_is_inline_bounded_and_does_not_allocate_while_formatting() {
        let mut reason = BoundedReason::empty();
        assert!(!reason.is_unavailable());
        let input = "x".repeat(2 * MAX_REASON_BYTES);

        fmt::write(&mut reason, format_args!("{input}"))
            .expect("inline reason formatting must be infallible");

        assert_eq!(reason.as_str().len(), MAX_REASON_BYTES);
        assert!(reason.is_truncated());
        assert_eq!(
            std::mem::size_of::<BoundedReason>(),
            MAX_REASON_BYTES + std::mem::size_of::<u8>()
        );
        assert!(!std::mem::needs_drop::<GltfFailure>());
        assert!(std::mem::size_of::<GltfFailure>() < 128);
    }

    #[test]
    fn invalid_utf8_failure_retains_only_bounded_stable_evidence() {
        let bytes = [u8::MAX];
        let source = std::str::from_utf8(&bytes).expect_err("fixture must be invalid UTF-8");
        let failure = GltfFailure::invalid_utf8(source, 4_096);

        assert_eq!(failure.code(), GltfFailureCode::InvalidUtf8);
        assert_eq!(failure.subject, Some("json-document"));
        assert_eq!(failure.byte_offset(), Some(4_096));
        assert!(failure.reason.as_str().contains("invalid UTF-8"));
        assert!(failure.reason.as_str().len() <= MAX_REASON_BYTES);
    }
}
