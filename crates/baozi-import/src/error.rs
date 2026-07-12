use crate::dependency::DependencyPolicyError;
use crate::descriptor::FormatDescriptor;
use crate::diagnostic::{DiagnosticCause, DiagnosticCode, valid_detail_key};
use crate::ledger::ImportDimension;
use baozi_core::{FormatId, SceneBuildError, SceneValidationError};
use baozi_io::{AssetOpenError, AssetReadError, PathError};
use baozi_operation::{CapacityPlan, CapacityPlanError, ControlCause, LedgerCause};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CauseOwner {
    Io,
    DependencyPolicy,
    Operation,
    Import,
    Core,
    Format(FormatId),
    TrustedExtension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustedExtensionBoundary {
    AssetIo,
    DependencyResolver,
    Detector,
    Importer,
    DiagnosticCallback,
    FormatFailureDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DetectionFailure {
    ForcedFormatContradiction {
        format: FormatId,
    },
    ForcedDetectorFault {
        format: FormatId,
    },
    NoSelection {
        had_match: bool,
        had_inconclusive: bool,
        had_fault: bool,
    },
    Ambiguous {
        candidates: Vec<FormatId>,
    },
}

impl fmt::Display for DetectionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForcedFormatContradiction { format } => {
                write!(
                    formatter,
                    "forced format {format} contradicted the primary input"
                )
            }
            Self::ForcedDetectorFault { format } => {
                write!(formatter, "forced format detector {format} failed")
            }
            Self::NoSelection { .. } => {
                formatter.write_str("format detection produced evidence but no selectable format")
            }
            Self::Ambiguous { candidates } => {
                formatter.write_str("format detection is ambiguous among ")?;
                for (index, candidate) in candidates.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    candidate.fmt(formatter)?;
                }
                Ok(())
            }
        }
    }
}

impl Error for DetectionFailure {}

#[derive(Debug)]
#[non_exhaustive]
pub enum ScopeAbortCause {
    Path(PathError),
    AssetOpen(AssetOpenError),
    AssetRead(AssetReadError),
    DependencyPolicy(DependencyPolicyError),
    Ledger(LedgerCause<ImportDimension>),
    Control(ControlCause),
    Diagnostic(DiagnosticCause),
    FormatRecoveryRejectedByConformance {
        code: DiagnosticCode,
    },
    FormatRecoveryRejectedByLossPolicy {
        code: DiagnosticCode,
    },
    FormatFailureDetail(FormatFailureDetailCause),
    SceneBuild(SceneBuildError),
    SceneAccounting(SceneValidationError),
    ResourceLimit {
        resource: &'static str,
        limit: u64,
    },
    Allocation {
        resource: &'static str,
    },
    OptionTypeMismatch,
    DescriptorContract {
        code: &'static str,
    },
    TrustedExtension {
        boundary: TrustedExtensionBoundary,
        code: &'static str,
    },
}

impl ScopeAbortCause {
    pub fn owner(&self) -> CauseOwner {
        match self {
            Self::Path(_) | Self::AssetOpen(_) | Self::AssetRead(_) => CauseOwner::Io,
            Self::DependencyPolicy(_) => CauseOwner::DependencyPolicy,
            Self::Control(_) => CauseOwner::Operation,
            Self::SceneBuild(_) | Self::SceneAccounting(_) => CauseOwner::Core,
            Self::TrustedExtension { .. } => CauseOwner::TrustedExtension,
            Self::Ledger(_)
            | Self::Diagnostic(_)
            | Self::FormatRecoveryRejectedByConformance { .. }
            | Self::FormatRecoveryRejectedByLossPolicy { .. }
            | Self::FormatFailureDetail(_)
            | Self::ResourceLimit { .. }
            | Self::Allocation { .. }
            | Self::OptionTypeMismatch
            | Self::DescriptorContract { .. } => CauseOwner::Import,
        }
    }
}

impl fmt::Display for ScopeAbortCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => error.fmt(formatter),
            Self::AssetOpen(error) => error.fmt(formatter),
            Self::AssetRead(error) => error.fmt(formatter),
            Self::DependencyPolicy(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Control(error) => error.fmt(formatter),
            Self::Diagnostic(error) => error.fmt(formatter),
            Self::FormatRecoveryRejectedByConformance { code } => write!(
                formatter,
                "format recovery {} was rejected by the active conformance contract",
                code.as_str()
            ),
            Self::FormatRecoveryRejectedByLossPolicy { code } => write!(
                formatter,
                "format recovery {} was rejected by loss policy",
                code.as_str()
            ),
            Self::FormatFailureDetail(error) => error.fmt(formatter),
            Self::SceneBuild(error) => error.fmt(formatter),
            Self::SceneAccounting(error) => error.fmt(formatter),
            Self::ResourceLimit { resource, limit } => {
                write!(formatter, "resource {resource} exceeds limit {limit}")
            }
            Self::Allocation { resource } => {
                write!(formatter, "allocation failed for {resource}")
            }
            Self::OptionTypeMismatch => formatter.write_str("selected format option type mismatch"),
            Self::DescriptorContract { code } => {
                write!(
                    formatter,
                    "selected format descriptor contract failed: {code}"
                )
            }
            Self::TrustedExtension { boundary, code } => {
                write!(
                    formatter,
                    "trusted extension failed at {boundary:?}: {code}"
                )
            }
        }
    }
}

impl Error for ScopeAbortCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::AssetOpen(error) => Some(error),
            Self::AssetRead(error) => Some(error),
            Self::DependencyPolicy(error) => Some(error),
            Self::Ledger(error) => Some(error),
            Self::Control(error) => Some(error),
            Self::Diagnostic(error) => Some(error),
            Self::FormatFailureDetail(error) => Some(error),
            Self::SceneBuild(error) => Some(error),
            Self::SceneAccounting(error) => Some(error),
            Self::OptionTypeMismatch
            | Self::FormatRecoveryRejectedByConformance { .. }
            | Self::FormatRecoveryRejectedByLossPolicy { .. }
            | Self::ResourceLimit { .. }
            | Self::Allocation { .. }
            | Self::DescriptorContract { .. }
            | Self::TrustedExtension { .. } => None,
        }
    }
}

/// The opaque owner of a terminal capability failure moved out of one parse scope.
#[derive(Debug)]
pub struct ScopeAbort {
    cause: ScopeAbortCause,
}

impl ScopeAbort {
    pub(crate) const fn new(cause: ScopeAbortCause) -> Self {
        Self { cause }
    }

    pub fn owner(&self) -> CauseOwner {
        self.cause.owner()
    }

    pub fn cause(&self) -> &ScopeAbortCause {
        &self.cause
    }
}

impl fmt::Display for ScopeAbort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl Error for ScopeAbort {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

pub trait FormatFailureDetailSink {
    fn string(&mut self, key: &'static str, value: &str);
    fn unsigned(&mut self, key: &'static str, value: u64);
    fn signed(&mut self, key: &'static str, value: i64);
}

const MAX_FORMAT_FAILURE_DETAILS: usize = 32;
const MAX_FORMAT_FAILURE_DETAIL_BYTES: usize = 4 * 1024;
const MAX_FORMAT_FAILURE_STRING_BYTES: usize = 512;
pub(crate) const MAX_FORMAT_FAILURE_SOURCE_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineFormatFailureDetailValue {
    String { offset: u16, len: u16 },
    Unsigned(u64),
    Signed(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InlineFormatFailureDetail {
    key: &'static str,
    value: InlineFormatFailureDetailValue,
}

impl InlineFormatFailureDetail {
    const EMPTY: Self = Self {
        key: "",
        value: InlineFormatFailureDetailValue::Unsigned(0),
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FormatFailureDetailString<'a> {
    value: &'a str,
}

impl<'a> FormatFailureDetailString<'a> {
    pub const fn as_str(self) -> &'a str {
        self.value
    }
}

impl AsRef<str> for FormatFailureDetailString<'_> {
    fn as_ref(&self) -> &str {
        self.value
    }
}

impl fmt::Display for FormatFailureDetailString<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.value)
    }
}

impl fmt::Debug for FormatFailureDetailString<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.value, formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatFailureDetailValueRef<'a> {
    String(FormatFailureDetailString<'a>),
    Unsigned(u64),
    Signed(i64),
}

pub type FormatFailureDetailValue<'a> = FormatFailureDetailValueRef<'a>;

#[derive(Clone, Copy)]
pub struct FormatFailureDetail<'a> {
    entry: &'a InlineFormatFailureDetail,
    string_bytes: &'a [u8],
}

impl<'a> FormatFailureDetail<'a> {
    pub const fn key(self) -> &'static str {
        self.entry.key
    }

    pub fn value(self) -> FormatFailureDetailValueRef<'a> {
        match self.entry.value {
            InlineFormatFailureDetailValue::String { offset, len } => {
                let start = usize::from(offset);
                let end = start.saturating_add(usize::from(len));
                let bytes = self.string_bytes.get(start..end).unwrap_or_default();
                let value = std::str::from_utf8(bytes).unwrap_or_default();
                FormatFailureDetailValueRef::String(FormatFailureDetailString { value })
            }
            InlineFormatFailureDetailValue::Unsigned(value) => {
                FormatFailureDetailValueRef::Unsigned(value)
            }
            InlineFormatFailureDetailValue::Signed(value) => {
                FormatFailureDetailValueRef::Signed(value)
            }
        }
    }
}

impl fmt::Debug for FormatFailureDetail<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FormatFailureDetail")
            .field("key", &self.key())
            .field("value", &self.value())
            .finish()
    }
}

pub struct FormatFailureDetails<'a> {
    details: [FormatFailureDetail<'a>; MAX_FORMAT_FAILURE_DETAILS],
    len: u8,
}

impl<'a> FormatFailureDetails<'a> {
    fn new(entries: &'a [InlineFormatFailureDetail], string_bytes: &'a [u8]) -> Self {
        let empty = FormatFailureDetail {
            entry: &InlineFormatFailureDetail::EMPTY,
            string_bytes: &[],
        };
        let mut details = [empty; MAX_FORMAT_FAILURE_DETAILS];
        for (detail, entry) in details.iter_mut().zip(entries) {
            *detail = FormatFailureDetail {
                entry,
                string_bytes,
            };
        }
        Self {
            details,
            len: u8::try_from(entries.len()).unwrap_or_default(),
        }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<FormatFailureDetail<'a>> {
        self.as_slice().get(index).copied()
    }

    pub fn iter(&self) -> std::iter::Copied<std::slice::Iter<'_, FormatFailureDetail<'a>>> {
        self.as_slice().iter().copied()
    }

    pub fn as_slice(&self) -> &[FormatFailureDetail<'a>] {
        self.details.get(..self.len()).unwrap_or_default()
    }
}

impl fmt::Debug for FormatFailureDetails<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<'a> std::ops::Index<usize> for FormatFailureDetails<'a> {
    type Output = FormatFailureDetail<'a>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl<'a, 'details> IntoIterator for &'details FormatFailureDetails<'a> {
    type Item = FormatFailureDetail<'a>;
    type IntoIter = std::iter::Copied<std::slice::Iter<'details, FormatFailureDetail<'a>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatFailureDetailCause {
    InvalidKey,
    CountExceeded,
    BytesExceeded,
    StringExceeded,
    AllocationFailed,
    CapacityPlan(CapacityPlanError),
}

impl fmt::Display for FormatFailureDetailCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "format failure detail rejected: {self:?}")
    }
}

impl Error for FormatFailureDetailCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CapacityPlan(cause) => Some(cause),
            Self::InvalidKey
            | Self::CountExceeded
            | Self::BytesExceeded
            | Self::StringExceeded
            | Self::AllocationFailed => None,
        }
    }
}

impl From<CapacityPlanError> for FormatFailureDetailCause {
    fn from(cause: CapacityPlanError) -> Self {
        Self::CapacityPlan(cause)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormatFailureDetailCapacityPlan {
    entries: CapacityPlan,
    string_bytes: CapacityPlan,
}

impl FormatFailureDetailCapacityPlan {
    fn new() -> Result<Self, CapacityPlanError> {
        Ok(Self {
            entries: CapacityPlan::for_type::<InlineFormatFailureDetail>(
                0,
                0,
                MAX_FORMAT_FAILURE_DETAILS,
            )?,
            string_bytes: CapacityPlan::for_type::<u8>(0, 0, MAX_FORMAT_FAILURE_DETAIL_BYTES)?,
        })
    }

    pub(crate) fn envelope_bytes(self) -> Result<u64, CapacityPlanError> {
        self.entries
            .envelope_delta_bytes()?
            .checked_add(self.string_bytes.envelope_delta_bytes()?)
            .ok_or(CapacityPlanError::CapacityOverflow)
    }

    fn finish(self, details: &PackedFormatFailureDetails) -> Result<u64, CapacityPlanError> {
        self.entries
            .finish(details.entries.len(), details.entries.capacity())?
            .checked_add(
                self.string_bytes
                    .finish(details.string_bytes.len(), details.string_bytes.capacity())?,
            )
            .ok_or(CapacityPlanError::CapacityOverflow)
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct PackedFormatFailureDetails {
    entries: Vec<InlineFormatFailureDetail>,
    string_bytes: Vec<u8>,
}

impl PackedFormatFailureDetails {
    fn try_allocate(
        plan: FormatFailureDetailCapacityPlan,
    ) -> Result<(Self, u64), FormatFailureDetailCause> {
        let mut entries = Vec::new();
        plan.entries
            .verify_preflight(entries.len(), entries.capacity())?;
        entries
            .try_reserve_exact(plan.entries.allocator_additional())
            .map_err(|_| FormatFailureDetailCause::AllocationFailed)?;

        let mut string_bytes = Vec::new();
        plan.string_bytes
            .verify_preflight(string_bytes.len(), string_bytes.capacity())?;
        string_bytes
            .try_reserve_exact(plan.string_bytes.allocator_additional())
            .map_err(|_| FormatFailureDetailCause::AllocationFailed)?;

        let details = Self {
            entries,
            string_bytes,
        };
        let actual = plan.finish(&details)?;
        Ok((details, actual))
    }

    fn push_scalar(
        &mut self,
        key: &'static str,
        value: InlineFormatFailureDetailValue,
    ) -> Result<(), FormatFailureDetailCause> {
        self.entries
            .try_reserve(1)
            .map_err(|_| FormatFailureDetailCause::AllocationFailed)?;
        self.entries.push(InlineFormatFailureDetail { key, value });
        Ok(())
    }

    fn push_string(
        &mut self,
        key: &'static str,
        value: &str,
        sanitized_len: usize,
    ) -> Result<(), FormatFailureDetailCause> {
        self.entries
            .try_reserve(1)
            .map_err(|_| FormatFailureDetailCause::AllocationFailed)?;
        self.string_bytes
            .try_reserve(sanitized_len)
            .map_err(|_| FormatFailureDetailCause::AllocationFailed)?;

        let offset = u16::try_from(self.string_bytes.len())
            .map_err(|_| FormatFailureDetailCause::BytesExceeded)?;
        let len =
            u16::try_from(sanitized_len).map_err(|_| FormatFailureDetailCause::StringExceeded)?;
        let mut encoded = [0; 4];
        for character in value.chars() {
            let character = if character.is_control() {
                '\u{fffd}'
            } else {
                character
            };
            let encoded = character.encode_utf8(&mut encoded);
            self.string_bytes.extend_from_slice(encoded.as_bytes());
        }
        self.entries.push(InlineFormatFailureDetail {
            key,
            value: InlineFormatFailureDetailValue::String { offset, len },
        });
        Ok(())
    }

    fn view(&self) -> FormatFailureDetails<'_> {
        FormatFailureDetails::new(&self.entries, &self.string_bytes)
    }
}

impl fmt::Debug for PackedFormatFailureDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.view().fmt(formatter)
    }
}

pub(crate) struct BoundedFormatFailureDetailSink {
    details: PackedFormatFailureDetails,
    bytes: usize,
    cause: Option<FormatFailureDetailCause>,
}

impl BoundedFormatFailureDetailSink {
    pub(crate) fn capacity_plan() -> Result<FormatFailureDetailCapacityPlan, CapacityPlanError> {
        FormatFailureDetailCapacityPlan::new()
    }

    pub(crate) fn try_allocate(
        plan: FormatFailureDetailCapacityPlan,
    ) -> Result<(Self, u64), FormatFailureDetailCause> {
        let (details, actual_capacity_bytes) = PackedFormatFailureDetails::try_allocate(plan)?;
        Ok((
            Self {
                details,
                bytes: 0,
                cause: None,
            },
            actual_capacity_bytes,
        ))
    }

    pub(crate) fn finish(self) -> Result<PackedFormatFailureDetails, FormatFailureDetailCause> {
        match self.cause {
            Some(cause) => Err(cause),
            None => Ok(self.details),
        }
    }

    fn admit(&mut self, key: &'static str, bytes: usize) -> Option<usize> {
        if self.cause.is_some() {
            return None;
        }
        if !valid_detail_key(key) {
            self.cause = Some(FormatFailureDetailCause::InvalidKey);
            return None;
        }
        if self.details.entries.len() >= MAX_FORMAT_FAILURE_DETAILS {
            self.cause = Some(FormatFailureDetailCause::CountExceeded);
            return None;
        }
        let Some(next_bytes) = self
            .bytes
            .checked_add(key.len())
            .and_then(|n| n.checked_add(bytes))
        else {
            self.cause = Some(FormatFailureDetailCause::BytesExceeded);
            return None;
        };
        if next_bytes > MAX_FORMAT_FAILURE_DETAIL_BYTES {
            self.cause = Some(FormatFailureDetailCause::BytesExceeded);
            return None;
        }
        Some(next_bytes)
    }

    fn push_scalar(&mut self, key: &'static str, value: InlineFormatFailureDetailValue) {
        let Some(next_bytes) = self.admit(key, 8) else {
            return;
        };
        if let Err(cause) = self.details.push_scalar(key, value) {
            self.cause = Some(cause);
            return;
        }
        self.bytes = next_bytes;
    }
}

impl FormatFailureDetailSink for BoundedFormatFailureDetailSink {
    fn string(&mut self, key: &'static str, value: &str) {
        if self.cause.is_some() {
            return;
        }
        let sanitized_len = match sanitized_detail_string_len(value) {
            Ok(len) => len,
            Err(cause) => {
                self.cause = Some(cause);
                return;
            }
        };
        let Some(next_bytes) = self.admit(key, sanitized_len) else {
            return;
        };
        if let Err(cause) = self.details.push_string(key, value, sanitized_len) {
            self.cause = Some(cause);
            return;
        }
        self.bytes = next_bytes;
    }

    fn unsigned(&mut self, key: &'static str, value: u64) {
        self.push_scalar(key, InlineFormatFailureDetailValue::Unsigned(value));
    }

    fn signed(&mut self, key: &'static str, value: i64) {
        self.push_scalar(key, InlineFormatFailureDetailValue::Signed(value));
    }
}

fn sanitized_detail_string_len(value: &str) -> Result<usize, FormatFailureDetailCause> {
    if value.len() > MAX_FORMAT_FAILURE_STRING_BYTES {
        return Err(FormatFailureDetailCause::StringExceeded);
    }
    let mut len = 0usize;
    for character in value.chars() {
        let bytes = if character.is_control() {
            '\u{fffd}'.len_utf8()
        } else {
            character.len_utf8()
        };
        len = len
            .checked_add(bytes)
            .ok_or(FormatFailureDetailCause::StringExceeded)?;
        if len > MAX_FORMAT_FAILURE_STRING_BYTES {
            return Err(FormatFailureDetailCause::StringExceeded);
        }
    }
    Ok(len)
}

/// A format-owned source for stable failure evidence.
///
/// The import boundary validates `code` and bounded `visit_details` output before retaining the
/// source as the direct standard error source of [`FormatFailureEnvelope`]. Backend-native types
/// remain private behind the format-owned source chain.
pub trait FormatFailureSource: Error + Send + Sync + 'static {
    fn code(&self) -> &'static str;
    fn visit_details(&self, _sink: &mut dyn FormatFailureDetailSink) {}
}

pub struct FormatFailureEnvelope {
    format: Arc<FormatDescriptor>,
    code: &'static str,
    details: PackedFormatFailureDetails,
    source: Box<dyn FormatFailureSource>,
}

impl FormatFailureEnvelope {
    pub(crate) fn seal<S>(
        format: Arc<FormatDescriptor>,
        code: &'static str,
        details: PackedFormatFailureDetails,
        source: S,
    ) -> Self
    where
        S: FormatFailureSource,
    {
        Self {
            format,
            code,
            details,
            source: Box::new(source),
        }
    }

    pub fn format(&self) -> &FormatId {
        self.format.id()
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub fn details(&self) -> FormatFailureDetails<'_> {
        self.details.view()
    }

    /// Returns the retained format-owned source when its concrete type is `T`.
    pub fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: FormatFailureSource,
    {
        let source: &(dyn Error + Send + Sync + 'static) = self.source.as_ref();
        source.downcast_ref::<T>()
    }
}

impl fmt::Debug for FormatFailureEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FormatFailureEnvelope")
            .field("format", &self.format())
            .field("code", &self.code)
            .field("details", &self.details)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for FormatFailureEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "format {} failed with {}",
            self.format.id(),
            self.code
        )
    }
}

impl Error for FormatFailureEnvelope {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ImportCause {
    Scope(ScopeAbort),
    Format(FormatFailureEnvelope),
    Validation(SceneValidationError),
    TrustedExtension {
        boundary: TrustedExtensionBoundary,
        code: &'static str,
    },
    UnsupportedFormat,
    Detection(DetectionFailure),
}

impl ImportCause {
    pub fn owner(&self) -> CauseOwner {
        match self {
            Self::Scope(abort) => abort.owner(),
            Self::Format(envelope) => CauseOwner::Format(envelope.format().clone()),
            Self::Validation(_) => CauseOwner::Core,
            Self::TrustedExtension { .. } => CauseOwner::TrustedExtension,
            Self::UnsupportedFormat | Self::Detection(_) => CauseOwner::Import,
        }
    }
}

impl fmt::Display for ImportCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope(abort) => abort.fmt(formatter),
            Self::Format(envelope) => envelope.fmt(formatter),
            Self::Validation(error) => error.fmt(formatter),
            Self::TrustedExtension { boundary, code } => {
                write!(
                    formatter,
                    "trusted extension failed at {boundary:?}: {code}"
                )
            }
            Self::UnsupportedFormat => formatter.write_str("no registered format matched input"),
            Self::Detection(failure) => failure.fmt(formatter),
        }
    }
}

impl Error for ImportCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scope(abort) => Some(abort),
            Self::Format(envelope) => Some(envelope),
            Self::Validation(error) => Some(error),
            Self::TrustedExtension { .. } | Self::UnsupportedFormat => None,
            Self::Detection(failure) => Some(failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_sink() -> BoundedFormatFailureDetailSink {
        let plan = BoundedFormatFailureDetailSink::capacity_plan()
            .expect("fixed format failure capacities must be plannable");
        let envelope = plan
            .envelope_bytes()
            .expect("fixed format failure envelope must fit u64");
        let (sink, actual) = BoundedFormatFailureDetailSink::try_allocate(plan)
            .expect("test format failure storage must allocate");
        assert!(actual <= envelope);
        sink
    }

    fn strict_text_profile() -> crate::descriptor::TextProfile {
        use crate::descriptor::{
            TextBomPolicy, TextControlPolicy, TextDecodingMode, TextHardCaps, TextLexicalMode,
            TextNewline, TextProfile, TextSourceEncoding,
        };

        TextProfile::builder(
            TextHardCaps::new(1024, 1024, 1024, 1024, 1024)
                .expect("static packed-failure text hard caps must be valid"),
        )
        .source_encodings([TextSourceEncoding::Utf8])
        .bom_policy(TextBomPolicy::Forbidden)
        .newlines([TextNewline::Lf])
        .lexical_mode(TextLexicalMode::Utf8Document)
        .decoding_modes([TextDecodingMode::RejectInvalid])
        .control_policy(TextControlPolicy::RejectAll)
        .build()
        .expect("static packed-failure text profile must be valid")
    }

    #[derive(Debug)]
    struct PackedFailureSource;

    impl fmt::Display for PackedFailureSource {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("packed failure source")
        }
    }

    impl Error for PackedFailureSource {}

    impl FormatFailureSource for PackedFailureSource {
        fn code(&self) -> &'static str {
            "test.inline_failure"
        }
    }

    #[test]
    fn detail_sink_accepts_exactly_the_maximum_entry_count() {
        let mut sink = new_sink();
        let entry_capacity = sink.details.entries.capacity();
        let string_capacity = sink.details.string_bytes.capacity();
        for value in 0..MAX_FORMAT_FAILURE_DETAILS {
            sink.unsigned("value", value as u64);
        }
        assert_eq!(sink.details.entries.capacity(), entry_capacity);
        assert_eq!(sink.details.string_bytes.capacity(), string_capacity);

        let details = sink.finish().expect("the maximum count must be accepted");
        let view = details.view();
        assert_eq!(view.len(), MAX_FORMAT_FAILURE_DETAILS);
        assert!(matches!(
            view.get(MAX_FORMAT_FAILURE_DETAILS - 1).map(FormatFailureDetail::value),
            Some(FormatFailureDetailValue::Unsigned(value))
                if value == (MAX_FORMAT_FAILURE_DETAILS - 1) as u64
        ));

        let mut overflowing = new_sink();
        for value in 0..=MAX_FORMAT_FAILURE_DETAILS {
            overflowing.unsigned("value", value as u64);
        }
        assert!(matches!(
            overflowing.finish(),
            Err(FormatFailureDetailCause::CountExceeded)
        ));
    }

    #[test]
    fn detail_string_preserves_the_exact_utf8_limit_without_allocation() {
        let value = format!("{}{}", "界".repeat(170), "é");
        assert_eq!(value.len(), MAX_FORMAT_FAILURE_STRING_BYTES);

        let mut sink = new_sink();
        sink.string("message", &value);
        let details = sink
            .finish()
            .expect("the exact string limit must be accepted");
        let FormatFailureDetailValue::String(stored) =
            details.view().get(0).expect("stored string detail").value()
        else {
            panic!("expected a string detail")
        };

        assert_eq!(stored.as_str(), value);
        assert_eq!(stored.as_ref(), value);
        assert_eq!(stored.to_string(), value);
        assert_eq!(format!("{stored:?}"), format!("{value:?}"));
        assert_eq!(stored.as_str().len(), MAX_FORMAT_FAILURE_STRING_BYTES);
    }

    #[test]
    fn detail_string_replaces_controls_at_complete_utf8_boundaries() {
        let value = format!("{}{}", "\u{0001}".repeat(170), "é");
        let expected = format!("{}{}", "\u{fffd}".repeat(170), "é");
        assert_eq!(expected.len(), MAX_FORMAT_FAILURE_STRING_BYTES);

        let mut sink = new_sink();
        sink.string("message", &value);
        let details = sink
            .finish()
            .expect("expanded controls that exactly fit must be accepted");
        let FormatFailureDetailValue::String(stored) =
            details.view().get(0).expect("stored string detail").value()
        else {
            panic!("expected a string detail")
        };

        assert_eq!(stored.as_str(), expected);
        assert!(
            stored
                .as_str()
                .chars()
                .all(|character| !character.is_control())
        );
    }

    #[test]
    fn detail_string_rejects_overlong_input_and_incomplete_expansion() {
        let mut overlong = new_sink();
        overlong.string("message", &"a".repeat(MAX_FORMAT_FAILURE_STRING_BYTES + 1));
        assert!(matches!(
            overlong.finish(),
            Err(FormatFailureDetailCause::StringExceeded)
        ));

        let expansion = format!("{}{}", "\u{0001}".repeat(170), "界");
        assert!(expansion.len() < MAX_FORMAT_FAILURE_STRING_BYTES);
        let mut incomplete = new_sink();
        incomplete.string("message", &expansion);
        incomplete.unsigned("ignored_after_failure", 1);
        assert!(matches!(
            incomplete.finish(),
            Err(FormatFailureDetailCause::StringExceeded)
        ));
    }

    #[test]
    fn detail_sink_preserves_key_and_aggregate_byte_limits() {
        let mut invalid_key = new_sink();
        invalid_key.unsigned("Invalid", 1);
        assert!(matches!(
            invalid_key.finish(),
            Err(FormatFailureDetailCause::InvalidKey)
        ));

        const MAX_KEY: &str = "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk";
        assert_eq!(MAX_KEY.len(), 64);
        let value = "a".repeat(MAX_FORMAT_FAILURE_STRING_BYTES);
        let mut aggregate = new_sink();
        for _ in 0..7 {
            aggregate.string(MAX_KEY, &value);
        }
        aggregate.string(MAX_KEY, &value);
        assert!(matches!(
            aggregate.finish(),
            Err(FormatFailureDetailCause::BytesExceeded)
        ));
    }

    #[test]
    fn failure_transport_stays_below_the_large_error_threshold() {
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(std::mem::size_of::<PackedFormatFailureDetails>(), 48);
            assert_eq!(std::mem::size_of::<FormatFailureEnvelope>(), 88);
        }
        assert_eq!(
            std::mem::size_of::<ScopeAbort>(),
            std::mem::size_of::<ScopeAbortCause>()
        );
        assert!(std::mem::size_of::<ScopeAbort>() < 128);
        assert!(std::mem::size_of::<FormatFailureEnvelope>() < 128);
        assert!(std::mem::size_of::<crate::scope::ParseFailure>() < 128);
        assert!(std::mem::size_of::<ImportCause>() < 128);
    }

    #[test]
    fn sealed_envelope_moves_packed_stable_evidence_without_reallocation() {
        let mut sink = new_sink();
        sink.signed("offset", -7);
        let details = sink.finish().expect("valid details must finish");
        let entry_pointer = details.entries.as_ptr();
        let entry_capacity = details.entries.capacity();
        let string_pointer = details.string_bytes.as_ptr();
        let string_capacity = details.string_bytes.capacity();
        let descriptor = Arc::new(
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.packed-failure").expect("valid format id"),
                "Packed failure test",
            )
            .extensions(["packed-failure"])
            .encoding(crate::descriptor::FormatEncoding::Text)
            .text_profile(strict_text_profile())
            .capabilities([(
                crate::descriptor::FormatCapability::Geometry,
                crate::descriptor::CapabilityStatus::Supported,
            )])
            .build()
            .expect("valid descriptor"),
        );
        let injected_descriptor = Arc::clone(&descriptor);
        let descriptor_strong_count = Arc::strong_count(&descriptor);
        let envelope = FormatFailureEnvelope::seal(
            injected_descriptor,
            "test.inline_failure",
            details,
            PackedFailureSource,
        );

        assert!(!std::mem::needs_drop::<FormatFailureDetailString<'static>>());
        assert!(std::mem::needs_drop::<PackedFormatFailureDetails>());
        assert!(std::mem::needs_drop::<FormatFailureEnvelope>());
        assert!(Arc::ptr_eq(&envelope.format, &descriptor));
        assert_eq!(Arc::strong_count(&descriptor), descriptor_strong_count);
        assert_eq!(envelope.format(), descriptor.id());
        assert_eq!(envelope.details.entries.as_ptr(), entry_pointer);
        assert_eq!(envelope.details.entries.capacity(), entry_capacity);
        assert_eq!(envelope.details.string_bytes.as_ptr(), string_pointer);
        assert_eq!(envelope.details.string_bytes.capacity(), string_capacity);
        assert_eq!(envelope.code(), "test.inline_failure");
        assert_eq!(envelope.details().len(), 1);
        let detail_view = envelope.details();
        assert_eq!(detail_view.as_slice().len(), 1);
        let detail = detail_view.get(0).expect("sealed detail");
        assert_eq!(detail.key(), "offset");
        assert!(matches!(
            detail.value(),
            FormatFailureDetailValue::Signed(-7)
        ));
        assert!(
            Error::source(&envelope)
                .and_then(|source| source.downcast_ref::<PackedFailureSource>())
                .is_some()
        );
        assert!(envelope.downcast_ref::<PackedFailureSource>().is_some());
        let repeated_detail = envelope.details().get(0).expect("repeated detail read");
        assert_eq!(repeated_detail.key(), detail.key());
        assert_eq!(repeated_detail.value(), detail.value());
    }
}
