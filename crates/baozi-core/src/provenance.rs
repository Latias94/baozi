use std::num::NonZeroU64;

use thiserror::Error;

use crate::extension::ExtensionSchemaId;
use crate::namespaced::NamespacedName;

/// Maximum encoded length of a sanitized resource identity.
pub const MAX_RESOURCE_IDENTITY_LEN: usize = 512;

/// Maximum encoded length of a format-owned source locator value.
pub const MAX_SOURCE_LOCATOR_LEN: usize = 256;

/// Maximum encoded length of a sanitized source display name.
pub const MAX_SOURCE_DISPLAY_NAME_LEN: usize = 128;

/// Encoded length of every supported cryptographic digest.
pub const DIGEST_LEN: usize = 32;

/// Maximum number of durable evidence summaries on one operation node.
pub const MAX_OPERATION_EVIDENCE_SUMMARIES: usize = 32;

/// A bounded, sanitized logical identity for an imported resource.
///
/// This value is deliberately not an operating-system path or URI. Relative
/// logical hierarchy separated by `/` is accepted, but absolute roots,
/// Windows drive and UNC forms, traversal segments, URI authorities, query
/// strings, fragments, and control characters are rejected.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceIdentity(String);

impl ResourceIdentity {
    /// Validates and copies one logical resource identity.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ResourceIdentityError> {
        let value = value.as_ref();
        validate_resource_identity(value)?;

        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| ResourceIdentityError::AllocationFailed)?;
        owned.push_str(value);
        Ok(Self(owned))
    }

    /// Returns the validated logical identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the backing allocation capacity for caller-owned storage accounting.
    pub const fn capacity_bytes(&self) -> usize {
        self.0.capacity()
    }

    /// Fallibly copies this heap-backed identity.
    pub fn try_clone(&self) -> Result<Self, ResourceIdentityError> {
        Self::new(self.as_str())
    }
}

impl AsRef<str> for ResourceIdentity {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for ResourceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ResourceIdentity")
            .field(&self.as_str())
            .finish()
    }
}

impl std::fmt::Display for ResourceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ResourceIdentity {
    type Error = ResourceIdentityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ResourceIdentity {
    type Error = ResourceIdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Why a logical resource identity was not safe to publish.
///
/// Errors report only a category, length, or byte offset. They never echo the
/// rejected value, which may contain a host path or credential-bearing URI.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ResourceIdentityError {
    #[error("resource identity must not be empty")]
    Empty,
    #[error("resource identity is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },
    #[error("resource identity contains a control character at byte offset {offset}")]
    ControlCharacter { offset: usize },
    #[error("resource identity must not contain surrounding whitespace")]
    SurroundingWhitespace,
    #[error("resource identity must not be an absolute host path")]
    AbsoluteHostPath,
    #[error("resource identity must not use a Windows drive path")]
    WindowsDrivePath,
    #[error("resource identity must not use a UNC or device path")]
    UncPath,
    #[error("resource identity must use canonical '/' separators")]
    NonCanonicalSeparator,
    #[error("resource identity must not contain empty, current, or parent path segments")]
    NonCanonicalSegment,
    #[error("resource identity must not contain a URI authority")]
    UriAuthority,
    #[error("resource identity must not contain a query string or fragment")]
    QueryOrFragment,
    #[error("resource identity allocation failed")]
    AllocationFailed,
}

pub(crate) fn validate_resource_identity(value: &str) -> Result<(), ResourceIdentityError> {
    if value.is_empty() {
        return Err(ResourceIdentityError::Empty);
    }
    if value.len() > MAX_RESOURCE_IDENTITY_LEN {
        return Err(ResourceIdentityError::TooLong {
            actual: value.len(),
            maximum: MAX_RESOURCE_IDENTITY_LEN,
        });
    }
    if value.trim() != value {
        return Err(ResourceIdentityError::SurroundingWhitespace);
    }
    if let Some((offset, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(ResourceIdentityError::ControlCharacter { offset });
    }

    let bytes = value.as_bytes();
    if value.starts_with("//") || value.starts_with("\\\\") {
        return Err(ResourceIdentityError::UncPath);
    }
    if value.starts_with('/') || value.starts_with('\\') {
        return Err(ResourceIdentityError::AbsoluteHostPath);
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(ResourceIdentityError::WindowsDrivePath);
    }
    if value.contains('\\') {
        return Err(ResourceIdentityError::NonCanonicalSeparator);
    }
    if has_uri_authority(value) {
        return Err(ResourceIdentityError::UriAuthority);
    }
    if value.bytes().any(|byte| matches!(byte, b'?' | b'#')) {
        return Err(ResourceIdentityError::QueryOrFragment);
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(ResourceIdentityError::NonCanonicalSegment);
    }
    Ok(())
}

fn has_uri_authority(value: &str) -> bool {
    let Some(separator) = value.find("://") else {
        return false;
    };
    let scheme = &value[..separator];
    let mut bytes = scheme.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// A bounded display-only source name that has passed public-text checks.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceDisplayName(String);

impl SourceDisplayName {
    /// Validates and copies a display-only source name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, SourceDisplayNameError> {
        let value = value.as_ref();
        validate_source_display_name(value)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| SourceDisplayNameError::AllocationFailed)?;
        owned.push_str(value);
        Ok(Self(owned))
    }

    /// Returns the sanitized display text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Fallibly copies this heap-backed display value.
    pub fn try_clone(&self) -> Result<Self, SourceDisplayNameError> {
        Self::new(self.as_str())
    }
}

impl AsRef<str> for SourceDisplayName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for SourceDisplayName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SourceDisplayName")
            .field(&self.as_str())
            .finish()
    }
}

impl std::fmt::Display for SourceDisplayName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why source display text was not safe to retain.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceDisplayNameError {
    #[error("source display name must not be empty")]
    Empty,
    #[error("source display name is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },
    #[error("source display name contains a control character at byte offset {offset}")]
    ControlCharacter { offset: usize },
    #[error("source display name must not contain surrounding whitespace")]
    SurroundingWhitespace,
    #[error("source display name contains a host-path or URI form")]
    HostIdentity,
    #[error("source display name allocation failed")]
    AllocationFailed,
}

pub(crate) fn validate_source_display_name(value: &str) -> Result<(), SourceDisplayNameError> {
    if value.is_empty() {
        return Err(SourceDisplayNameError::Empty);
    }
    if value.len() > MAX_SOURCE_DISPLAY_NAME_LEN {
        return Err(SourceDisplayNameError::TooLong {
            actual: value.len(),
            maximum: MAX_SOURCE_DISPLAY_NAME_LEN,
        });
    }
    if value.trim() != value {
        return Err(SourceDisplayNameError::SurroundingWhitespace);
    }
    if let Some((offset, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(SourceDisplayNameError::ControlCharacter { offset });
    }
    if is_host_or_uri_form(value) {
        return Err(SourceDisplayNameError::HostIdentity);
    }
    Ok(())
}

fn is_host_or_uri_form(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || value.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || has_uri_authority(value)
        || value.bytes().any(|byte| matches!(byte, b'?' | b'#'))
}

/// A standard source-object category with a stable format-local index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SourceObjectKind {
    Document,
    Scene,
    Node,
    Mesh,
    Primitive,
    Material,
    Image,
    Texture,
    Sampler,
    Skin,
    Animation,
    AnimationChannel,
    Camera,
    Light,
    Vertex,
    Face,
    Element,
    Row,
    Statement,
    Segment,
    Buffer,
}

/// A bounded format-defined locator payload.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLocatorValue(String);

impl SourceLocatorValue {
    /// Validates and copies one format-defined locator payload.
    pub fn new(value: impl AsRef<str>) -> Result<Self, SourceLocatorValueError> {
        let value = value.as_ref();
        validate_source_locator_value(value)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| SourceLocatorValueError::AllocationFailed)?;
        owned.push_str(value);
        Ok(Self(owned))
    }

    /// Returns the format-defined locator payload.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the backing allocation capacity for caller-owned storage accounting.
    pub const fn capacity_bytes(&self) -> usize {
        self.0.capacity()
    }

    /// Fallibly copies this heap-backed locator payload.
    pub fn try_clone(&self) -> Result<Self, SourceLocatorValueError> {
        Self::new(self.as_str())
    }
}

impl AsRef<str> for SourceLocatorValue {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for SourceLocatorValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SourceLocatorValue")
            .field(&self.as_str())
            .finish()
    }
}

impl std::fmt::Display for SourceLocatorValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a format-defined source locator payload was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceLocatorValueError {
    #[error("source locator value must not be empty")]
    Empty,
    #[error("source locator value is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },
    #[error("source locator value contains a control character at byte offset {offset}")]
    ControlCharacter { offset: usize },
    #[error("source locator value contains a host-path or URI form")]
    HostIdentity,
    #[error("source locator value allocation failed")]
    AllocationFailed,
}

pub(crate) fn validate_source_locator_value(value: &str) -> Result<(), SourceLocatorValueError> {
    if value.is_empty() {
        return Err(SourceLocatorValueError::Empty);
    }
    if value.len() > MAX_SOURCE_LOCATOR_LEN {
        return Err(SourceLocatorValueError::TooLong {
            actual: value.len(),
            maximum: MAX_SOURCE_LOCATOR_LEN,
        });
    }
    if let Some((offset, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(SourceLocatorValueError::ControlCharacter { offset });
    }
    if is_host_or_uri_form(value) {
        return Err(SourceLocatorValueError::HostIdentity);
    }
    Ok(())
}

/// A validated format-owned source-object locator.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamespacedSourceObjectLocator {
    schema_id: ExtensionSchemaId,
    value: SourceLocatorValue,
}

impl NamespacedSourceObjectLocator {
    /// Creates a locator only when its format-owned schema was admitted.
    pub fn try_new(
        schema_id: Option<ExtensionSchemaId>,
        value: SourceLocatorValue,
    ) -> Result<Self, NamespacedSourceObjectLocatorError> {
        let schema_id = schema_id.ok_or(NamespacedSourceObjectLocatorError::SchemaRequired)?;
        Ok(Self { schema_id, value })
    }

    /// Returns the format-owned locator kind.
    pub const fn kind(&self) -> &NamespacedName {
        self.schema_id.name()
    }

    /// Returns the declarative schema that defines this locator kind and payload.
    pub const fn schema_id(&self) -> &ExtensionSchemaId {
        &self.schema_id
    }

    /// Returns the bounded locator payload.
    pub const fn value(&self) -> &SourceLocatorValue {
        &self.value
    }

    /// Fallibly copies this locator without hiding its payload allocation.
    pub fn try_clone(&self) -> Result<Self, SourceLocatorValueError> {
        Ok(Self {
            schema_id: self.schema_id.clone(),
            value: self.value.try_clone()?,
        })
    }
}

/// Why a format-owned source-object locator could not be constructed.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NamespacedSourceObjectLocatorError {
    #[error("namespaced source-object locator requires an admitted extension schema")]
    SchemaRequired,
}

/// A deterministic source-object identity within one format resource.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceObjectLocator {
    /// A standard object category and stable format-local index.
    Indexed { kind: SourceObjectKind, index: u64 },
    /// A format-owned extension locator with a validated kind and payload.
    Namespaced(NamespacedSourceObjectLocator),
}

impl SourceObjectLocator {
    /// Creates a standard indexed locator.
    pub const fn indexed(kind: SourceObjectKind, index: u64) -> Self {
        Self::Indexed { kind, index }
    }

    /// Creates a format-owned namespaced locator backed by a declarative schema.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
        value: SourceLocatorValue,
    ) -> Result<Self, NamespacedSourceObjectLocatorError> {
        NamespacedSourceObjectLocator::try_new(schema_id, value).map(Self::Namespaced)
    }

    /// Returns the schema for a format-owned locator, when present.
    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Indexed { .. } => None,
            Self::Namespaced(locator) => Some(locator.schema_id()),
        }
    }

    /// Returns the bounded payload for a format-owned locator, when present.
    pub const fn namespaced_value(&self) -> Option<&SourceLocatorValue> {
        match self {
            Self::Indexed { .. } => None,
            Self::Namespaced(locator) => Some(locator.value()),
        }
    }

    /// Fallibly copies a locator when it owns a namespaced text payload.
    pub fn try_clone(&self) -> Result<Self, SourceLocatorValueError> {
        match self {
            Self::Indexed { kind, index } => Ok(Self::Indexed {
                kind: *kind,
                index: *index,
            }),
            Self::Namespaced(locator) => locator.try_clone().map(Self::Namespaced),
        }
    }
}

/// The cryptographic algorithm attached to a fixed-size digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    Sha256,
    Blake3,
}

/// A typed digest of source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest {
    algorithm: DigestAlgorithm,
    bytes: [u8; DIGEST_LEN],
}

impl ContentDigest {
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self { algorithm, bytes }
    }

    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self::new(DigestAlgorithm::Sha256, bytes)
    }

    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self::new(DigestAlgorithm::Blake3, bytes)
    }

    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.bytes
    }
}

/// A typed digest of canonical operation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionsDigest(ContentDigest);

impl OptionsDigest {
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::new(algorithm, bytes))
    }

    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::sha256(bytes))
    }

    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::blake3(bytes))
    }

    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.0.algorithm()
    }

    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        self.0.as_bytes()
    }
}

/// A typed digest covering durable operation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvidenceDigest(ContentDigest);

impl EvidenceDigest {
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::new(algorithm, bytes))
    }

    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::sha256(bytes))
    }

    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::blake3(bytes))
    }

    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.0.algorithm()
    }

    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        self.0.as_bytes()
    }
}

/// A standard or format-owned transformation category.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum OperationKind {
    Generated,
    Default,
    Normalize,
    Repair,
    Split,
    Merge,
    Convert,
    Transform,
    Optimize,
    Namespaced(NamespacedName),
}

/// A bounded durable summary of evidence owned by an operation node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationEvidenceSummary {
    code: NamespacedName,
    occurrence_count: NonZeroU64,
    digest: Option<EvidenceDigest>,
}

impl OperationEvidenceSummary {
    pub fn new(
        code: NamespacedName,
        occurrence_count: u64,
        digest: Option<EvidenceDigest>,
    ) -> Result<Self, OperationEvidenceSummaryError> {
        let occurrence_count = NonZeroU64::new(occurrence_count)
            .ok_or(OperationEvidenceSummaryError::ZeroOccurrences)?;
        Ok(Self {
            code,
            occurrence_count,
            digest,
        })
    }

    pub const fn code(&self) -> &NamespacedName {
        &self.code
    }

    pub const fn occurrence_count(&self) -> NonZeroU64 {
        self.occurrence_count
    }

    pub const fn digest(&self) -> Option<&EvidenceDigest> {
        self.digest.as_ref()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OperationEvidenceSummaryError {
    #[error("operation evidence summary must represent at least one occurrence")]
    ZeroOccurrences,
}

/// A deterministic identity for one provenance-producing operation.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct OperationIdentity {
    kind: OperationKind,
    options_digest: OptionsDigest,
}

impl OperationIdentity {
    pub fn new(kind: OperationKind, options_digest: OptionsDigest) -> Self {
        Self {
            kind,
            options_digest,
        }
    }

    pub const fn kind(&self) -> &OperationKind {
        &self.kind
    }

    pub const fn options_digest(&self) -> &OptionsDigest {
        &self.options_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_identity_rejects_host_specific_forms_without_echoing_input() {
        for value in [
            "/private/model.glb",
            "C:/private/model.glb",
            "https://host/model",
        ] {
            let error = ResourceIdentity::new(value).expect_err("host identity must be rejected");
            assert!(!error.to_string().contains(value));
        }
    }
}
