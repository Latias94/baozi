use crate::SceneDraft;
use crate::descriptor::FormatDescriptor;
use crate::detection::{DetectionResult, DetectionScope};
use crate::options::{
    FormatOptionCodec, OptionRuntimeTypeKey, no_options_codec, no_options_runtime_type_key,
};
use crate::scope::{ParseResult, ParseScope};
use crate::session::ReadHint;
use std::any::Any;
use std::fmt;
use std::panic::RefUnwindSafe;
use std::sync::Arc;

const MAX_DETECTION_FAULT_DETAIL_BYTES: usize = 96;
const MAX_EXTENSION_PANIC_DETAIL_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReadConfidence {
    Maybe,
    Likely,
    Certain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectionInconclusive {
    ProbeExhausted,
}

/// Stable classification of a detector-local failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DetectionFaultCode {
    DetectorFailure,
    ExtensionPanicked,
    ExtensionPoisoned,
}

/// Sanitized, bounded extension failure text.
///
/// Allocation failure is represented explicitly so failure reporting never
/// needs an infallible fallback allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFaultDetail(BoundedFaultDetailStorage);

#[derive(Debug, Clone, PartialEq, Eq)]
enum BoundedFaultDetailStorage {
    Text(String),
    Unavailable,
}

impl BoundedFaultDetail {
    pub(crate) fn new(value: &str) -> Self {
        let required_bytes = sanitized_prefix_len(value, MAX_EXTENSION_PANIC_DETAIL_BYTES);
        let mut output = String::new();
        if output.try_reserve_exact(required_bytes).is_err() {
            return Self(BoundedFaultDetailStorage::Unavailable);
        }
        push_sanitized_prefix(&mut output, value, MAX_EXTENSION_PANIC_DETAIL_BYTES);
        Self(BoundedFaultDetailStorage::Text(output))
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            BoundedFaultDetailStorage::Text(detail) => detail,
            BoundedFaultDetailStorage::Unavailable => "failure detail unavailable",
        }
    }
}

impl AsRef<str> for BoundedFaultDetail {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for BoundedFaultDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionFault {
    code: DetectionFaultCode,
    detail: InlineDetectionFaultDetail,
}

impl DetectionFault {
    fn new(code: DetectionFaultCode, detail: impl AsRef<str>) -> Self {
        Self {
            code,
            detail: InlineDetectionFaultDetail::new(detail.as_ref()),
        }
    }

    pub const fn code(&self) -> DetectionFaultCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        self.detail.as_str()
    }

    pub(crate) fn extension_panicked(detail: impl AsRef<str>) -> Self {
        Self::new(DetectionFaultCode::ExtensionPanicked, detail)
    }

    pub(crate) fn extension_poisoned() -> Self {
        Self::new(
            DetectionFaultCode::ExtensionPoisoned,
            "format registration is poisoned",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionOutcome {
    Match(ReadConfidence),
    NoMatch,
    Inconclusive(DetectionInconclusive),
    Fault(DetectionFault),
}

impl DetectionOutcome {
    /// Creates detector-owned fault evidence.
    ///
    /// Host-owned panic and poison classifications are intentionally not
    /// constructible through the public extension API.
    ///
    /// ```compile_fail
    /// use baozi_import::{DetectionFault, DetectionFaultCode};
    ///
    /// let _forged = DetectionFault::new(
    ///     DetectionFaultCode::ExtensionPoisoned,
    ///     "forged host evidence",
    /// );
    /// ```
    pub fn fault(detail: impl AsRef<str>) -> Self {
        Self::Fault(DetectionFault::new(
            DetectionFaultCode::DetectorFailure,
            detail,
        ))
    }
}

/// Read-only bounded view shared by every detector in one session.
#[derive(Debug, Clone, Copy)]
pub struct Probe<'a> {
    bytes: &'a [u8],
    source_len: u64,
}

impl<'a> Probe<'a> {
    pub(crate) const fn new(bytes: &'a [u8], source_len: u64) -> Self {
        Self { bytes, source_len }
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub const fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn is_complete(&self) -> bool {
        self.bytes.len() as u64 == self.source_len
    }
}

/// Trusted, semantically reentrant format capability stored in a frozen registry.
///
/// Implementations may not retain per-session parse or option state. An unwind
/// poisons the registration and every snapshot that shares it.
pub trait FormatImporter: Send + Sync + RefUnwindSafe + 'static {
    /// Returns immutable claims captured exactly once when the importer is registered.
    fn descriptor(&self) -> FormatDescriptor;

    /// Returns the object-safe format-owned option codec captured at registration.
    fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
        no_options_codec()
    }

    /// Declares the concrete options type expected by this parser.
    fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        no_options_runtime_type_key()
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::NoMatch)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft>;
}

#[derive(Clone, PartialEq, Eq)]
struct InlineDetectionFaultDetail {
    bytes: [u8; MAX_DETECTION_FAULT_DETAIL_BYTES],
    len: u8,
}

impl InlineDetectionFaultDetail {
    fn new(value: &str) -> Self {
        let mut bytes = [0_u8; MAX_DETECTION_FAULT_DETAIL_BYTES];
        let mut len = 0_usize;
        for character in value.chars() {
            let character = if character.is_control() {
                '\u{fffd}'
            } else {
                character
            };
            let mut encoded = [0_u8; 4];
            let encoded = character.encode_utf8(&mut encoded).as_bytes();
            let Some(next_len) = len.checked_add(encoded.len()) else {
                break;
            };
            if next_len > MAX_DETECTION_FAULT_DETAIL_BYTES {
                break;
            }
            bytes[len..next_len].copy_from_slice(encoded);
            len = next_len;
        }
        Self {
            bytes,
            len: len as u8,
        }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .unwrap_or("failure detail unavailable")
    }
}

impl fmt::Debug for InlineDetectionFaultDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(formatter)
    }
}

fn sanitized_prefix_len(value: &str, maximum: usize) -> usize {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'.len_utf8()
            } else {
                character.len_utf8()
            }
        })
        .try_fold(0_usize, |total, bytes| {
            let next = total.checked_add(bytes)?;
            (next <= maximum).then_some(next)
        })
        .unwrap_or(maximum)
}

fn push_sanitized_prefix(output: &mut String, value: &str, maximum: usize) {
    for character in value.chars() {
        let character = if character.is_control() {
            '\u{fffd}'
        } else {
            character
        };
        if output.len() + character.len_utf8() > maximum {
            break;
        }
        output.push(character);
    }
}

pub(crate) fn bounded_panic_detail(payload: Box<dyn Any + Send>) -> BoundedFaultDetail {
    let payload = match payload.downcast::<&'static str>() {
        Ok(message) => return BoundedFaultDetail::new(*message),
        Err(payload) => payload,
    };
    let payload = match payload.downcast::<String>() {
        Ok(message) => return BoundedFaultDetail::new(message.as_str()),
        Err(payload) => payload,
    };

    discard_untrusted_panic_payload(payload);
    BoundedFaultDetail::new("extension panicked with a non-string payload")
}

pub(crate) fn discard_untrusted_panic_payload(payload: Box<dyn Any + Send>) {
    if payload.is::<String>() || payload.is::<&'static str>() {
        drop(payload);
    } else {
        // Extension-defined payloads may run arbitrary Drop code, including another panic.
        std::mem::forget(payload);
    }
}

pub(crate) fn detection_fault_from_panic(payload: Box<dyn Any + Send>) -> DetectionFault {
    let payload = match payload.downcast::<&'static str>() {
        Ok(message) => return DetectionFault::extension_panicked(*message),
        Err(payload) => payload,
    };
    let payload = match payload.downcast::<String>() {
        Ok(message) => return DetectionFault::extension_panicked(message.as_str()),
        Err(payload) => payload,
    };

    discard_untrusted_panic_payload(payload);
    DetectionFault::extension_panicked("extension panicked with a non-string payload")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_object_safe(_: &dyn FormatImporter) {}

    #[test]
    fn detection_fault_detail_is_bounded_and_sanitized() {
        let detail = format!("{}\nsecret", "x".repeat(MAX_DETECTION_FAULT_DETAIL_BYTES));
        let DetectionOutcome::Fault(fault) = DetectionOutcome::fault(detail) else {
            panic!("expected a fault");
        };
        assert!(fault.detail().len() <= MAX_DETECTION_FAULT_DETAIL_BYTES);
        assert!(!fault.detail().contains('\n'));
    }

    #[test]
    fn panic_detail_reads_only_string_payloads_and_is_bounded() {
        struct PanickingDrop;

        impl Drop for PanickingDrop {
            fn drop(&mut self) {
                panic!("opaque payload destructor must not run");
            }
        }

        let string_payload: Box<dyn Any + Send> = Box::new(format!(
            "panic\n{}",
            "x".repeat(MAX_EXTENSION_PANIC_DETAIL_BYTES)
        ));
        let string_detail = bounded_panic_detail(string_payload);
        assert!(string_detail.as_ref().len() <= MAX_EXTENSION_PANIC_DETAIL_BYTES);
        assert!(!string_detail.as_ref().contains('\n'));

        let opaque_payload: Box<dyn Any + Send> = Box::new(PanickingDrop);
        assert_eq!(
            bounded_panic_detail(opaque_payload).as_ref(),
            "extension panicked with a non-string payload"
        );
    }

    #[test]
    fn trait_remains_object_safe() {
        let _ = assert_object_safe as fn(&dyn FormatImporter);
    }

    #[test]
    fn detection_fault_evidence_is_inline_and_small() {
        assert!(!std::mem::needs_drop::<DetectionFault>());
        assert!(std::mem::size_of::<DetectionFault>() <= 128);
    }
}
