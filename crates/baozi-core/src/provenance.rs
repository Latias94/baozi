use crate::FormatId;
use crate::extension::ExtensionSchemaId;
use crate::namespaced::NamespacedName;
use crate::shared_storage::{SharedStorage, SharedStorageError, SharedStorageFootprintError};
use baozi_operation::{CapacityPlan, GrowableCapacityPlan};
use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;
use thiserror::Error;

#[cfg(test)]
use crate::process_nonce::fail_next_process_nonce_for_test;
use crate::process_nonce::issue_process_nonce;

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

/// Maximum number of direct parents on one operation node.
pub const MAX_PROVENANCE_PARENTS_PER_OPERATION: usize = 4_096;

/// Maximum provenance attachments retained by one canonical owner.
pub const MAX_PROVENANCE_ATTACHMENTS_PER_OWNER: usize = 4_096;

/// Maximum number of nodes in one scene-owned provenance arena.
pub const MAX_PROVENANCE_NODES: usize = 1_048_576;

/// Maximum number of parent edges in one scene-owned provenance arena.
pub const MAX_PROVENANCE_EDGES: usize = 4_194_304;

/// Maximum canonical lineage depth in one provenance arena.
pub const MAX_PROVENANCE_DEPTH: usize = 65_536;

/// Maximum validation work charged as one unit per node and parent edge.
pub const MAX_PROVENANCE_VALIDATION_WORK: usize = 5_242_880;

/// Maximum retained provenance allocation bytes in one arena.
pub const MAX_PROVENANCE_RETAINED_BYTES: usize = 1_073_741_824;

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

    /// Fallibly copies this heap-backed identity.
    pub fn try_clone(&self) -> Result<Self, ResourceIdentityError> {
        Self::new(self.as_str())
    }

    fn retained_allocation_bytes(&self) -> usize {
        self.0.capacity()
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

fn validate_resource_identity(value: &str) -> Result<(), ResourceIdentityError> {
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

    fn retained_allocation_bytes(&self) -> usize {
        self.0.capacity()
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

fn validate_source_display_name(value: &str) -> Result<(), SourceDisplayNameError> {
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

    /// Fallibly copies this heap-backed locator payload.
    pub fn try_clone(&self) -> Result<Self, SourceLocatorValueError> {
        Self::new(self.as_str())
    }

    fn retained_allocation_bytes(&self) -> usize {
        self.0.capacity()
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

fn validate_source_locator_value(value: &str) -> Result<(), SourceLocatorValueError> {
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
///
/// This is deliberately separate from source identity: equal source
/// identities can carry different content digests after a source edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest {
    algorithm: DigestAlgorithm,
    bytes: [u8; DIGEST_LEN],
}

impl ContentDigest {
    /// Creates a digest for one supported algorithm.
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self { algorithm, bytes }
    }

    /// Creates a SHA-256 content digest.
    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self::new(DigestAlgorithm::Sha256, bytes)
    }

    /// Creates a BLAKE3 content digest.
    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self::new(DigestAlgorithm::Blake3, bytes)
    }

    /// Returns the digest algorithm.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    /// Returns the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.bytes
    }
}

/// A typed digest of canonical operation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionsDigest(ContentDigest);

impl OptionsDigest {
    /// Creates an options digest for one supported algorithm.
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::new(algorithm, bytes))
    }

    /// Creates a SHA-256 options digest.
    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::sha256(bytes))
    }

    /// Creates a BLAKE3 options digest.
    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::blake3(bytes))
    }

    /// Returns the digest algorithm.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.0.algorithm()
    }

    /// Returns the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        self.0.as_bytes()
    }
}

/// A typed digest covering durable operation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvidenceDigest(ContentDigest);

impl EvidenceDigest {
    /// Creates an evidence digest for one supported algorithm.
    pub const fn new(algorithm: DigestAlgorithm, bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::new(algorithm, bytes))
    }

    /// Creates a SHA-256 evidence digest.
    pub const fn sha256(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::sha256(bytes))
    }

    /// Creates a BLAKE3 evidence digest.
    pub const fn blake3(bytes: [u8; DIGEST_LEN]) -> Self {
        Self(ContentDigest::blake3(bytes))
    }

    /// Returns the digest algorithm.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.0.algorithm()
    }

    /// Returns the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        self.0.as_bytes()
    }
}

/// A stable source identity with optional non-identity display and byte proof.
pub struct SourceIdentity {
    resource: ResourceIdentity,
    format: FormatId,
    locator: SourceObjectLocator,
    display_name: Option<SourceDisplayName>,
    content_digest: Option<ContentDigest>,
}

impl SourceIdentity {
    /// Creates the stable portion of one source identity.
    pub const fn new(
        resource: ResourceIdentity,
        format: FormatId,
        locator: SourceObjectLocator,
    ) -> Self {
        Self {
            resource,
            format,
            locator,
            display_name: None,
            content_digest: None,
        }
    }

    /// Adds display-only source text.
    pub fn with_display_name(mut self, display_name: SourceDisplayName) -> Self {
        self.display_name = Some(display_name);
        self
    }

    /// Adds an independent digest of the observed resource bytes.
    pub const fn with_content_digest(mut self, content_digest: ContentDigest) -> Self {
        self.content_digest = Some(content_digest);
        self
    }

    /// Returns the sanitized resource identity.
    pub const fn resource(&self) -> &ResourceIdentity {
        &self.resource
    }

    /// Returns the stable source format.
    pub const fn format(&self) -> &FormatId {
        &self.format
    }

    /// Returns the format-owned object locator.
    pub const fn locator(&self) -> &SourceObjectLocator {
        &self.locator
    }

    /// Returns optional display-only source text.
    pub const fn display_name(&self) -> Option<&SourceDisplayName> {
        self.display_name.as_ref()
    }

    /// Returns the independent digest of observed source bytes.
    pub const fn content_digest(&self) -> Option<&ContentDigest> {
        self.content_digest.as_ref()
    }

    /// Fallibly copies all heap-backed identity and display payloads.
    pub fn try_clone(&self) -> Result<Self, SourceIdentityCloneError> {
        let resource = self
            .resource
            .try_clone()
            .map_err(|_| SourceIdentityCloneError::Resource)?;
        let locator = self
            .locator
            .try_clone()
            .map_err(|_| SourceIdentityCloneError::Locator)?;
        let display_name = self
            .display_name
            .as_ref()
            .map(SourceDisplayName::try_clone)
            .transpose()
            .map_err(|_| SourceIdentityCloneError::DisplayName)?;
        Ok(Self {
            resource,
            format: self.format.clone(),
            locator,
            display_name,
            content_digest: self.content_digest,
        })
    }

    fn checked_dynamic_allocation_bytes(&self) -> Result<usize, ProvenanceFootprintError> {
        let mut bytes = self.resource.retained_allocation_bytes();
        if let SourceObjectLocator::Namespaced(locator) = &self.locator {
            bytes = checked_add_footprint(bytes, locator.value.retained_allocation_bytes())?;
        }
        if let Some(display_name) = &self.display_name {
            bytes = checked_add_footprint(bytes, display_name.retained_allocation_bytes())?;
        }
        Ok(bytes)
    }
}

/// Why an explicit deep copy of a source identity failed.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceIdentityCloneError {
    #[error("source resource identity allocation failed")]
    Resource,

    #[error("source locator allocation failed")]
    Locator,

    #[error("source display-name allocation failed")]
    DisplayName,
}

impl std::fmt::Debug for SourceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourceIdentity")
            .field("resource", &self.resource)
            .field("format", &self.format)
            .field("locator", &self.locator)
            .field("display_name", &self.display_name)
            .field("content_digest", &self.content_digest)
            .finish()
    }
}

impl PartialEq for SourceIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.resource == other.resource
            && self.format == other.format
            && self.locator == other.locator
    }
}

impl Eq for SourceIdentity {}

impl Hash for SourceIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.resource.hash(state);
        self.format.hash(state);
        self.locator.hash(state);
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
///
/// The code is namespaced, the count is non-zero, and an optional digest can
/// bind richer external evidence without storing a dangling report index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationEvidenceSummary {
    code: NamespacedName,
    occurrence_count: NonZeroU64,
    digest: Option<EvidenceDigest>,
}

impl OperationEvidenceSummary {
    /// Creates one durable evidence summary.
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

    /// Returns the stable namespaced evidence code.
    pub const fn code(&self) -> &NamespacedName {
        &self.code
    }

    /// Returns the number of represented occurrences.
    pub const fn occurrence_count(&self) -> NonZeroU64 {
        self.occurrence_count
    }

    /// Returns the optional digest of richer durable evidence.
    pub const fn digest(&self) -> Option<&EvidenceDigest> {
        self.digest.as_ref()
    }
}

/// Why an operation evidence summary was invalid.
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
    evidence: Vec<OperationEvidenceSummary>,
}

impl OperationIdentity {
    /// Creates an operation identity and canonicalizes evidence by code.
    pub fn new(
        kind: OperationKind,
        options_digest: OptionsDigest,
        evidence: impl IntoIterator<Item = OperationEvidenceSummary>,
    ) -> Result<Self, OperationIdentityError> {
        let mut evidence = evidence.into_iter();
        let (lower, _) = evidence.size_hint();
        if lower > MAX_OPERATION_EVIDENCE_SUMMARIES {
            return Err(OperationIdentityError::EvidenceLimitExceeded {
                maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
            });
        }

        let mut canonical = Vec::new();
        if lower > 0 {
            try_reserve_operation_evidence(&mut canonical, lower)?;
        }

        for summary in evidence.by_ref() {
            let next_len = canonical.len().checked_add(1).ok_or(
                OperationIdentityError::EvidenceLimitExceeded {
                    maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
                },
            )?;
            if next_len > MAX_OPERATION_EVIDENCE_SUMMARIES {
                return Err(OperationIdentityError::EvidenceLimitExceeded {
                    maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
                });
            }
            if canonical.len() == canonical.capacity() {
                try_reserve_operation_evidence(&mut canonical, next_len)?;
            }
            canonical.push(summary);
        }
        canonical.sort_unstable_by(|left, right| left.code.cmp(&right.code));
        if canonical
            .windows(2)
            .any(|pair| pair[0].code == pair[1].code)
        {
            return Err(OperationIdentityError::DuplicateEvidenceCode);
        }

        Ok(Self {
            kind,
            options_digest,
            evidence: canonical,
        })
    }

    /// Creates an operation identity without evidence summaries.
    pub fn without_evidence(kind: OperationKind, options_digest: OptionsDigest) -> Self {
        Self {
            kind,
            options_digest,
            evidence: Vec::new(),
        }
    }

    /// Returns the standard or namespaced operation kind.
    pub const fn kind(&self) -> &OperationKind {
        &self.kind
    }

    /// Returns the digest of canonical operation options.
    pub const fn options_digest(&self) -> &OptionsDigest {
        &self.options_digest
    }

    /// Iterates evidence summaries in canonical code order.
    pub fn evidence(&self) -> impl ExactSizeIterator<Item = &OperationEvidenceSummary> {
        self.evidence.iter()
    }

    /// Fallibly copies the canonical evidence vector.
    pub fn try_clone(&self) -> Result<Self, OperationIdentityError> {
        let mut evidence = Vec::new();
        if !self.evidence.is_empty() {
            try_reserve_operation_evidence(&mut evidence, self.evidence.len())?;
            evidence.extend(self.evidence.iter().cloned());
        }
        Ok(Self {
            kind: self.kind.clone(),
            options_digest: self.options_digest,
            evidence,
        })
    }

    pub(crate) fn checked_dynamic_allocation_bytes(
        &self,
    ) -> Result<usize, ProvenanceFootprintError> {
        checked_capacity_bytes::<OperationEvidenceSummary>(self.evidence.capacity())
    }
}

fn try_reserve_operation_evidence(
    evidence: &mut Vec<OperationEvidenceSummary>,
    desired: usize,
) -> Result<(), OperationIdentityError> {
    if desired <= evidence.capacity() {
        return Ok(());
    }
    let plan = CapacityPlan::for_type::<OperationEvidenceSummary>(
        evidence.len(),
        evidence.capacity(),
        desired,
    )
    .map_err(|_| OperationIdentityError::AllocationFailed)?;
    evidence
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| OperationIdentityError::AllocationFailed)?;
    plan.finish(evidence.len(), evidence.capacity())
        .map_err(|_| OperationIdentityError::AllocationFailed)?;
    Ok(())
}

/// Why an operation identity could not be created.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OperationIdentityError {
    #[error("operation identity exceeds the evidence-summary limit of {maximum}")]
    EvidenceLimitExceeded { maximum: usize },

    #[error("operation identity contains a duplicate evidence code")]
    DuplicateEvidenceCode,

    #[error("operation evidence allocation failed")]
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ProvenanceNodeId(usize);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ArenaNonce(NonZeroU64);

#[cfg(test)]
fn fail_next_arena_nonce_for_test() {
    fail_next_process_nonce_for_test();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArenaNonceError {
    Exhausted,
}

fn next_arena_nonce() -> Result<ArenaNonce, ArenaNonceError> {
    issue_process_nonce()
        .map(ArenaNonce)
        .map_err(|_| ArenaNonceError::Exhausted)
}

/// An opaque, non-empty attachment to one immutable provenance node.
///
/// The segment nonce and node address are intentionally private and absent
/// from every public accessor and debug representation. Forks preserve
/// ancestor attachments by identity while assigning fresh nonces only to
/// branch-local segments.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Provenance {
    arena: ArenaNonce,
    segment: usize,
    node: ProvenanceNodeId,
    topological_index: usize,
}

impl std::fmt::Debug for Provenance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provenance")
            .field("attachment", &"opaque")
            .finish()
    }
}

/// An ordered, non-empty set of provenance attachments for one owner.
///
/// The first attachment is stored inline, so the common single-source case
/// performs no heap allocation. Additional attachments retain deterministic
/// producer order in separately bounded storage.
#[derive(PartialEq, Eq, Hash)]
pub struct ProvenanceAttachments {
    first: Provenance,
    additional: Vec<Provenance>,
}

impl ProvenanceAttachments {
    /// Fallibly copies an ordered, non-empty attachment sequence.
    pub fn try_from_iter(
        attachments: impl IntoIterator<Item = Provenance>,
    ) -> Result<Self, ProvenanceAttachmentsError> {
        let mut attachments = attachments.into_iter();
        let first = attachments
            .next()
            .ok_or(ProvenanceAttachmentsError::Empty)?;
        let maximum_additional = MAX_PROVENANCE_ATTACHMENTS_PER_OWNER - 1;
        let (lower, _) = attachments.size_hint();
        if lower > maximum_additional {
            return Err(ProvenanceAttachmentsError::LimitExceeded {
                maximum: MAX_PROVENANCE_ATTACHMENTS_PER_OWNER,
            });
        }

        let mut additional = Vec::new();
        try_reserve_provenance_attachments(&mut additional, lower)?;

        for attachment in attachments {
            if additional.len() == maximum_additional {
                return Err(ProvenanceAttachmentsError::LimitExceeded {
                    maximum: MAX_PROVENANCE_ATTACHMENTS_PER_OWNER,
                });
            }
            let desired = additional.len().checked_add(1).ok_or(
                ProvenanceAttachmentsError::LimitExceeded {
                    maximum: MAX_PROVENANCE_ATTACHMENTS_PER_OWNER,
                },
            )?;
            try_reserve_provenance_attachments(&mut additional, desired)?;
            additional.push(attachment);
        }

        Ok(Self { first, additional })
    }

    /// Returns the first mandatory attachment.
    pub const fn first(&self) -> Provenance {
        self.first
    }

    /// Returns the number of attachments.
    pub fn len(&self) -> usize {
        1 + self.additional.len()
    }

    /// Iterates over every attachment in deterministic producer order.
    pub fn iter(&self) -> ProvenanceAttachmentsIter<'_> {
        ProvenanceAttachmentsIter {
            first: Some(self.first),
            additional: self.additional.iter(),
        }
    }

    /// Returns heap cells retained for attachments after the inline first one.
    pub fn retained_allocation_cells(&self) -> usize {
        self.additional.capacity()
    }

    /// Returns heap bytes retained for attachments after the inline first one.
    pub fn retained_allocation_bytes(&self) -> usize {
        self.additional.capacity() * std::mem::size_of::<Provenance>()
    }
}

/// An exact iterator over one owner's ordered provenance attachments.
pub struct ProvenanceAttachmentsIter<'a> {
    first: Option<Provenance>,
    additional: std::slice::Iter<'a, Provenance>,
}

impl Iterator for ProvenanceAttachmentsIter<'_> {
    type Item = Provenance;

    fn next(&mut self) -> Option<Self::Item> {
        self.first
            .take()
            .or_else(|| self.additional.next().copied())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl DoubleEndedIterator for ProvenanceAttachmentsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.additional
            .next_back()
            .copied()
            .or_else(|| self.first.take())
    }
}

impl ExactSizeIterator for ProvenanceAttachmentsIter<'_> {
    fn len(&self) -> usize {
        usize::from(self.first.is_some()) + self.additional.len()
    }
}

impl std::iter::FusedIterator for ProvenanceAttachmentsIter<'_> {}

impl<'a> IntoIterator for &'a ProvenanceAttachments {
    type Item = Provenance;
    type IntoIter = ProvenanceAttachmentsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Provenance> for ProvenanceAttachments {
    fn from(first: Provenance) -> Self {
        Self {
            first,
            additional: Vec::new(),
        }
    }
}

impl std::fmt::Debug for ProvenanceAttachments {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvenanceAttachments")
            .field("count", &self.len())
            .finish()
    }
}

/// Why a non-empty owner attachment set could not be created.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceAttachmentsError {
    #[error("provenance attachments cannot be empty")]
    Empty,

    #[error("provenance attachment count exceeds the maximum of {maximum}")]
    LimitExceeded { maximum: usize },

    #[error("provenance attachment allocation failed")]
    AllocationFailed,
}

fn try_reserve_provenance_attachments(
    attachments: &mut Vec<Provenance>,
    desired: usize,
) -> Result<(), ProvenanceAttachmentsError> {
    if desired <= attachments.capacity() {
        return Ok(());
    }
    let plan = GrowableCapacityPlan::for_type::<Provenance>(
        attachments.len(),
        attachments.capacity(),
        desired,
    )
    .map_err(|_| ProvenanceAttachmentsError::AllocationFailed)?;
    attachments
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| ProvenanceAttachmentsError::AllocationFailed)?;
    plan.finish(attachments.len(), attachments.capacity())
        .map_err(|_| ProvenanceAttachmentsError::AllocationFailed)?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ProvenanceNode {
    depth: usize,
    payload: ProvenanceNodePayload,
}

#[derive(Debug, PartialEq, Eq)]
enum ProvenanceNodePayload {
    Source(SourceIdentity),
    Operation {
        identity: OperationIdentity,
        parents: Vec<Provenance>,
    },
}

struct ProvenanceSegment {
    nonce: ArenaNonce,
    ordinal: usize,
    start: usize,
    nodes: Vec<SharedStorage<ProvenanceNode>>,
    retained_node_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ProvenanceSegmentSnapshot {
    nonce: ArenaNonce,
    ordinal: usize,
    start: usize,
    node_count: usize,
}

fn try_clone_segments_for_fork(
    segments: &[SharedStorage<ProvenanceSegment>],
) -> Result<Vec<SharedStorage<ProvenanceSegment>>, ProvenanceForkError> {
    let desired_capacity = segments
        .len()
        .checked_add(1)
        .ok_or(ProvenanceForkError::AccountingOverflow)?;
    let capacity_plan =
        CapacityPlan::for_type::<SharedStorage<ProvenanceSegment>>(0, 0, desired_capacity)
            .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
    let mut cloned = Vec::new();
    try_reserve_for_fork(
        &mut cloned,
        capacity_plan.allocator_additional(),
        ProvenanceAllocation::ForkSegments,
    )?;
    capacity_plan.finish(0, cloned.capacity()).map_err(|_| {
        ProvenanceForkError::AllocationFailed {
            resource: ProvenanceAllocation::ForkSegments,
        }
    })?;
    cloned.extend(segments.iter().cloned());
    Ok(cloned)
}

fn try_reserve_for_fork<T>(
    values: &mut Vec<T>,
    additional: usize,
    resource: ProvenanceAllocation,
) -> Result<(), ProvenanceForkError> {
    if additional == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if take_injected_fork_reserve_failure() {
        return Err(ProvenanceForkError::AllocationFailed { resource });
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| ProvenanceForkError::AllocationFailed { resource })
}

fn checked_add_footprint(left: usize, right: usize) -> Result<usize, ProvenanceFootprintError> {
    left.checked_add(right)
        .ok_or(ProvenanceFootprintError::AccountingOverflow)
}

fn checked_capacity_bytes<T>(capacity: usize) -> Result<usize, ProvenanceFootprintError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(ProvenanceFootprintError::AccountingOverflow)
}

fn checked_node_retained_bytes(node: &ProvenanceNode) -> Result<usize, ProvenanceFootprintError> {
    let allocation = SharedStorage::<ProvenanceNode>::checked_layout_footprint()
        .map_err(ProvenanceFootprintError::from_shared_storage)?;
    checked_add_footprint(allocation, checked_node_dynamic_bytes(node)?)
}

fn checked_stored_node_retained_bytes(
    node: &SharedStorage<ProvenanceNode>,
) -> Result<usize, ProvenanceFootprintError> {
    Ok(node.retained_allocation_bytes())
}

fn checked_node_dynamic_bytes(node: &ProvenanceNode) -> Result<usize, ProvenanceFootprintError> {
    match &node.payload {
        ProvenanceNodePayload::Source(identity) => identity.checked_dynamic_allocation_bytes(),
        ProvenanceNodePayload::Operation { identity, parents } => checked_add_footprint(
            identity.checked_dynamic_allocation_bytes()?,
            checked_capacity_bytes::<Provenance>(parents.capacity())?,
        ),
    }
}

fn checked_operation_node_retained_bytes(
    identity: &OperationIdentity,
    parent_capacity: usize,
) -> Result<usize, ProvenanceFootprintError> {
    let allocation = SharedStorage::<ProvenanceNode>::checked_layout_footprint()
        .map_err(ProvenanceFootprintError::from_shared_storage)?;
    let dynamic = checked_add_footprint(
        identity.checked_dynamic_allocation_bytes()?,
        checked_capacity_bytes::<Provenance>(parent_capacity)?,
    )?;
    checked_add_footprint(allocation, dynamic)
}

fn checked_segments_retained_footprint(
    segments: &[SharedStorage<ProvenanceSegment>],
    outer_capacity: usize,
    retained_node_bytes: usize,
) -> Result<ProvenanceRetainedFootprint, ProvenanceFootprintError> {
    let mut control_storage_bytes =
        checked_capacity_bytes::<SharedStorage<ProvenanceSegment>>(outer_capacity)?;
    for segment in segments {
        control_storage_bytes =
            checked_add_footprint(control_storage_bytes, segment.retained_allocation_bytes())?;
    }
    ProvenanceRetainedFootprint::try_new(control_storage_bytes, retained_node_bytes)
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_FORK_RESERVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_fork_reserve_for_test() {
    FAIL_NEXT_FORK_RESERVE.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_fork_reserve_failure() -> bool {
    FAIL_NEXT_FORK_RESERVE.with(|flag| flag.replace(false))
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_ARENA_CAPACITY_QUALIFICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_arena_capacity_qualification_for_test() {
    FAIL_NEXT_ARENA_CAPACITY_QUALIFICATION.with(|flag| flag.set(true));
}

#[cfg(test)]
fn arena_capacity_for_qualification(actual: usize, envelope: usize) -> usize {
    if FAIL_NEXT_ARENA_CAPACITY_QUALIFICATION.with(|flag| flag.replace(false)) {
        envelope.saturating_add(1)
    } else {
        actual
    }
}

#[cfg(not(test))]
fn arena_capacity_for_qualification(actual: usize, _envelope: usize) -> usize {
    actual
}

#[derive(Debug, Clone, Copy)]
struct PreparedProvenanceNode {
    retained_node_bytes: usize,
    node_dynamic_bytes: usize,
    local_capacity: CapacityPlan,
    segment_capacity: Option<CapacityPlan>,
}

/// A scene-local builder that creates a topologically ordered provenance DAG.
pub struct ProvenanceArenaBuilder {
    nonce: ArenaNonce,
    segments: Vec<SharedStorage<ProvenanceSegment>>,
    local_nodes: Vec<SharedStorage<ProvenanceNode>>,
    local_retained_node_bytes: usize,
    node_count: usize,
    edge_count: usize,
    retained_node_bytes: usize,
    fork_control_storage_bytes: usize,
    limits: ProvenanceLimits,
}

impl ProvenanceArenaBuilder {
    /// Creates an empty builder with a process-unique opaque arena nonce.
    pub fn try_new() -> Result<Self, ProvenanceBuildError> {
        Self::try_with_limits(ProvenanceLimits::hard())
    }

    /// Creates an empty builder with limits no greater than the hard bounds.
    pub fn try_with_limits(limits: ProvenanceLimits) -> Result<Self, ProvenanceBuildError> {
        Ok(Self {
            nonce: next_arena_nonce().map_err(|ArenaNonceError::Exhausted| {
                ProvenanceBuildError::ArenaIdentityExhausted
            })?,
            segments: Vec::new(),
            local_nodes: Vec::new(),
            local_retained_node_bytes: 0,
            node_count: 0,
            edge_count: 0,
            retained_node_bytes: 0,
            fork_control_storage_bytes: 0,
            limits,
        })
    }

    /// Adds one source leaf and returns its opaque attachment.
    pub fn add_source(
        &mut self,
        identity: SourceIdentity,
    ) -> Result<Provenance, ProvenanceBuildError> {
        let node_value = ProvenanceNode {
            depth: 1,
            payload: ProvenanceNodePayload::Source(identity),
        };
        let node_bytes = checked_node_retained_bytes(&node_value)
            .map_err(ProvenanceBuildError::from_footprint)?;
        let prepared = self.prepare_node(1, self.edge_count, node_bytes)?;
        let node = self.commit_node(node_value, prepared)?;
        Ok(Provenance {
            arena: self.nonce,
            segment: self.segments.len(),
            node,
            topological_index: self.node_count - 1,
        })
    }

    /// Adds one operation after validating and canonicalizing its parents.
    ///
    /// Every parent must be an existing attachment from this builder. Parent
    /// edges are sorted by their private topological address, while duplicate
    /// edges are rejected rather than silently discarded.
    pub fn add_operation<I, P>(
        &mut self,
        identity: OperationIdentity,
        parents: I,
    ) -> Result<Provenance, ProvenanceBuildError>
    where
        I: IntoIterator<Item = P>,
        P: Borrow<Provenance>,
    {
        if identity.evidence.len() > self.limits.max_evidence_summaries {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::OperationEvidenceSummaries,
                maximum: self.limits.max_evidence_summaries,
            });
        }
        let mut parents = parents.into_iter();
        let (lower, _) = parents.size_hint();
        if lower > self.limits.max_parents_per_operation {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::OperationParents,
                maximum: self.limits.max_parents_per_operation,
            });
        }
        let edge_remaining = self
            .limits
            .max_edges
            .checked_sub(self.edge_count)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        if lower > edge_remaining {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaEdges,
                maximum: self.limits.max_edges,
            });
        }

        let mut canonical_parents = Vec::new();
        if lower > 0 {
            self.try_reserve_operation_parents(&identity, &mut canonical_parents, lower)?;
        }

        let mut maximum_parent_depth = 0usize;
        for parent in parents.by_ref() {
            let next_parent_count = canonical_parents
                .len()
                .checked_add(1)
                .ok_or(ProvenanceBuildError::AccountingOverflow)?;
            if next_parent_count > self.limits.max_parents_per_operation {
                return Err(ProvenanceBuildError::LimitExceeded {
                    limit: ProvenanceLimit::OperationParents,
                    maximum: self.limits.max_parents_per_operation,
                });
            }
            let next_edge_count = self
                .edge_count
                .checked_add(next_parent_count)
                .ok_or(ProvenanceBuildError::AccountingOverflow)?;
            if next_edge_count > self.limits.max_edges {
                return Err(ProvenanceBuildError::LimitExceeded {
                    limit: ProvenanceLimit::ArenaEdges,
                    maximum: self.limits.max_edges,
                });
            }

            let parent = *parent.borrow();
            let parent_node = self.resolve_parent(&parent)?;
            maximum_parent_depth = maximum_parent_depth.max(parent_node.depth);
            if canonical_parents.len() == canonical_parents.capacity() {
                let doubled = canonical_parents
                    .capacity()
                    .checked_mul(2)
                    .ok_or(ProvenanceBuildError::AccountingOverflow)?;
                let target = doubled
                    .max(next_parent_count)
                    .min(self.limits.max_parents_per_operation)
                    .min(edge_remaining);
                self.try_reserve_operation_parents(&identity, &mut canonical_parents, target)?;
            }
            canonical_parents.push(parent);
        }

        canonical_parents.sort_unstable_by_key(|parent| parent.topological_index);
        if canonical_parents.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ProvenanceBuildError::DuplicateParent);
        }

        let edge_count = self
            .edge_count
            .checked_add(canonical_parents.len())
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        if edge_count > self.limits.max_edges {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaEdges,
                maximum: self.limits.max_edges,
            });
        }

        let depth = maximum_parent_depth
            .checked_add(1)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        let node_value = ProvenanceNode {
            depth,
            payload: ProvenanceNodePayload::Operation {
                identity,
                parents: canonical_parents,
            },
        };
        let node_bytes = checked_node_retained_bytes(&node_value)
            .map_err(ProvenanceBuildError::from_footprint)?;
        let prepared = self.prepare_node(depth, edge_count, node_bytes)?;
        let node = self.commit_node(node_value, prepared)?;
        self.edge_count = edge_count;
        Ok(Provenance {
            arena: self.nonce,
            segment: self.segments.len(),
            node,
            topological_index: self.node_count - 1,
        })
    }

    /// Returns the number of nodes currently owned by the builder.
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Returns the number of parent edges currently owned by the builder.
    pub const fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Returns the active construction and validation limits.
    pub const fn limits(&self) -> ProvenanceLimits {
        self.limits
    }

    /// Returns checked retained capacity, including shared nodes and coupled fork control storage.
    pub fn retained_footprint(
        &self,
    ) -> Result<ProvenanceRetainedFootprint, ProvenanceFootprintError> {
        let mut footprint = checked_segments_retained_footprint(
            &self.segments,
            self.segments.capacity(),
            self.retained_node_bytes,
        )?;
        if !self.local_nodes.is_empty() {
            let control_storage_bytes = checked_add_footprint(
                footprint.control_storage_bytes(),
                SharedStorage::<ProvenanceSegment>::checked_layout_footprint()
                    .map_err(ProvenanceFootprintError::from_shared_storage)?,
            )?;
            let control_storage_bytes = checked_add_footprint(
                control_storage_bytes,
                checked_capacity_bytes::<SharedStorage<ProvenanceNode>>(
                    self.local_nodes.capacity(),
                )?,
            )?;
            footprint = ProvenanceRetainedFootprint::try_new(
                control_storage_bytes,
                self.retained_node_bytes,
            )?;
        }
        let control_storage_bytes = checked_add_footprint(
            footprint.control_storage_bytes(),
            self.fork_control_storage_bytes,
        )?;
        ProvenanceRetainedFootprint::try_new(control_storage_bytes, self.retained_node_bytes)
    }

    /// Validates and freezes the arena in deterministic topological order.
    pub fn finish(mut self) -> Result<ProvenanceArena, ProvenanceValidationError> {
        if !self.local_nodes.is_empty() {
            let start = self
                .node_count
                .checked_sub(self.local_nodes.len())
                .ok_or_else(|| {
                    ProvenanceValidationError::new(
                        ProvenanceValidationIssue::RetainedFootprintMismatch,
                        ProvenanceValidationStats::empty(),
                    )
                })?;
            let nested_allocation_bytes = checked_capacity_bytes::<SharedStorage<ProvenanceNode>>(
                self.local_nodes.capacity(),
            )
            .map_err(|_| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    ProvenanceValidationStats::empty(),
                )
            })?;
            let segment = SharedStorage::try_new_prepared(
                ProvenanceSegment {
                    nonce: self.nonce,
                    ordinal: self.segments.len(),
                    start,
                    nodes: self.local_nodes,
                    retained_node_bytes: self.local_retained_node_bytes,
                },
                nested_allocation_bytes,
            )
            .map_err(|error| {
                let issue = match error {
                    SharedStorageError::AllocationFailed => {
                        ProvenanceValidationIssue::AllocationFailed
                    }
                    SharedStorageError::AccountingOverflow => {
                        ProvenanceValidationIssue::RetainedFootprintOverflow
                    }
                };
                ProvenanceValidationError::new(issue, ProvenanceValidationStats::empty())
            })?;
            self.segments.push(segment);
        }
        let arena = ProvenanceArena {
            nonce: self.nonce,
            segments: self.segments,
            node_count: self.node_count,
            edge_count: self.edge_count,
            retained_node_bytes: self.retained_node_bytes,
            limits: self.limits,
        };
        validate_provenance_arena(&arena)?;
        Ok(arena)
    }

    fn try_reserve_operation_parents(
        &self,
        identity: &OperationIdentity,
        parents: &mut Vec<Provenance>,
        desired: usize,
    ) -> Result<(), ProvenanceBuildError> {
        if desired <= parents.capacity() {
            return Ok(());
        }
        let plan = GrowableCapacityPlan::for_type::<Provenance>(
            parents.len(),
            parents.capacity(),
            desired,
        )
        .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
        let node_bytes = checked_operation_node_retained_bytes(identity, plan.envelope_capacity())
            .map_err(ProvenanceBuildError::from_footprint)?;
        self.prepare_retained_capacity(node_bytes)?;
        parents
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| ProvenanceBuildError::AllocationFailed {
                resource: ProvenanceAllocation::OperationParents,
            })?;
        plan.finish(parents.len(), parents.capacity())
            .map_err(|_| ProvenanceBuildError::AllocationFailed {
                resource: ProvenanceAllocation::OperationParents,
            })?;
        Ok(())
    }

    fn prepare_node(
        &self,
        depth: usize,
        next_edge_count: usize,
        node_bytes: usize,
    ) -> Result<PreparedProvenanceNode, ProvenanceBuildError> {
        let next_len = self
            .node_count
            .checked_add(1)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        if next_len > self.limits.max_nodes {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaNodes,
                maximum: self.limits.max_nodes,
            });
        }
        if next_edge_count > self.limits.max_edges {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaEdges,
                maximum: self.limits.max_edges,
            });
        }
        if depth > self.limits.max_depth {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaDepth,
                maximum: self.limits.max_depth,
            });
        }
        let validation_work = next_len
            .checked_add(next_edge_count)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        if validation_work > self.limits.max_validation_work {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ValidationWork,
                maximum: self.limits.max_validation_work,
            });
        }
        self.prepare_retained_capacity(node_bytes)
    }

    fn prepare_retained_capacity(
        &self,
        node_bytes: usize,
    ) -> Result<PreparedProvenanceNode, ProvenanceBuildError> {
        let next_len = self
            .node_count
            .checked_add(1)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        if next_len > self.limits.max_nodes {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaNodes,
                maximum: self.limits.max_nodes,
            });
        }
        let retained_node_bytes = self
            .retained_node_bytes
            .checked_add(node_bytes)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        let local_capacity = CapacityPlan::for_type::<SharedStorage<ProvenanceNode>>(
            self.local_nodes.len(),
            self.local_nodes.capacity(),
            self.local_nodes.len() + 1,
        )
        .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
        let segment_capacity = if self.local_nodes.is_empty() {
            Some(
                CapacityPlan::for_type::<SharedStorage<ProvenanceSegment>>(
                    self.segments.len(),
                    self.segments.capacity(),
                    self.segments.len() + 1,
                )
                .map_err(|_| ProvenanceBuildError::AccountingOverflow)?,
            )
        } else {
            None
        };
        let segment_pointer_capacity = segment_capacity
            .as_ref()
            .map_or(self.segments.capacity(), |plan| plan.envelope_capacity());
        let mut outer_bytes =
            checked_capacity_bytes::<SharedStorage<ProvenanceSegment>>(segment_pointer_capacity)
                .map_err(ProvenanceBuildError::from_footprint)?;
        let segment_allocation = SharedStorage::<ProvenanceSegment>::checked_layout_footprint()
            .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
        for segment in &self.segments {
            outer_bytes = outer_bytes
                .checked_add(segment.retained_allocation_bytes())
                .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        }
        outer_bytes = outer_bytes
            .checked_add(segment_allocation)
            .and_then(|bytes| {
                checked_capacity_bytes::<SharedStorage<ProvenanceNode>>(
                    local_capacity.envelope_capacity(),
                )
                .ok()
                .and_then(|local| bytes.checked_add(local))
            })
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        outer_bytes = outer_bytes
            .checked_add(self.fork_control_storage_bytes)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        let projected = ProvenanceRetainedFootprint::try_new(outer_bytes, retained_node_bytes)
            .map_err(ProvenanceBuildError::from_footprint)?;
        if projected.total_bytes() > self.limits.max_retained_bytes {
            return Err(ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: self.limits.max_retained_bytes,
            });
        }
        Ok(PreparedProvenanceNode {
            retained_node_bytes,
            node_dynamic_bytes: node_bytes
                .checked_sub(
                    SharedStorage::<ProvenanceNode>::checked_layout_footprint()
                        .map_err(|_| ProvenanceBuildError::AccountingOverflow)?,
                )
                .ok_or(ProvenanceBuildError::AccountingOverflow)?,
            local_capacity,
            segment_capacity,
        })
    }

    fn commit_node(
        &mut self,
        node_value: ProvenanceNode,
        prepared: PreparedProvenanceNode,
    ) -> Result<ProvenanceNodeId, ProvenanceBuildError> {
        let stored = SharedStorage::try_new_prepared(node_value, prepared.node_dynamic_bytes)
            .map_err(|error| match error {
                SharedStorageError::AllocationFailed => ProvenanceBuildError::AllocationFailed {
                    resource: ProvenanceAllocation::SharedNode,
                },
                SharedStorageError::AccountingOverflow => ProvenanceBuildError::AccountingOverflow,
            })?;

        prepared
            .local_capacity
            .verify_preflight(self.local_nodes.len(), self.local_nodes.capacity())
            .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
        let segment_replacement = if let Some(segment_plan) = prepared.segment_capacity {
            segment_plan
                .verify_preflight(self.segments.len(), self.segments.capacity())
                .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
            if segment_plan.desired_capacity() > self.segments.capacity() {
                let replacement_plan = CapacityPlan::for_type::<SharedStorage<ProvenanceSegment>>(
                    0,
                    0,
                    segment_plan.envelope_capacity(),
                )
                .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
                let mut replacement = Vec::new();
                replacement
                    .try_reserve_exact(replacement_plan.allocator_additional())
                    .map_err(|_| ProvenanceBuildError::AllocationFailed {
                        resource: ProvenanceAllocation::ArenaNodes,
                    })?;
                let qualified_capacity = arena_capacity_for_qualification(
                    replacement.capacity(),
                    replacement_plan.envelope_capacity(),
                );
                replacement_plan
                    .finish(0, qualified_capacity)
                    .map_err(|_| ProvenanceBuildError::AllocationFailed {
                        resource: ProvenanceAllocation::ArenaNodes,
                    })?;
                replacement.extend(self.segments.iter().cloned());
                Some(replacement)
            } else {
                None
            }
        } else {
            None
        };
        let node = ProvenanceNodeId(self.local_nodes.len());
        let mut local_replacement =
            if prepared.local_capacity.desired_capacity() > self.local_nodes.capacity() {
                let replacement_plan = CapacityPlan::for_type::<SharedStorage<ProvenanceNode>>(
                    0,
                    0,
                    prepared.local_capacity.envelope_capacity(),
                )
                .map_err(|_| ProvenanceBuildError::AccountingOverflow)?;
                let mut replacement = Vec::new();
                replacement
                    .try_reserve_exact(replacement_plan.allocator_additional())
                    .map_err(|_| ProvenanceBuildError::AllocationFailed {
                        resource: ProvenanceAllocation::ArenaNodes,
                    })?;
                let qualified_capacity = arena_capacity_for_qualification(
                    replacement.capacity(),
                    replacement_plan.envelope_capacity(),
                );
                replacement_plan
                    .finish(0, qualified_capacity)
                    .map_err(|_| ProvenanceBuildError::AllocationFailed {
                        resource: ProvenanceAllocation::ArenaNodes,
                    })?;
                Some(replacement)
            } else {
                prepared
                    .local_capacity
                    .finish(self.local_nodes.len(), self.local_nodes.capacity())
                    .map_err(|_| ProvenanceBuildError::AllocationFailed {
                        resource: ProvenanceAllocation::ArenaNodes,
                    })?;
                None
            };

        if let Some(replacement) = segment_replacement {
            self.segments = replacement;
        }
        if let Some(mut replacement) = local_replacement.take() {
            replacement.append(&mut self.local_nodes);
            self.local_nodes = replacement;
        }
        self.local_nodes.push(stored);
        let ancestor_retained_node_bytes = self
            .retained_node_bytes
            .checked_sub(self.local_retained_node_bytes)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        self.local_retained_node_bytes = prepared
            .retained_node_bytes
            .checked_sub(ancestor_retained_node_bytes)
            .ok_or(ProvenanceBuildError::AccountingOverflow)?;
        self.node_count += 1;
        self.retained_node_bytes = prepared.retained_node_bytes;
        Ok(node)
    }

    fn resolve_parent(
        &self,
        provenance: &Provenance,
    ) -> Result<&ProvenanceNode, ProvenanceBuildError> {
        if provenance.segment == self.segments.len() {
            if provenance.arena != self.nonce {
                return Err(ProvenanceBuildError::ArenaMismatch);
            }
            let local_start = self
                .node_count
                .checked_sub(self.local_nodes.len())
                .ok_or(ProvenanceBuildError::AccountingOverflow)?;
            let expected_topological_index = local_start
                .checked_add(provenance.node.0)
                .ok_or(ProvenanceBuildError::ParentOutOfRange)?;
            if provenance.topological_index != expected_topological_index {
                return Err(ProvenanceBuildError::ParentOutOfRange);
            }
            let node = self
                .local_nodes
                .get(provenance.node.0)
                .ok_or(ProvenanceBuildError::ParentOutOfRange)?;
            return Ok(node.as_ref());
        }
        let Some(segment) = self.segments.get(provenance.segment) else {
            return Err(ProvenanceBuildError::ArenaMismatch);
        };
        let segment = segment.as_ref();
        if segment.ordinal != provenance.segment || segment.nonce != provenance.arena {
            return Err(ProvenanceBuildError::ArenaMismatch);
        }
        let expected_topological_index = segment
            .start
            .checked_add(provenance.node.0)
            .ok_or(ProvenanceBuildError::ParentOutOfRange)?;
        if provenance.topological_index != expected_topological_index {
            return Err(ProvenanceBuildError::ParentOutOfRange);
        }
        let node = segment
            .nodes
            .get(provenance.node.0)
            .ok_or(ProvenanceBuildError::ParentOutOfRange)?;
        Ok(node.as_ref())
    }
}

impl std::fmt::Debug for ProvenanceArenaBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvenanceArenaBuilder")
            .field("node_count", &self.node_count)
            .field("edge_count", &self.edge_count)
            .finish_non_exhaustive()
    }
}

/// A bounded provenance construction failure.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceBuildError {
    #[error("provenance arena identity space is exhausted")]
    ArenaIdentityExhausted,

    #[error("provenance parent belongs to a different arena")]
    ArenaMismatch,

    #[error("provenance parent is not an existing node in this arena")]
    ParentOutOfRange,

    #[error("provenance operation contains a duplicate parent")]
    DuplicateParent,

    #[error("provenance accounting overflowed")]
    AccountingOverflow,

    #[error("provenance limit {limit:?} exceeded; the maximum is {maximum}")]
    LimitExceeded {
        limit: ProvenanceLimit,
        maximum: usize,
    },

    #[error("provenance allocation failed for {resource:?}")]
    AllocationFailed { resource: ProvenanceAllocation },
}

impl ProvenanceBuildError {
    fn from_footprint(_: ProvenanceFootprintError) -> Self {
        Self::AccountingOverflow
    }
}

/// Configurable provenance bounds constrained by process-wide hard maxima.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProvenanceLimits {
    max_nodes: usize,
    max_edges: usize,
    max_parents_per_operation: usize,
    max_evidence_summaries: usize,
    max_depth: usize,
    max_validation_work: usize,
    max_retained_bytes: usize,
}

impl ProvenanceLimits {
    /// Returns the process-wide hard provenance limits.
    pub const fn hard() -> Self {
        Self {
            max_nodes: MAX_PROVENANCE_NODES,
            max_edges: MAX_PROVENANCE_EDGES,
            max_parents_per_operation: MAX_PROVENANCE_PARENTS_PER_OPERATION,
            max_evidence_summaries: MAX_OPERATION_EVIDENCE_SUMMARIES,
            max_depth: MAX_PROVENANCE_DEPTH,
            max_validation_work: MAX_PROVENANCE_VALIDATION_WORK,
            max_retained_bytes: MAX_PROVENANCE_RETAINED_BYTES,
        }
    }

    /// Creates a stricter limit set without permitting hard-limit expansion.
    #[allow(clippy::too_many_arguments)]
    pub const fn try_new(
        max_nodes: usize,
        max_edges: usize,
        max_parents_per_operation: usize,
        max_evidence_summaries: usize,
        max_depth: usize,
        max_validation_work: usize,
        max_retained_bytes: usize,
    ) -> Result<Self, ProvenanceLimitsError> {
        if max_nodes > MAX_PROVENANCE_NODES {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::ArenaNodes,
                requested: max_nodes,
                maximum: MAX_PROVENANCE_NODES,
            });
        }
        if max_edges > MAX_PROVENANCE_EDGES {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::ArenaEdges,
                requested: max_edges,
                maximum: MAX_PROVENANCE_EDGES,
            });
        }
        if max_parents_per_operation > MAX_PROVENANCE_PARENTS_PER_OPERATION {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::OperationParents,
                requested: max_parents_per_operation,
                maximum: MAX_PROVENANCE_PARENTS_PER_OPERATION,
            });
        }
        if max_evidence_summaries > MAX_OPERATION_EVIDENCE_SUMMARIES {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::OperationEvidenceSummaries,
                requested: max_evidence_summaries,
                maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
            });
        }
        if max_depth > MAX_PROVENANCE_DEPTH {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::ArenaDepth,
                requested: max_depth,
                maximum: MAX_PROVENANCE_DEPTH,
            });
        }
        if max_validation_work > MAX_PROVENANCE_VALIDATION_WORK {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::ValidationWork,
                requested: max_validation_work,
                maximum: MAX_PROVENANCE_VALIDATION_WORK,
            });
        }
        if max_retained_bytes > MAX_PROVENANCE_RETAINED_BYTES {
            return Err(ProvenanceLimitsError::ExceedsHardLimit {
                limit: ProvenanceLimit::RetainedBytes,
                requested: max_retained_bytes,
                maximum: MAX_PROVENANCE_RETAINED_BYTES,
            });
        }
        Ok(Self {
            max_nodes,
            max_edges,
            max_parents_per_operation,
            max_evidence_summaries,
            max_depth,
            max_validation_work,
            max_retained_bytes,
        })
    }

    pub const fn max_nodes(self) -> usize {
        self.max_nodes
    }

    pub const fn max_edges(self) -> usize {
        self.max_edges
    }

    pub const fn max_parents_per_operation(self) -> usize {
        self.max_parents_per_operation
    }

    pub const fn max_evidence_summaries(self) -> usize {
        self.max_evidence_summaries
    }

    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    pub const fn max_validation_work(self) -> usize {
        self.max_validation_work
    }

    pub const fn max_retained_bytes(self) -> usize {
        self.max_retained_bytes
    }
}

impl Default for ProvenanceLimits {
    fn default() -> Self {
        Self::hard()
    }
}

/// Why a configurable provenance limit set was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceLimitsError {
    #[error("provenance limit {limit:?} requested {requested}, exceeding hard maximum {maximum}")]
    ExceedsHardLimit {
        limit: ProvenanceLimit,
        requested: usize,
        maximum: usize,
    },
}

/// Checked heap capacity retained by one provenance arena, builder, or coupled fork state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProvenanceRetainedFootprint {
    control_storage_bytes: usize,
    shared_node_bytes: usize,
}

impl ProvenanceRetainedFootprint {
    fn try_new(
        control_storage_bytes: usize,
        shared_node_bytes: usize,
    ) -> Result<Self, ProvenanceFootprintError> {
        checked_add_footprint(control_storage_bytes, shared_node_bytes)?;
        Ok(Self {
            control_storage_bytes,
            shared_node_bytes,
        })
    }

    /// Returns segment, pointer-vector, and fork-control storage bytes.
    pub const fn control_storage_bytes(self) -> usize {
        self.control_storage_bytes
    }

    /// Returns Arc header/payload and node-owned dynamic capacities.
    pub const fn shared_node_bytes(self) -> usize {
        self.shared_node_bytes
    }

    /// Returns total retained bytes for this arena's ownership view.
    pub const fn total_bytes(self) -> usize {
        self.control_storage_bytes + self.shared_node_bytes
    }
}

/// Why retained provenance capacity could not be represented.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceFootprintError {
    #[error("provenance retained-footprint accounting overflowed")]
    AccountingOverflow,
}

impl ProvenanceFootprintError {
    fn from_shared_storage(_: SharedStorageFootprintError) -> Self {
        Self::AccountingOverflow
    }
}

/// A bounded provenance dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProvenanceLimit {
    ArenaNodes,
    ArenaEdges,
    OperationParents,
    OperationEvidenceSummaries,
    ArenaDepth,
    ValidationWork,
    RetainedBytes,
}

/// A provenance-owned collection whose growth can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProvenanceAllocation {
    ArenaNodes,
    OperationParents,
    SharedNode,
    ForkSegments,
    ForkSnapshot,
}

/// An editable descendant that shares immutable ancestor segments.
pub struct ProvenanceArenaFork {
    builder: ProvenanceArenaBuilder,
    remap: ProvenanceForkRemap,
}

impl ProvenanceArenaFork {
    /// Returns the cloned builder for inspection.
    pub const fn builder(&self) -> &ProvenanceArenaBuilder {
        &self.builder
    }

    /// Returns the cloned builder for adding branch-local lineage.
    pub const fn builder_mut(&mut self) -> &mut ProvenanceArenaBuilder {
        &mut self.builder
    }

    /// Returns the source-snapshot validator for identity-preserved attachments.
    pub const fn remap(&self) -> &ProvenanceForkRemap {
        &self.remap
    }

    /// Validates one identity-preserved source attachment for this fork.
    pub fn remap_attachment(
        &self,
        provenance: &Provenance,
    ) -> Result<Provenance, ProvenanceRemapError> {
        self.remap.remap(provenance)
    }

    /// Validates and freezes the fork while retaining its prefix remap.
    pub fn finish(
        self,
    ) -> Result<(ProvenanceArena, ProvenanceForkRemap), ProvenanceValidationError> {
        let arena = self.builder.finish()?;
        Ok((arena, self.remap))
    }
}

impl std::fmt::Debug for ProvenanceArenaFork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvenanceArenaFork")
            .field("node_count", &self.builder.node_count())
            .field("edge_count", &self.builder.edge_count())
            .finish_non_exhaustive()
    }
}

/// An identity remap validated against one immutable source snapshot.
pub struct ProvenanceForkRemap {
    source: ArenaNonce,
    target: ArenaNonce,
    source_segments: SharedStorage<Vec<ProvenanceSegmentSnapshot>>,
    source_node_count: usize,
}

impl ProvenanceForkRemap {
    /// Maps one source attachment without exposing its private node address.
    pub fn remap(&self, provenance: &Provenance) -> Result<Provenance, ProvenanceRemapError> {
        let Some(segment) = self.source_segments.as_ref().get(provenance.segment) else {
            return Err(ProvenanceRemapError::ForeignSourceArena);
        };
        if segment.nonce != provenance.arena || segment.ordinal != provenance.segment {
            return Err(ProvenanceRemapError::ForeignSourceArena);
        }
        let expected_topological_index = segment
            .start
            .checked_add(provenance.node.0)
            .ok_or(ProvenanceRemapError::StaleSourceAttachment)?;
        if provenance.topological_index != expected_topological_index
            || provenance.topological_index >= self.source_node_count
            || provenance.node.0 >= segment.node_count
        {
            return Err(ProvenanceRemapError::StaleSourceAttachment);
        }
        Ok(*provenance)
    }

    /// Returns the number of source nodes covered by this snapshot remap.
    pub const fn mapped_node_count(&self) -> usize {
        self.source_node_count
    }

    /// Returns retained bytes for the shared immutable source snapshot.
    pub fn retained_allocation_bytes(&self) -> usize {
        self.source_segments.retained_allocation_bytes()
    }

    pub(crate) fn validate_target_arena(
        &self,
        arena: &ProvenanceArena,
    ) -> Result<(), ProvenanceRemapError> {
        if arena.nonce == self.target {
            Ok(())
        } else {
            Err(ProvenanceRemapError::ForeignTargetArena)
        }
    }
}

impl Clone for ProvenanceForkRemap {
    fn clone(&self) -> Self {
        Self {
            source: self.source,
            target: self.target,
            source_segments: self.source_segments.clone(),
            source_node_count: self.source_node_count,
        }
    }
}

impl PartialEq for ProvenanceForkRemap {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.target == other.target
            && self.source_node_count == other.source_node_count
    }
}

impl Eq for ProvenanceForkRemap {}

impl Hash for ProvenanceForkRemap {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        self.target.hash(state);
        self.source_node_count.hash(state);
    }
}

impl std::fmt::Debug for ProvenanceForkRemap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvenanceForkRemap")
            .field("arenas", &"opaque")
            .field("mapped_node_count", &self.source_node_count)
            .finish()
    }
}

/// Why an immutable provenance arena could not be forked.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceForkError {
    #[error("provenance arena identity space is exhausted")]
    ArenaIdentityExhausted,

    #[error("cannot fork invalid provenance arena: {issue}")]
    InvalidSource { issue: ProvenanceValidationIssue },

    #[error("provenance fork accounting overflowed")]
    AccountingOverflow,

    #[error("provenance fork limit {limit:?} exceeded; the maximum is {maximum}")]
    LimitExceeded {
        limit: ProvenanceLimit,
        maximum: usize,
    },

    #[error("provenance fork allocation failed for {resource:?}")]
    AllocationFailed { resource: ProvenanceAllocation },
}

impl ProvenanceForkError {
    fn from_validation(error: ProvenanceValidationError, limits: ProvenanceLimits) -> Self {
        match error.issue() {
            ProvenanceValidationIssue::NodeLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::ArenaNodes,
                maximum: limits.max_nodes,
            },
            ProvenanceValidationIssue::EdgeLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::ArenaEdges,
                maximum: limits.max_edges,
            },
            ProvenanceValidationIssue::ParentLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::OperationParents,
                maximum: limits.max_parents_per_operation,
            },
            ProvenanceValidationIssue::EvidenceLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::OperationEvidenceSummaries,
                maximum: limits.max_evidence_summaries,
            },
            ProvenanceValidationIssue::DepthLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::ArenaDepth,
                maximum: limits.max_depth,
            },
            ProvenanceValidationIssue::WorkLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::ValidationWork,
                maximum: limits.max_validation_work,
            },
            ProvenanceValidationIssue::RetainedBytesLimitExceeded => Self::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: limits.max_retained_bytes,
            },
            ProvenanceValidationIssue::EdgeCountOverflow
            | ProvenanceValidationIssue::RetainedFootprintOverflow => Self::AccountingOverflow,
            issue => Self::InvalidSource { issue },
        }
    }
}

/// Why an attachment could not be translated through a fork remap.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceRemapError {
    #[error("provenance attachment does not belong to the remap source arena")]
    ForeignSourceArena,

    #[error("provenance attachment is outside the remap source snapshot")]
    StaleSourceAttachment,

    #[error("provenance arena is not the target of this fork remap")]
    ForeignTargetArena,
}

/// An immutable scene-owned provenance DAG.
pub struct ProvenanceArena {
    nonce: ArenaNonce,
    segments: Vec<SharedStorage<ProvenanceSegment>>,
    node_count: usize,
    edge_count: usize,
    retained_node_bytes: usize,
    limits: ProvenanceLimits,
}

impl ProvenanceArena {
    /// Returns the number of topologically ordered nodes.
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Returns the number of parent edges.
    pub const fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Returns the limits under which this arena was constructed.
    pub const fn limits(&self) -> ProvenanceLimits {
        self.limits
    }

    /// Recomputes retained capacity from actual Vec/String capacities.
    pub fn retained_footprint(
        &self,
    ) -> Result<ProvenanceRetainedFootprint, ProvenanceFootprintError> {
        checked_segments_retained_footprint(
            &self.segments,
            self.segments.capacity(),
            self.retained_node_bytes,
        )
    }

    /// Fallibly forks this DAG while sharing immutable ancestor segments.
    ///
    /// Existing attachments remain valid by identity in the descendant. A
    /// fresh opaque nonce is assigned only to the descendant's branch-local
    /// segment, so sibling-local attachments cannot cross.
    pub fn try_fork(&self) -> Result<ProvenanceArenaFork, ProvenanceForkError> {
        let validated = validate_provenance_arena(self)
            .map_err(|error| ProvenanceForkError::from_validation(error, self.limits))?;
        let shared_node_bytes = validated.stats().retained_footprint().shared_node_bytes();
        let segment_capacity = CapacityPlan::for_type::<SharedStorage<ProvenanceSegment>>(
            0,
            0,
            self.segments
                .len()
                .checked_add(1)
                .ok_or(ProvenanceForkError::AccountingOverflow)?,
        )
        .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let arena_envelope = checked_segments_retained_footprint(
            &self.segments,
            segment_capacity.envelope_capacity(),
            shared_node_bytes,
        )
        .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let snapshot_capacity =
            CapacityPlan::for_type::<ProvenanceSegmentSnapshot>(0, 0, self.segments.len())
                .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let snapshot_envelope_bytes = checked_capacity_bytes::<ProvenanceSegmentSnapshot>(
            snapshot_capacity.envelope_capacity(),
        )
        .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let snapshot_layout =
            SharedStorage::<Vec<ProvenanceSegmentSnapshot>>::checked_layout_footprint()
                .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let snapshot_envelope_bytes = snapshot_layout
            .checked_add(snapshot_envelope_bytes)
            .ok_or(ProvenanceForkError::AccountingOverflow)?;
        let fork_retained_bytes = arena_envelope
            .total_bytes()
            .checked_add(snapshot_envelope_bytes)
            .ok_or(ProvenanceForkError::AccountingOverflow)?;
        if fork_retained_bytes > self.limits.max_retained_bytes {
            return Err(ProvenanceForkError::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: self.limits.max_retained_bytes,
            });
        }

        let segments = try_clone_segments_for_fork(&self.segments)?;
        let mut snapshots = Vec::new();
        try_reserve_for_fork(
            &mut snapshots,
            snapshot_capacity.allocator_additional(),
            ProvenanceAllocation::ForkSnapshot,
        )?;
        snapshot_capacity
            .finish(0, snapshots.capacity())
            .map_err(|_| ProvenanceForkError::AllocationFailed {
                resource: ProvenanceAllocation::ForkSnapshot,
            })?;
        snapshots.extend(self.segments.iter().map(|segment| {
            let segment = segment.as_ref();
            ProvenanceSegmentSnapshot {
                nonce: segment.nonce,
                ordinal: segment.ordinal,
                start: segment.start,
                node_count: segment.nodes.len(),
            }
        }));
        let snapshot_bytes =
            checked_capacity_bytes::<ProvenanceSegmentSnapshot>(snapshots.capacity())
                .map_err(|_| ProvenanceForkError::AccountingOverflow)?;
        let target = next_arena_nonce()
            .map_err(|ArenaNonceError::Exhausted| ProvenanceForkError::ArenaIdentityExhausted)?;
        let source_segments = SharedStorage::try_new_prepared(snapshots, snapshot_bytes).map_err(
            |error| match error {
                SharedStorageError::AllocationFailed => ProvenanceForkError::AllocationFailed {
                    resource: ProvenanceAllocation::ForkSnapshot,
                },
                SharedStorageError::AccountingOverflow => ProvenanceForkError::AccountingOverflow,
            },
        )?;
        let fork_control_storage_bytes = source_segments.retained_allocation_bytes();
        Ok(ProvenanceArenaFork {
            builder: ProvenanceArenaBuilder {
                nonce: target,
                segments,
                local_nodes: Vec::new(),
                local_retained_node_bytes: 0,
                node_count: self.node_count,
                edge_count: self.edge_count,
                retained_node_bytes: shared_node_bytes,
                fork_control_storage_bytes,
                limits: self.limits,
            },
            remap: ProvenanceForkRemap {
                source: self.nonce,
                target,
                source_segments,
                source_node_count: self.node_count,
            },
        })
    }

    /// Returns whether an opaque attachment belongs to this arena and is live.
    pub fn contains(&self, provenance: &Provenance) -> bool {
        self.resolve(provenance).is_ok()
    }

    /// Resolves an opaque attachment without exposing its arena address.
    pub fn view(
        &self,
        provenance: &Provenance,
    ) -> Result<ProvenanceView<'_>, ProvenanceAccessError> {
        let node = self.resolve(provenance)?;
        Ok(ProvenanceView { arena: self, node })
    }

    /// Iterates nodes in deterministic topological order without numeric IDs.
    pub fn iter(&self) -> ProvenanceIter<'_> {
        ProvenanceIter {
            arena: self,
            front_segment: 0,
            front_node: 0,
            back_segment: self.segments.len(),
            back_node: 0,
            remaining: self.node_count,
        }
    }

    /// Revalidates the immutable DAG and returns a checked read facade.
    pub fn validate(&self) -> Result<ValidatedProvenanceArena<'_>, ProvenanceValidationError> {
        validate_provenance_arena(self)
    }

    /// Revalidates this arena under an equal or stricter configured envelope.
    pub fn validate_with_limits(
        &self,
        limits: ProvenanceLimits,
    ) -> Result<ValidatedProvenanceArena<'_>, ProvenanceValidationError> {
        validate_provenance_arena_with_limits(self, limits)
    }

    fn resolve(&self, provenance: &Provenance) -> Result<&ProvenanceNode, ProvenanceAccessError> {
        let segment = self
            .segments
            .get(provenance.segment)
            .ok_or(ProvenanceAccessError::ArenaMismatch)?
            .as_ref();
        if segment.ordinal != provenance.segment || segment.nonce != provenance.arena {
            return Err(ProvenanceAccessError::ArenaMismatch);
        }
        let expected_topological_index = segment
            .start
            .checked_add(provenance.node.0)
            .ok_or(ProvenanceAccessError::AttachmentOutOfRange)?;
        if provenance.topological_index != expected_topological_index {
            return Err(ProvenanceAccessError::AttachmentOutOfRange);
        }
        let node = segment
            .nodes
            .get(provenance.node.0)
            .ok_or(ProvenanceAccessError::AttachmentOutOfRange)?;
        Ok(node.as_ref())
    }
}

impl std::fmt::Debug for ProvenanceArena {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvenanceArena")
            .field("node_count", &self.node_count)
            .field("edge_count", &self.edge_count)
            .finish()
    }
}

/// Why an opaque provenance attachment could not be resolved.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceAccessError {
    #[error("provenance attachment belongs to a different arena")]
    ArenaMismatch,

    #[error("provenance attachment does not address an existing node")]
    AttachmentOutOfRange,
}

/// A validated read facade over one immutable provenance arena.
#[derive(Clone, Copy)]
pub struct ValidatedProvenanceArena<'a> {
    arena: &'a ProvenanceArena,
    stats: ProvenanceValidationStats,
}

impl<'a> ValidatedProvenanceArena<'a> {
    /// Returns the number of topologically ordered nodes.
    pub fn node_count(self) -> usize {
        self.arena.node_count()
    }

    /// Returns the number of parent edges.
    pub fn edge_count(self) -> usize {
        self.arena.edge_count()
    }

    /// Resolves an attachment without exposing a numeric node address.
    pub fn view(
        self,
        provenance: &Provenance,
    ) -> Result<ProvenanceView<'a>, ProvenanceAccessError> {
        self.arena.view(provenance)
    }

    /// Iterates nodes in deterministic topological order.
    pub fn iter(self) -> ProvenanceIter<'a> {
        self.arena.iter()
    }

    /// Returns deterministic node, edge, depth, work, and footprint evidence.
    pub const fn stats(self) -> ProvenanceValidationStats {
        self.stats
    }
}

impl std::fmt::Debug for ValidatedProvenanceArena<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedProvenanceArena")
            .field("node_count", &self.arena.node_count())
            .field("edge_count", &self.arena.edge_count())
            .field("stats", &self.stats)
            .finish()
    }
}

/// Validates node bounds, strict parent order, duplicate freedom, and topology.
pub fn validate_provenance_arena(
    arena: &ProvenanceArena,
) -> Result<ValidatedProvenanceArena<'_>, ProvenanceValidationError> {
    validate_provenance_arena_with_limits(arena, arena.limits)
}

/// Validates one arena under an equal or stricter caller-selected envelope.
pub fn validate_provenance_arena_with_limits(
    arena: &ProvenanceArena,
    limits: ProvenanceLimits,
) -> Result<ValidatedProvenanceArena<'_>, ProvenanceValidationError> {
    match validate_provenance_arena_with_limits_controlled(arena, limits, &mut || {
        Ok::<(), std::convert::Infallible>(())
    }) {
        Ok(validated) => Ok(validated),
        Err(ControlledProvenanceValidationError::Validation(error)) => Err(error),
        Err(ControlledProvenanceValidationError::Control(error)) => match error {},
    }
}

#[derive(Debug)]
pub(crate) enum ControlledProvenanceValidationError<E> {
    Validation(ProvenanceValidationError),
    Control(E),
}

impl<E> From<ProvenanceValidationError> for ControlledProvenanceValidationError<E> {
    fn from(error: ProvenanceValidationError) -> Self {
        Self::Validation(error)
    }
}

pub(crate) fn validate_provenance_arena_controlled<'arena, E>(
    arena: &'arena ProvenanceArena,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<ValidatedProvenanceArena<'arena>, ControlledProvenanceValidationError<E>> {
    validate_provenance_arena_with_limits_controlled(arena, arena.limits, charge_work)
}

fn validate_provenance_arena_with_limits_controlled<'arena, E>(
    arena: &'arena ProvenanceArena,
    limits: ProvenanceLimits,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<ValidatedProvenanceArena<'arena>, ControlledProvenanceValidationError<E>> {
    let mut control_storage_bytes =
        checked_capacity_bytes::<SharedStorage<ProvenanceSegment>>(arena.segments.capacity())
            .map_err(|_| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    ProvenanceValidationStats::empty(),
                )
            })?;
    for segment in &arena.segments {
        control_storage_bytes = control_storage_bytes
            .checked_add(segment.retained_allocation_bytes())
            .ok_or_else(|| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    ProvenanceValidationStats::empty(),
                )
            })?;
    }
    let mut stats = ProvenanceValidationStats::with_control_storage_bytes(control_storage_bytes);
    if control_storage_bytes > limits.max_retained_bytes {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::RetainedBytesLimitExceeded,
            stats,
        ));
    }
    if arena.node_count > limits.max_nodes {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::NodeLimitExceeded,
            stats,
        ));
    }

    let mut observed_edges = 0usize;
    let mut observed_node_bytes = 0usize;
    for (segment_index, stored_segment) in arena.segments.iter().enumerate() {
        let segment = stored_segment.as_ref();
        if segment.ordinal != segment_index
            || segment.start != stats.node_count
            || segment.nodes.is_empty()
        {
            return Err(controlled_validation_error(
                ProvenanceValidationIssue::SegmentLayoutInvalid,
                stats,
            ));
        }
        let segment_start_bytes = observed_node_bytes;

        for stored in &segment.nodes {
            if stats.node_count == limits.max_nodes {
                return Err(controlled_validation_error(
                    ProvenanceValidationIssue::NodeLimitExceeded,
                    stats,
                ));
            }
            charge_validation_work(&mut stats, limits.max_validation_work, charge_work)?;
            let child_index = stats.node_count;
            stats.node_count += 1;

            let node_bytes = checked_stored_node_retained_bytes(stored).map_err(|_| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    stats,
                )
            })?;
            observed_node_bytes = observed_node_bytes.checked_add(node_bytes).ok_or_else(|| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    stats,
                )
            })?;
            stats.retained_footprint =
                ProvenanceRetainedFootprint::try_new(control_storage_bytes, observed_node_bytes)
                    .map_err(|_| {
                        ProvenanceValidationError::new(
                            ProvenanceValidationIssue::RetainedFootprintOverflow,
                            stats,
                        )
                    })?;
            if stats.retained_footprint.total_bytes() > limits.max_retained_bytes {
                return Err(controlled_validation_error(
                    ProvenanceValidationIssue::RetainedBytesLimitExceeded,
                    stats,
                ));
            }

            let node: &ProvenanceNode = stored.borrow();
            let expected_depth = match &node.payload {
                ProvenanceNodePayload::Source(_) => 1,
                ProvenanceNodePayload::Operation { identity, parents } => {
                    if identity.evidence.len() > limits.max_evidence_summaries {
                        return Err(controlled_validation_error(
                            ProvenanceValidationIssue::EvidenceLimitExceeded,
                            stats,
                        ));
                    }
                    if parents.len() > limits.max_parents_per_operation {
                        return Err(controlled_validation_error(
                            ProvenanceValidationIssue::ParentLimitExceeded,
                            stats,
                        ));
                    }

                    let mut previous = None;
                    let mut maximum_parent_depth = 0usize;
                    for parent in parents {
                        charge_validation_work(
                            &mut stats,
                            limits.max_validation_work,
                            charge_work,
                        )?;
                        observed_edges = observed_edges.checked_add(1).ok_or_else(|| {
                            ProvenanceValidationError::new(
                                ProvenanceValidationIssue::EdgeCountOverflow,
                                stats,
                            )
                        })?;
                        stats.edge_count = observed_edges;
                        if observed_edges > limits.max_edges {
                            return Err(controlled_validation_error(
                                ProvenanceValidationIssue::EdgeLimitExceeded,
                                stats,
                            ));
                        }
                        let parent_node = arena.resolve(parent).map_err(|_| {
                            controlled_validation_error(
                                ProvenanceValidationIssue::ParentOutOfRange,
                                stats,
                            )
                        })?;
                        if parent.topological_index >= child_index {
                            return Err(controlled_validation_error(
                                ProvenanceValidationIssue::ParentNotBeforeChild,
                                stats,
                            ));
                        }
                        if previous == Some(*parent) {
                            return Err(controlled_validation_error(
                                ProvenanceValidationIssue::DuplicateParent,
                                stats,
                            ));
                        }
                        if previous.is_some_and(|previous: Provenance| {
                            previous.topological_index > parent.topological_index
                        }) {
                            return Err(controlled_validation_error(
                                ProvenanceValidationIssue::ParentOrderNotCanonical,
                                stats,
                            ));
                        }
                        maximum_parent_depth = maximum_parent_depth.max(parent_node.depth);
                        previous = Some(*parent);
                    }
                    maximum_parent_depth.checked_add(1).ok_or_else(|| {
                        ProvenanceValidationError::new(
                            ProvenanceValidationIssue::DepthLimitExceeded,
                            stats,
                        )
                    })?
                }
            };
            if expected_depth > limits.max_depth {
                return Err(controlled_validation_error(
                    ProvenanceValidationIssue::DepthLimitExceeded,
                    stats,
                ));
            }
            if node.depth != expected_depth {
                return Err(controlled_validation_error(
                    ProvenanceValidationIssue::StoredDepthMismatch,
                    stats,
                ));
            }
            stats.maximum_depth = stats.maximum_depth.max(expected_depth);
        }

        let observed_segment_bytes = observed_node_bytes
            .checked_sub(segment_start_bytes)
            .ok_or_else(|| {
                ProvenanceValidationError::new(
                    ProvenanceValidationIssue::RetainedFootprintOverflow,
                    stats,
                )
            })?;
        if observed_segment_bytes != segment.retained_node_bytes {
            return Err(controlled_validation_error(
                ProvenanceValidationIssue::RetainedFootprintMismatch,
                stats,
            ));
        }
    }

    if stats.node_count != arena.node_count {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::NodeCountMismatch,
            stats,
        ));
    }

    if observed_edges != arena.edge_count {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::EdgeCountMismatch,
            stats,
        ));
    }
    if observed_node_bytes != arena.retained_node_bytes {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::RetainedFootprintMismatch,
            stats,
        ));
    }
    Ok(ValidatedProvenanceArena { arena, stats })
}

fn charge_validation_work<E>(
    stats: &mut ProvenanceValidationStats,
    maximum: usize,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), ControlledProvenanceValidationError<E>> {
    if stats.work_units >= maximum {
        return Err(controlled_validation_error(
            ProvenanceValidationIssue::WorkLimitExceeded,
            *stats,
        ));
    }
    charge_work().map_err(ControlledProvenanceValidationError::Control)?;
    stats.work_units = stats.work_units.checked_add(1).ok_or_else(|| {
        ControlledProvenanceValidationError::Validation(ProvenanceValidationError::new(
            ProvenanceValidationIssue::WorkLimitExceeded,
            *stats,
        ))
    })?;
    Ok(())
}

fn controlled_validation_error<E>(
    issue: ProvenanceValidationIssue,
    stats: ProvenanceValidationStats,
) -> ControlledProvenanceValidationError<E> {
    ControlledProvenanceValidationError::Validation(ProvenanceValidationError::new(issue, stats))
}

/// Deterministic evidence from a bounded linear provenance validation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProvenanceValidationStats {
    node_count: usize,
    edge_count: usize,
    maximum_depth: usize,
    work_units: usize,
    retained_footprint: ProvenanceRetainedFootprint,
}

impl ProvenanceValidationStats {
    const fn empty() -> Self {
        Self::with_control_storage_bytes(0)
    }

    const fn with_control_storage_bytes(control_storage_bytes: usize) -> Self {
        Self {
            node_count: 0,
            edge_count: 0,
            maximum_depth: 0,
            work_units: 0,
            retained_footprint: ProvenanceRetainedFootprint {
                control_storage_bytes,
                shared_node_bytes: 0,
            },
        }
    }

    /// Returns nodes examined before success or failure.
    pub const fn node_count(self) -> usize {
        self.node_count
    }

    /// Returns parent edges examined before success or failure.
    pub const fn edge_count(self) -> usize {
        self.edge_count
    }

    /// Returns the maximum canonical depth observed so far.
    pub const fn maximum_depth(self) -> usize {
        self.maximum_depth
    }

    /// Returns one charged work unit per examined node and edge.
    pub const fn work_units(self) -> usize {
        self.work_units
    }

    /// Returns checked retained-capacity evidence accumulated so far.
    pub const fn retained_footprint(self) -> ProvenanceRetainedFootprint {
        self.retained_footprint
    }
}

/// A stable, allocation-free provenance validation category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProvenanceValidationIssue {
    AllocationFailed,
    NodeLimitExceeded,
    NodeCountMismatch,
    SegmentLayoutInvalid,
    EdgeLimitExceeded,
    ParentLimitExceeded,
    EvidenceLimitExceeded,
    DepthLimitExceeded,
    WorkLimitExceeded,
    EdgeCountOverflow,
    EdgeCountMismatch,
    ParentOutOfRange,
    ParentNotBeforeChild,
    DuplicateParent,
    ParentOrderNotCanonical,
    StoredDepthMismatch,
    RetainedBytesLimitExceeded,
    RetainedFootprintOverflow,
    RetainedFootprintMismatch,
}

impl std::fmt::Display for ProvenanceValidationIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AllocationFailed => "provenance immutable storage allocation failed",
            Self::NodeLimitExceeded => "provenance node limit exceeded",
            Self::NodeCountMismatch => "provenance node count does not match stored segments",
            Self::SegmentLayoutInvalid => "provenance segments are not canonically laid out",
            Self::EdgeLimitExceeded => "provenance edge limit exceeded",
            Self::ParentLimitExceeded => "provenance operation parent limit exceeded",
            Self::EvidenceLimitExceeded => "provenance operation evidence limit exceeded",
            Self::DepthLimitExceeded => "provenance depth limit exceeded",
            Self::WorkLimitExceeded => "provenance validation work limit exceeded",
            Self::EdgeCountOverflow => "provenance edge count overflowed",
            Self::EdgeCountMismatch => "provenance edge count does not match stored edges",
            Self::ParentOutOfRange => "provenance parent is out of range",
            Self::ParentNotBeforeChild => "provenance parent does not precede its child",
            Self::DuplicateParent => "provenance operation contains a duplicate parent",
            Self::ParentOrderNotCanonical => "provenance parents are not canonically ordered",
            Self::StoredDepthMismatch => "provenance stored depth is not canonical",
            Self::RetainedBytesLimitExceeded => "provenance retained-byte limit exceeded",
            Self::RetainedFootprintOverflow => {
                "provenance retained-footprint accounting overflowed"
            }
            Self::RetainedFootprintMismatch => {
                "provenance retained-footprint evidence does not match stored nodes"
            }
        })
    }
}

/// A failed immutable provenance validation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("invalid provenance arena: {issue}")]
pub struct ProvenanceValidationError {
    issue: ProvenanceValidationIssue,
    stats: ProvenanceValidationStats,
}

impl ProvenanceValidationError {
    const fn new(issue: ProvenanceValidationIssue, stats: ProvenanceValidationStats) -> Self {
        Self { issue, stats }
    }

    /// Returns the stable validation category.
    pub const fn issue(self) -> ProvenanceValidationIssue {
        self.issue
    }

    /// Returns deterministic partial validation evidence.
    pub const fn stats(self) -> ProvenanceValidationStats {
        self.stats
    }
}

/// A node resolved from an opaque provenance attachment.
#[derive(Clone, Copy)]
pub struct ProvenanceView<'a> {
    arena: &'a ProvenanceArena,
    node: &'a ProvenanceNode,
}

impl<'a> ProvenanceView<'a> {
    /// Returns canonical lineage depth, where leaves have depth one.
    pub const fn depth(self) -> usize {
        self.node.depth
    }

    /// Returns the typed node payload without exposing its private address.
    pub fn node(self) -> ProvenanceNodeView<'a> {
        match &self.node.payload {
            ProvenanceNodePayload::Source(identity) => ProvenanceNodeView::Source(identity),
            ProvenanceNodePayload::Operation { identity, parents } => {
                ProvenanceNodeView::Operation(OperationProvenanceView {
                    arena: self.arena,
                    identity,
                    parents,
                })
            }
        }
    }
}

impl std::fmt::Debug for ProvenanceView<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (*self).node() {
            ProvenanceNodeView::Source(identity) => formatter
                .debug_tuple("ProvenanceView::Source")
                .field(identity)
                .finish(),
            ProvenanceNodeView::Operation(operation) => formatter
                .debug_struct("ProvenanceView::Operation")
                .field("identity", operation.identity())
                .field("parent_count", &operation.parent_count())
                .finish(),
        }
    }
}

/// The public payload of one provenance node.
#[derive(Debug, Clone, Copy)]
pub enum ProvenanceNodeView<'a> {
    Source(&'a SourceIdentity),
    Operation(OperationProvenanceView<'a>),
}

/// A read-only operation node whose parent iterator yields views, not IDs.
#[derive(Clone, Copy)]
pub struct OperationProvenanceView<'a> {
    arena: &'a ProvenanceArena,
    identity: &'a OperationIdentity,
    parents: &'a [Provenance],
}

impl<'a> OperationProvenanceView<'a> {
    /// Returns the operation identity.
    pub const fn identity(self) -> &'a OperationIdentity {
        self.identity
    }

    /// Returns the number of direct parents.
    pub const fn parent_count(self) -> usize {
        self.parents.len()
    }

    /// Iterates direct parents without exposing numeric arena addresses.
    pub fn parents(self) -> ProvenanceParentIter<'a> {
        ProvenanceParentIter {
            arena: self.arena,
            parents: self.parents.iter(),
        }
    }
}

impl std::fmt::Debug for OperationProvenanceView<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperationProvenanceView")
            .field("identity", &self.identity)
            .field("parent_count", &self.parents.len())
            .finish()
    }
}

/// A topological iterator that never exposes private node addresses.
pub struct ProvenanceIter<'a> {
    arena: &'a ProvenanceArena,
    front_segment: usize,
    front_node: usize,
    back_segment: usize,
    back_node: usize,
    remaining: usize,
}

impl<'a> Iterator for ProvenanceIter<'a> {
    type Item = ProvenanceView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining != 0 {
            let segment = self.arena.segments.get(self.front_segment)?.as_ref();
            if let Some(node) = segment.nodes.get(self.front_node) {
                self.front_node += 1;
                self.remaining -= 1;
                return Some(ProvenanceView {
                    arena: self.arena,
                    node: node.borrow(),
                });
            }
            self.front_segment += 1;
            self.front_node = 0;
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for ProvenanceIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        while self.remaining != 0 {
            if self.back_node == 0 {
                self.back_segment = self.back_segment.checked_sub(1)?;
                self.back_node = self.arena.segments[self.back_segment].as_ref().nodes.len();
                continue;
            }
            self.back_node -= 1;
            let node = &self.arena.segments[self.back_segment].as_ref().nodes[self.back_node];
            self.remaining -= 1;
            return Some(ProvenanceView {
                arena: self.arena,
                node: node.borrow(),
            });
        }
        None
    }
}

impl ExactSizeIterator for ProvenanceIter<'_> {
    fn len(&self) -> usize {
        self.remaining
    }
}

impl std::iter::FusedIterator for ProvenanceIter<'_> {}

/// An iterator over operation parents that yields only validated node views.
pub struct ProvenanceParentIter<'a> {
    arena: &'a ProvenanceArena,
    parents: std::slice::Iter<'a, Provenance>,
}

impl<'a> Iterator for ProvenanceParentIter<'a> {
    type Item = ProvenanceView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        for parent in self.parents.by_ref() {
            if let Ok(node) = self.arena.resolve(parent) {
                return Some(ProvenanceView {
                    arena: self.arena,
                    node,
                });
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.parents.size_hint()
    }
}

impl DoubleEndedIterator for ProvenanceParentIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        for parent in self.parents.by_ref().rev() {
            if let Ok(node) = self.arena.resolve(parent) {
                return Some(ProvenanceView {
                    arena: self.arena,
                    node,
                });
            }
        }
        None
    }
}

impl ExactSizeIterator for ProvenanceParentIter<'_> {
    fn len(&self) -> usize {
        self.parents.len()
    }
}

impl std::iter::FusedIterator for ProvenanceParentIter<'_> {}

/// Creates a valid arena-owned attachment for crate unit tests.
#[cfg(test)]
pub(crate) fn test_provenance_fixture() -> (ProvenanceArena, Provenance) {
    let mut builder =
        ProvenanceArenaBuilder::try_new().expect("test arena nonce must be available");
    let identity = SourceIdentity::new(
        ResourceIdentity::new("tests/fixture.asset").expect("test resource identity must be valid"),
        FormatId::new("dev.baozi.test").expect("test format id must be valid"),
        SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
    );
    let provenance = builder
        .add_source(identity)
        .expect("test source provenance must fit");
    let arena = builder.finish().expect("test arena must be valid");
    (arena, provenance)
}

/// Creates two valid ordered attachments in one arena for crate unit tests.
#[cfg(test)]
pub(crate) fn test_provenance_pair_fixture() -> (ProvenanceArena, [Provenance; 2]) {
    let mut builder =
        ProvenanceArenaBuilder::try_new().expect("test arena nonce must be available");
    let first = builder
        .add_source(SourceIdentity::new(
            ResourceIdentity::new("tests/first.asset")
                .expect("test resource identity must be valid"),
            FormatId::new("dev.baozi.test").expect("test format id must be valid"),
            SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
        ))
        .expect("first test source provenance must fit");
    let second = builder
        .add_source(SourceIdentity::new(
            ResourceIdentity::new("tests/second.asset")
                .expect("test resource identity must be valid"),
            FormatId::new("dev.baozi.test").expect("test format id must be valid"),
            SourceObjectLocator::indexed(SourceObjectKind::Document, 1),
        ))
        .expect("second test source provenance must fit");
    let arena = builder.finish().expect("test arena must be valid");
    (arena, [first, second])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::ExtensionSchemaId;
    use crate::shared_storage::fail_next_allocation_for_test;
    use std::collections::hash_map::DefaultHasher;

    fn source_identity(resource: &str, display: &str) -> SourceIdentity {
        SourceIdentity::new(
            ResourceIdentity::new(resource).unwrap(),
            FormatId::new("org.baozi.obj").unwrap(),
            SourceObjectLocator::indexed(SourceObjectKind::Mesh, 7),
        )
        .with_display_name(SourceDisplayName::new(display).unwrap())
    }

    fn operation(kind: OperationKind) -> OperationIdentity {
        OperationIdentity::without_evidence(kind, OptionsDigest::sha256([0; DIGEST_LEN]))
    }

    fn source_node(identity: SourceIdentity) -> ProvenanceNode {
        ProvenanceNode {
            depth: 1,
            payload: ProvenanceNodePayload::Source(identity),
        }
    }

    fn operation_node(
        identity: OperationIdentity,
        parents: Vec<ProvenanceNodeId>,
        depth: usize,
    ) -> ProvenanceNode {
        let mut mapped_parents = Vec::new();
        mapped_parents
            .try_reserve_exact(parents.capacity())
            .unwrap();
        mapped_parents.extend(parents.into_iter().map(|node| Provenance {
            arena: ArenaNonce(NonZeroU64::new(u64::MAX).unwrap()),
            segment: 0,
            topological_index: node.0,
            node,
        }));
        ProvenanceNode {
            depth,
            payload: ProvenanceNodePayload::Operation {
                identity,
                parents: mapped_parents,
            },
        }
    }

    fn arena_from_nodes(mut nodes: Vec<ProvenanceNode>, edge_count: usize) -> ProvenanceArena {
        let nonce = next_arena_nonce().unwrap();
        for node in &mut nodes {
            let ProvenanceNodePayload::Operation { parents, .. } = &mut node.payload else {
                continue;
            };
            for parent in parents {
                parent.arena = nonce;
            }
        }

        let node_count = nodes.len();
        let mut stored = Vec::new();
        stored.try_reserve_exact(nodes.len()).unwrap();
        let mut retained_node_bytes = 0usize;
        for node in nodes {
            let dynamic_bytes = checked_node_dynamic_bytes(&node).unwrap();
            let node = SharedStorage::try_new_prepared(node, dynamic_bytes).unwrap();
            retained_node_bytes = retained_node_bytes
                .checked_add(node.retained_allocation_bytes())
                .unwrap();
            stored.push(node);
        }
        let mut segments = Vec::new();
        if !stored.is_empty() {
            let nested_bytes =
                checked_capacity_bytes::<SharedStorage<ProvenanceNode>>(stored.capacity()).unwrap();
            let segment = SharedStorage::try_new_prepared(
                ProvenanceSegment {
                    nonce,
                    ordinal: 0,
                    start: 0,
                    nodes: stored,
                    retained_node_bytes,
                },
                nested_bytes,
            )
            .unwrap();
            segments.try_reserve_exact(1).unwrap();
            segments.push(segment);
        }
        ProvenanceArena {
            nonce,
            segments,
            node_count,
            edge_count,
            retained_node_bytes,
            limits: ProvenanceLimits::hard(),
        }
    }

    fn limits_with_depth_and_work(max_depth: usize, max_work: usize) -> ProvenanceLimits {
        ProvenanceLimits::try_new(
            64,
            128,
            16,
            MAX_OPERATION_EVIDENCE_SUMMARIES,
            max_depth,
            max_work,
            1024 * 1024,
        )
        .unwrap()
    }

    #[test]
    fn namespaced_source_locators_require_and_preserve_schema_ids() {
        assert_eq!(
            NamespacedSourceObjectLocator::try_new(
                None,
                SourceLocatorValue::new("object/ship").unwrap(),
            ),
            Err(NamespacedSourceObjectLocatorError::SchemaRequired)
        );

        let schema_id = ExtensionSchemaId::new("obj:statement", 2)
            .expect("test source-locator schema should be canonical");
        let locator = NamespacedSourceObjectLocator::try_new(
            Some(schema_id.clone()),
            SourceLocatorValue::new("object/ship").unwrap(),
        )
        .expect("schema-bound source locator should be accepted");
        assert_eq!(locator.kind(), schema_id.name());
        assert_eq!(locator.schema_id(), &schema_id);
        assert_eq!(locator.value().as_str(), "object/ship");

        let locator = SourceObjectLocator::Namespaced(locator);
        assert_eq!(locator.schema_id(), Some(&schema_id));
        assert_eq!(
            SourceObjectLocator::indexed(SourceObjectKind::Mesh, 0).schema_id(),
            None
        );
    }

    #[test]
    fn display_and_content_digest_do_not_change_source_identity_equality() {
        let first = source_identity("models/ship.obj", "Ship")
            .with_content_digest(ContentDigest::sha256([1; DIGEST_LEN]));
        let second = source_identity("models/ship.obj", "Renamed ship")
            .with_content_digest(ContentDigest::sha256([2; DIGEST_LEN]));

        assert_eq!(first, second);
        assert_ne!(first.display_name(), second.display_name());
        assert_ne!(first.content_digest(), second.content_digest());

        let mut first_hash = DefaultHasher::new();
        first.hash(&mut first_hash);
        let mut second_hash = DefaultHasher::new();
        second.hash(&mut second_hash);
        assert_eq!(first_hash.finish(), second_hash.finish());
    }

    #[test]
    fn heap_backed_identity_copies_are_explicit_and_fallible() {
        let source = source_identity("models/ship.obj", "Ship");
        let copied = source.try_clone().unwrap();
        assert_eq!(source, copied);
        assert_ne!(
            source.resource().as_str().as_ptr(),
            copied.resource().as_str().as_ptr(),
            "explicit source copies must not masquerade as shared storage"
        );

        let identity = OperationIdentity::new(
            OperationKind::Normalize,
            OptionsDigest::sha256([0; DIGEST_LEN]),
            [OperationEvidenceSummary::new(
                NamespacedName::new("obj:normalized").unwrap(),
                1,
                None,
            )
            .unwrap()],
        )
        .unwrap();
        let copied = identity.try_clone().unwrap();
        assert_eq!(identity, copied);
        assert_ne!(
            identity.evidence.as_ptr(),
            copied.evidence.as_ptr(),
            "operation evidence deep copies must be opt-in"
        );
    }

    #[test]
    fn resource_identity_rejects_control_and_host_absolute_forms_without_echoing_them() {
        let cases = [
            "models/ship\n.obj",
            "/home/user/private.obj",
            "C:\\Users\\user\\private.obj",
            "\\\\server\\share\\private.obj",
            "https://user:secret@example.test/private.obj?token=secret",
        ];

        for value in cases {
            let error = ResourceIdentity::new(value).unwrap_err();
            let message = error.to_string();
            assert!(!message.contains(value));
        }
    }

    #[test]
    fn operation_rejects_a_parent_from_another_arena() {
        let mut first = ProvenanceArenaBuilder::try_new().unwrap();
        let foreign = first
            .add_source(source_identity("models/a.obj", "A"))
            .unwrap();
        let mut second = ProvenanceArenaBuilder::try_new().unwrap();

        assert_eq!(
            second
                .add_operation(operation(OperationKind::Merge), [foreign])
                .unwrap_err(),
            ProvenanceBuildError::ArenaMismatch
        );
    }

    #[test]
    fn operation_rejects_a_forward_or_uncreated_parent() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let forward = Provenance {
            arena: builder.nonce,
            segment: 0,
            node: ProvenanceNodeId(1),
            topological_index: 1,
        };

        assert_eq!(
            builder
                .add_operation(operation(OperationKind::Merge), [forward])
                .unwrap_err(),
            ProvenanceBuildError::ParentOutOfRange
        );
        assert_eq!(builder.node_count(), 0);
    }

    #[test]
    fn operation_rejects_duplicate_parent_edges() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let source = builder
            .add_source(source_identity("models/a.obj", "A"))
            .unwrap();

        assert_eq!(
            builder
                .add_operation(operation(OperationKind::Merge), [source, source])
                .unwrap_err(),
            ProvenanceBuildError::DuplicateParent
        );
        assert_eq!(builder.node_count(), 1);
    }

    #[test]
    fn builder_produces_stable_topology_and_canonical_parent_order() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let first = builder
            .add_source(source_identity("models/a.obj", "A"))
            .unwrap();
        let second = builder
            .add_source(source_identity("models/b.obj", "B"))
            .unwrap();
        let merged = builder
            .add_operation(operation(OperationKind::Merge), [second, first])
            .unwrap();
        let arena = builder.finish().unwrap();
        let stats = arena.validate().unwrap().stats();
        assert_eq!(stats.node_count(), 3);
        assert_eq!(stats.edge_count(), 2);
        assert_eq!(stats.work_units(), 5);
        assert_eq!(stats.maximum_depth(), 2);
        assert_eq!(
            stats.retained_footprint(),
            arena.retained_footprint().unwrap()
        );

        let kinds: Vec<_> = arena
            .iter()
            .map(|view| match view.node() {
                ProvenanceNodeView::Source(identity) => identity.resource().as_str(),
                ProvenanceNodeView::Operation(_) => "operation",
            })
            .collect();
        assert_eq!(kinds, ["models/a.obj", "models/b.obj", "operation"]);

        let ProvenanceNodeView::Operation(operation) = arena.view(&merged).unwrap().node() else {
            panic!("merged attachment must resolve to an operation");
        };
        let parents: Vec<_> = operation
            .parents()
            .map(|parent| match parent.node() {
                ProvenanceNodeView::Source(identity) => identity.resource().as_str(),
                ProvenanceNodeView::Operation(_) => "operation",
            })
            .collect();
        assert_eq!(parents, ["models/a.obj", "models/b.obj"]);
    }

    #[test]
    fn public_views_and_debug_output_do_not_expose_internal_node_ids() {
        let (arena, provenance) = test_provenance_fixture();
        assert_eq!(
            format!("{provenance:?}"),
            "Provenance { attachment: \"opaque\" }"
        );
        let ProvenanceNodeView::Source(identity) = arena.view(&provenance).unwrap().node() else {
            panic!("fixture attachment must resolve to a source");
        };
        assert_eq!(identity.resource().as_str(), "tests/fixture.asset");
    }

    #[test]
    fn one_provenance_attachment_is_non_empty_inline_storage() {
        let (_arena, provenance) = test_provenance_fixture();
        let attachments = ProvenanceAttachments::from(provenance);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments.first(), provenance);
        assert_eq!(attachments.iter().collect::<Vec<_>>(), [provenance]);
        assert_eq!(attachments.retained_allocation_cells(), 0);
        assert_eq!(attachments.retained_allocation_bytes(), 0);
        assert_eq!(
            format!("{attachments:?}"),
            "ProvenanceAttachments { count: 1 }"
        );
    }

    #[test]
    fn many_provenance_attachments_are_bounded_ordered_and_compact() {
        let (_arena, [first, second]) = test_provenance_pair_fixture();
        let mut source = Vec::with_capacity(1_024);
        source.extend([first, second]);
        let source_capacity = source.capacity();

        let attachments = ProvenanceAttachments::try_from_iter(source).unwrap();

        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments.first(), first);
        assert_eq!(attachments.iter().collect::<Vec<_>>(), [first, second]);
        assert!(attachments.retained_allocation_cells() >= 1);
        assert!(attachments.retained_allocation_cells() < source_capacity);
        assert_eq!(
            attachments.retained_allocation_bytes(),
            attachments.retained_allocation_cells() * std::mem::size_of::<Provenance>()
        );
        assert_eq!(
            format!("{attachments:?}"),
            "ProvenanceAttachments { count: 2 }"
        );

        assert_eq!(
            ProvenanceAttachments::try_from_iter(std::iter::empty()),
            Err(ProvenanceAttachmentsError::Empty)
        );
        assert_eq!(
            ProvenanceAttachments::try_from_iter(std::iter::repeat_n(
                first,
                MAX_PROVENANCE_ATTACHMENTS_PER_OWNER + 1,
            )),
            Err(ProvenanceAttachmentsError::LimitExceeded {
                maximum: MAX_PROVENANCE_ATTACHMENTS_PER_OWNER,
            })
        );
    }

    #[test]
    fn loose_attachment_size_hint_does_not_retain_empty_slots() {
        let (_arena, [first, second]) = test_provenance_pair_fixture();
        let attachments = ProvenanceAttachments::try_from_iter(std::iter::once(first).chain(
            std::iter::repeat_n(second, MAX_PROVENANCE_ATTACHMENTS_PER_OWNER - 1).filter(|_| false),
        ))
        .unwrap();

        assert_eq!(attachments.iter().collect::<Vec<_>>(), [first]);
        assert_eq!(attachments.retained_allocation_cells(), 0);
    }

    #[test]
    fn validation_reports_stable_parent_problems_without_node_ids() {
        let source = source_node(source_identity("models/a.obj", "A"));
        let operation = operation_node(
            operation(OperationKind::Merge),
            vec![ProvenanceNodeId(1)],
            2,
        );
        let arena = arena_from_nodes(vec![source, operation], 1);

        let error = validate_provenance_arena(&arena).unwrap_err();
        assert_eq!(
            error.issue(),
            ProvenanceValidationIssue::ParentNotBeforeChild
        );
        assert_eq!(
            error.to_string(),
            "invalid provenance arena: provenance parent does not precede its child"
        );
    }

    #[test]
    fn controlled_validation_charges_partial_work_before_failure_or_cancellation() {
        let source = source_node(source_identity("models/a.obj", "A"));
        let operation = operation_node(
            operation(OperationKind::Merge),
            vec![ProvenanceNodeId(1)],
            2,
        );
        let arena = arena_from_nodes(vec![source, operation], 1);

        let mut semantic_work = 0usize;
        let semantic_error = validate_provenance_arena_controlled(&arena, &mut || {
            semantic_work += 1;
            Ok::<(), &'static str>(())
        })
        .unwrap_err();
        match semantic_error {
            ControlledProvenanceValidationError::Validation(error) => {
                assert_eq!(
                    error.issue(),
                    ProvenanceValidationIssue::ParentNotBeforeChild
                );
                assert_eq!(error.stats().work_units(), semantic_work);
            }
            ControlledProvenanceValidationError::Control(error) => {
                panic!("semantic failure must not become a control failure: {error}")
            }
        }

        let mut cancelled_work = 0usize;
        let cancellation = validate_provenance_arena_controlled(&arena, &mut || {
            cancelled_work += 1;
            if cancelled_work == 2 {
                Err("cancelled")
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(matches!(
            cancellation,
            ControlledProvenanceValidationError::Control("cancelled")
        ));
        assert_eq!(cancelled_work, 2);
    }

    #[test]
    fn validation_checks_parent_range_duplicate_and_canonical_order() {
        let cases = [
            (
                vec![ProvenanceNodeId(3)],
                ProvenanceValidationIssue::ParentOutOfRange,
            ),
            (
                vec![ProvenanceNodeId(0), ProvenanceNodeId(0)],
                ProvenanceValidationIssue::DuplicateParent,
            ),
            (
                vec![ProvenanceNodeId(1), ProvenanceNodeId(0)],
                ProvenanceValidationIssue::ParentOrderNotCanonical,
            ),
        ];

        for (parents, expected) in cases {
            let edge_count = parents.len();
            let arena = arena_from_nodes(
                vec![
                    source_node(source_identity("models/a.obj", "A")),
                    source_node(source_identity("models/b.obj", "B")),
                    operation_node(operation(OperationKind::Merge), parents, 2),
                ],
                edge_count,
            );

            assert_eq!(
                validate_provenance_arena(&arena).unwrap_err().issue(),
                expected
            );
        }
    }

    #[test]
    fn finish_revalidates_builder_topology_before_publication() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let mut node = operation_node(
            operation(OperationKind::Generated),
            vec![ProvenanceNodeId(0)],
            2,
        );
        let ProvenanceNodePayload::Operation { parents, .. } = &mut node.payload else {
            unreachable!("test node is an operation");
        };
        parents[0].arena = builder.nonce;
        let dynamic_bytes = checked_node_dynamic_bytes(&node).unwrap();
        let stored = SharedStorage::try_new_prepared(node, dynamic_bytes).unwrap();
        builder.retained_node_bytes = stored.retained_allocation_bytes();
        builder.local_retained_node_bytes = builder.retained_node_bytes;
        builder.local_nodes.push(stored);
        builder.node_count = 1;
        builder.edge_count = 1;

        assert_eq!(
            builder.finish().unwrap_err().issue(),
            ProvenanceValidationIssue::ParentNotBeforeChild
        );
    }

    #[test]
    fn fork_clones_lineage_and_remaps_attachments_for_branch_local_edits() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let locator_schema = ExtensionSchemaId::new("obj:statement", 7)
            .expect("test source-locator schema should be canonical");
        let source_identity = SourceIdentity::new(
            ResourceIdentity::new("models/a.obj").unwrap(),
            FormatId::new("org.baozi.obj").unwrap(),
            SourceObjectLocator::try_namespaced(
                Some(locator_schema.clone()),
                SourceLocatorValue::new("object/ship").unwrap(),
            )
            .expect("schema-bound source locator should be accepted"),
        )
        .with_display_name(SourceDisplayName::new("A").unwrap())
        .with_content_digest(ContentDigest::sha256([7; DIGEST_LEN]));
        let source = builder.add_source(source_identity).unwrap();
        let evidence = OperationEvidenceSummary::new(
            NamespacedName::new("obj:normalized").unwrap(),
            1,
            Some(EvidenceDigest::sha256([8; DIGEST_LEN])),
        )
        .unwrap();
        let normalized = builder
            .add_operation(
                OperationIdentity::new(
                    OperationKind::Normalize,
                    OptionsDigest::sha256([9; DIGEST_LEN]),
                    [evidence],
                )
                .unwrap(),
                [source],
            )
            .unwrap();
        let original = builder.finish().unwrap();

        let mut fork = original.try_fork().unwrap();
        assert_eq!(fork.builder().node_count(), original.node_count());
        assert_eq!(fork.remap().mapped_node_count(), original.node_count());
        assert!(
            original
                .segments
                .iter()
                .zip(&fork.builder().segments)
                .all(|(source, target)| source.ptr_eq(target)),
            "forked prefixes must share immutable segment allocations"
        );
        assert_eq!(
            fork.builder()
                .retained_footprint()
                .unwrap()
                .shared_node_bytes(),
            original.retained_footprint().unwrap().shared_node_bytes()
        );
        let mapped_source = fork.remap_attachment(&source).unwrap();
        let mapped_normalized = fork.remap_attachment(&normalized).unwrap();
        assert_eq!(mapped_source, source);
        assert_eq!(mapped_normalized, normalized);
        let repaired = fork
            .builder_mut()
            .add_operation(operation(OperationKind::Repair), [normalized])
            .unwrap();
        let (branch, remap) = fork.finish().unwrap();

        assert_eq!(branch.node_count(), original.node_count() + 1);
        assert_eq!(remap.remap(&source).unwrap(), mapped_source);
        assert!(original.contains(&mapped_source));
        assert!(branch.contains(&source));
        let ProvenanceNodeView::Source(mapped_identity) =
            branch.view(&mapped_source).unwrap().node()
        else {
            panic!("mapped source must remain a source node");
        };
        assert_eq!(mapped_identity.display_name().unwrap().as_str(), "A");
        assert_eq!(
            mapped_identity.content_digest(),
            Some(&ContentDigest::sha256([7; DIGEST_LEN]))
        );
        let SourceObjectLocator::Namespaced(locator) = mapped_identity.locator() else {
            panic!("mapped source must retain its namespaced locator");
        };
        assert_eq!(locator.kind().as_str(), "obj:statement");
        assert_eq!(locator.schema_id(), &locator_schema);
        assert_eq!(locator.value().as_str(), "object/ship");
        let ProvenanceNodeView::Operation(mapped_operation) =
            branch.view(&mapped_normalized).unwrap().node()
        else {
            panic!("mapped operation must remain an operation node");
        };
        assert_eq!(
            mapped_operation
                .identity()
                .evidence()
                .next()
                .unwrap()
                .code()
                .as_str(),
            "obj:normalized"
        );
        let ProvenanceNodeView::Operation(repair) = branch.view(&repaired).unwrap().node() else {
            panic!("branch-local attachment must resolve to its operation");
        };
        assert_eq!(repair.identity().kind(), &OperationKind::Repair);
        assert_eq!(repair.parent_count(), 1);
    }

    #[test]
    fn descendant_fork_shares_all_ancestor_segments_and_accepts_their_attachments() {
        let (root, source) = test_provenance_fixture();
        let mut first_fork = root.try_fork().unwrap();
        let normalized = first_fork
            .builder_mut()
            .add_operation(operation(OperationKind::Normalize), [source])
            .unwrap();
        let (first_branch, _) = first_fork.finish().unwrap();
        assert_eq!(first_branch.segments.len(), 2);

        let mut descendant = first_branch.try_fork().unwrap();
        assert_eq!(descendant.remap_attachment(&source).unwrap(), source);
        assert_eq!(
            descendant.remap_attachment(&normalized).unwrap(),
            normalized
        );
        assert!(
            first_branch
                .segments
                .iter()
                .zip(&descendant.builder().segments)
                .all(|(ancestor, inherited)| ancestor.ptr_eq(inherited))
        );
        let repaired = descendant
            .builder_mut()
            .add_operation(operation(OperationKind::Repair), [normalized, source])
            .unwrap();
        let (second_branch, _) = descendant.finish().unwrap();

        assert_eq!(second_branch.segments.len(), 3);
        assert!(second_branch.contains(&source));
        assert!(second_branch.contains(&normalized));
        assert!(second_branch.contains(&repaired));
        assert_eq!(second_branch.iter().len(), 3);
        assert_eq!(second_branch.iter().rev().count(), 3);
    }

    #[test]
    fn fork_remap_rejects_foreign_and_stale_source_attachments() {
        let (source_arena, source) = test_provenance_fixture();
        let (foreign_arena, foreign) = test_provenance_fixture();
        let fork = source_arena.try_fork().unwrap();

        assert_eq!(
            fork.remap_attachment(&foreign).unwrap_err(),
            ProvenanceRemapError::ForeignSourceArena
        );
        let stale = Provenance {
            arena: source_arena.nonce,
            segment: 0,
            node: ProvenanceNodeId(source_arena.node_count()),
            topological_index: source_arena.node_count(),
        };
        assert_eq!(
            fork.remap_attachment(&stale).unwrap_err(),
            ProvenanceRemapError::StaleSourceAttachment
        );
        let stale_topology = Provenance {
            topological_index: source.topological_index + 1,
            ..source
        };
        assert_eq!(
            fork.remap_attachment(&stale_topology).unwrap_err(),
            ProvenanceRemapError::StaleSourceAttachment
        );
        assert_eq!(
            source_arena.view(&stale_topology).unwrap_err(),
            ProvenanceAccessError::AttachmentOutOfRange
        );
        let stale_segment = Provenance {
            segment: source.segment + 1,
            ..source
        };
        assert_eq!(
            fork.remap_attachment(&stale_segment).unwrap_err(),
            ProvenanceRemapError::ForeignSourceArena
        );
        assert!(source_arena.contains(&source));
        assert!(foreign_arena.contains(&foreign));
    }

    #[test]
    fn fork_remap_clone_is_allocation_free_and_preserves_target_qualification() {
        let (source_arena, source) = test_provenance_fixture();
        let fork = source_arena.try_fork().unwrap();
        let remap = fork.remap().clone();
        let (target_arena, _) = fork.finish().unwrap();
        fail_next_allocation_for_test();

        let cloned = remap.clone();

        assert_eq!(cloned, remap);
        assert!(remap.retained_allocation_bytes() > 0);
        assert_eq!(
            cloned.retained_allocation_bytes(),
            remap.retained_allocation_bytes()
        );
        assert_eq!(cloned.remap(&source).unwrap(), source);
        assert_eq!(cloned.validate_target_arena(&target_arena), Ok(()));
        assert_eq!(
            cloned.validate_target_arena(&source_arena),
            Err(ProvenanceRemapError::ForeignTargetArena)
        );
        assert_eq!(
            SharedStorage::try_new(1_u8).err(),
            Some(SharedStorageError::AllocationFailed),
            "cloning a fork remap must not consume the next shared allocation attempt"
        );
    }

    #[test]
    fn sibling_forks_use_isolated_attachment_namespaces() {
        let (original, source) = test_provenance_fixture();
        let mut first = original.try_fork().unwrap();
        let mut second = original.try_fork().unwrap();
        let first_source = first.remap_attachment(&source).unwrap();
        let second_source = second.remap_attachment(&source).unwrap();

        assert_eq!(source, first_source);
        assert_eq!(first_source, second_source);
        let first_local = first
            .builder_mut()
            .add_operation(operation(OperationKind::Repair), [source])
            .unwrap();
        let second_local = second
            .builder_mut()
            .add_operation(operation(OperationKind::Normalize), [source])
            .unwrap();
        let (first_arena, _) = first.finish().unwrap();
        let (second_arena, _) = second.finish().unwrap();
        assert!(first_arena.contains(&source));
        assert!(second_arena.contains(&source));
        assert!(first_arena.contains(&first_local));
        assert!(second_arena.contains(&second_local));
        assert!(!first_arena.contains(&second_local));
        assert!(!second_arena.contains(&first_local));
        assert!(first_arena.contains(&first_source));
        assert!(second_arena.contains(&second_source));
        assert_eq!(
            first_arena.view(&second_local).unwrap_err(),
            ProvenanceAccessError::ArenaMismatch
        );
        assert_eq!(
            second_arena.view(&first_local).unwrap_err(),
            ProvenanceAccessError::ArenaMismatch
        );
    }

    #[test]
    fn fork_reports_typed_allocation_and_limit_failures() {
        let mut bytes = Vec::<u8>::new();
        assert_eq!(
            try_reserve_for_fork(&mut bytes, usize::MAX, ProvenanceAllocation::ForkSegments,)
                .unwrap_err(),
            ProvenanceForkError::AllocationFailed {
                resource: ProvenanceAllocation::ForkSegments,
            }
        );

        let summary =
            OperationEvidenceSummary::new(NamespacedName::new("obj:repair").unwrap(), 1, None)
                .unwrap();
        let oversized_identity = OperationIdentity {
            kind: OperationKind::Repair,
            options_digest: OptionsDigest::sha256([0; DIGEST_LEN]),
            evidence: vec![summary; MAX_OPERATION_EVIDENCE_SUMMARIES + 1],
        };
        let arena = arena_from_nodes(vec![operation_node(oversized_identity, Vec::new(), 1)], 0);
        assert_eq!(
            arena.try_fork().unwrap_err(),
            ProvenanceForkError::LimitExceeded {
                limit: ProvenanceLimit::OperationEvidenceSummaries,
                maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
            }
        );
    }

    #[test]
    fn shared_node_allocation_failure_leaves_builder_logically_unchanged() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        fail_next_allocation_for_test();
        assert_eq!(
            builder
                .add_source(source_identity("models/failure.obj", "Failure"))
                .unwrap_err(),
            ProvenanceBuildError::AllocationFailed {
                resource: ProvenanceAllocation::SharedNode,
            }
        );
        assert_eq!(builder.node_count(), 0);
        assert_eq!(builder.edge_count(), 0);

        let source = builder
            .add_source(source_identity("models/retry.obj", "Retry"))
            .unwrap();
        let arena = builder.finish().unwrap();
        assert!(arena.contains(&source));
    }

    #[test]
    fn arena_capacity_qualification_failure_is_transactional_and_retryable() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let before = builder.retained_footprint().unwrap();

        fail_next_arena_capacity_qualification_for_test();
        assert_eq!(
            builder
                .add_source(source_identity("models/qualification.obj", "Qualification"))
                .unwrap_err(),
            ProvenanceBuildError::AllocationFailed {
                resource: ProvenanceAllocation::ArenaNodes,
            }
        );
        assert_eq!(builder.node_count(), 0);
        assert_eq!(builder.edge_count(), 0);
        assert_eq!(builder.retained_footprint().unwrap(), before);

        let source = builder
            .add_source(source_identity("models/retry.obj", "Retry"))
            .unwrap();
        let arena = builder.finish().unwrap();
        assert!(arena.contains(&source));
    }

    #[test]
    fn retained_byte_limit_accepts_exact_footprint_and_rejects_one_byte_less() {
        let mut reference = ProvenanceArenaBuilder::try_new().unwrap();
        reference
            .add_source(source_identity("models/retained.obj", "Retained"))
            .unwrap();
        let exact_bytes = reference.retained_footprint().unwrap().total_bytes();
        assert!(exact_bytes > 0);

        let exact_limits = ProvenanceLimits {
            max_retained_bytes: exact_bytes,
            ..ProvenanceLimits::hard()
        };
        let mut exact = ProvenanceArenaBuilder::try_with_limits(exact_limits).unwrap();
        exact
            .add_source(source_identity("models/retained.obj", "Retained"))
            .unwrap();
        assert_eq!(
            exact.retained_footprint().unwrap().total_bytes(),
            exact_bytes
        );

        let below_limits = ProvenanceLimits {
            max_retained_bytes: exact_bytes - 1,
            ..ProvenanceLimits::hard()
        };
        let mut below = ProvenanceArenaBuilder::try_with_limits(below_limits).unwrap();
        assert_eq!(
            below
                .add_source(source_identity("models/retained.obj", "Retained"))
                .unwrap_err(),
            ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: exact_bytes - 1,
            }
        );
        assert_eq!(below.node_count(), 0);
        assert_eq!(below.edge_count(), 0);
        assert_eq!(below.retained_footprint().unwrap().total_bytes(), 0);
    }

    #[test]
    fn fork_pointer_allocation_failure_is_transactional() {
        let (arena, source) = test_provenance_fixture();
        let before = arena.validate().unwrap().stats();

        fail_next_fork_reserve_for_test();
        assert_eq!(
            arena.try_fork().unwrap_err(),
            ProvenanceForkError::AllocationFailed {
                resource: ProvenanceAllocation::ForkSegments,
            }
        );
        assert!(arena.contains(&source));
        assert_eq!(arena.validate().unwrap().stats(), before);

        let fork = arena.try_fork().unwrap();
        let mapped = fork.remap_attachment(&source).unwrap();
        let (branch, _) = fork.finish().unwrap();
        assert!(branch.contains(&mapped));
    }

    #[test]
    fn fork_snapshot_shared_allocation_failure_is_transactional() {
        let (arena, source) = test_provenance_fixture();
        let before = arena.validate().unwrap().stats();
        fail_next_allocation_for_test();

        assert_eq!(
            arena.try_fork().unwrap_err(),
            ProvenanceForkError::AllocationFailed {
                resource: ProvenanceAllocation::ForkSnapshot,
            }
        );
        assert!(arena.contains(&source));
        assert_eq!(arena.validate().unwrap().stats(), before);

        let fork = arena.try_fork().unwrap();
        let mapped = fork.remap_attachment(&source).unwrap();
        let (branch, _) = fork.finish().unwrap();
        assert!(branch.contains(&mapped));
    }

    #[test]
    fn fork_identity_failure_precedes_snapshot_shared_allocation() {
        let (arena, source) = test_provenance_fixture();
        let before = arena.validate().unwrap().stats();
        fail_next_arena_nonce_for_test();
        fail_next_allocation_for_test();

        assert_eq!(
            arena.try_fork().unwrap_err(),
            ProvenanceForkError::ArenaIdentityExhausted
        );
        assert!(arena.contains(&source));
        assert_eq!(arena.validate().unwrap().stats(), before);
        match SharedStorage::try_new(()) {
            Err(error) => assert_eq!(
                error,
                SharedStorageError::AllocationFailed,
                "identity rejection must leave the snapshot allocation sentinel untouched"
            ),
            Ok(_) => panic!("identity rejection consumed the snapshot allocation sentinel"),
        }
    }

    #[test]
    fn fork_retained_limit_accounts_for_source_snapshot_storage() {
        let (mut arena, source) = test_provenance_fixture();
        let segment_capacity = CapacityPlan::for_type::<SharedStorage<ProvenanceSegment>>(
            0,
            0,
            arena.segments.len() + 1,
        )
        .unwrap();
        let arena_envelope = checked_segments_retained_footprint(
            &arena.segments,
            segment_capacity.envelope_capacity(),
            arena.retained_node_bytes,
        )
        .unwrap()
        .total_bytes();
        let snapshot_capacity =
            CapacityPlan::for_type::<ProvenanceSegmentSnapshot>(0, 0, arena.segments.len())
                .unwrap()
                .envelope_capacity();
        let snapshot_bytes =
            SharedStorage::<Vec<ProvenanceSegmentSnapshot>>::checked_layout_footprint()
                .unwrap()
                .checked_add(
                    checked_capacity_bytes::<ProvenanceSegmentSnapshot>(snapshot_capacity).unwrap(),
                )
                .unwrap();
        let exact_bytes = arena_envelope.checked_add(snapshot_bytes).unwrap();

        arena.limits.max_retained_bytes = exact_bytes;
        let mut fork = arena.try_fork().unwrap();
        assert_eq!(fork.remap_attachment(&source).unwrap(), source);
        let exact_branch_bytes = fork.builder().retained_footprint().unwrap().total_bytes();
        fork.builder_mut().limits.max_retained_bytes = exact_branch_bytes;
        assert_eq!(
            fork.builder_mut()
                .add_source(source_identity("models/over-budget.obj", "Over budget"))
                .unwrap_err(),
            ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: exact_branch_bytes,
            }
        );
        drop(fork);

        arena.limits.max_retained_bytes = exact_bytes - 1;
        assert_eq!(
            arena.try_fork().unwrap_err(),
            ProvenanceForkError::LimitExceeded {
                limit: ProvenanceLimit::RetainedBytes,
                maximum: exact_bytes - 1,
            }
        );
        assert!(arena.contains(&source));
    }

    #[test]
    fn builder_and_validator_enforce_depth_and_linear_work_limits() {
        let mut builder =
            ProvenanceArenaBuilder::try_with_limits(limits_with_depth_and_work(2, 16)).unwrap();
        let source = builder
            .add_source(source_identity("models/depth.obj", "Depth"))
            .unwrap();
        let normalized = builder
            .add_operation(operation(OperationKind::Normalize), [source])
            .unwrap();
        assert_eq!(
            builder
                .add_operation(operation(OperationKind::Repair), [normalized])
                .unwrap_err(),
            ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ArenaDepth,
                maximum: 2,
            }
        );
        assert_eq!(builder.node_count(), 2);
        assert_eq!(builder.edge_count(), 1);
        let arena = builder.finish().unwrap();

        let depth_error = arena
            .validate_with_limits(limits_with_depth_and_work(1, 16))
            .unwrap_err();
        assert_eq!(
            depth_error.issue(),
            ProvenanceValidationIssue::DepthLimitExceeded
        );
        assert_eq!(depth_error.stats().node_count(), 2);

        let work_error = arena
            .validate_with_limits(limits_with_depth_and_work(2, 2))
            .unwrap_err();
        assert_eq!(
            work_error.issue(),
            ProvenanceValidationIssue::WorkLimitExceeded
        );
        assert_eq!(work_error.stats().work_units(), 2);
        assert_eq!(work_error.stats().node_count(), 2);
        assert_eq!(work_error.stats().edge_count(), 0);

        let mut work_limited =
            ProvenanceArenaBuilder::try_with_limits(limits_with_depth_and_work(4, 2)).unwrap();
        let source = work_limited
            .add_source(source_identity("models/work.obj", "Work"))
            .unwrap();
        assert_eq!(
            work_limited
                .add_operation(operation(OperationKind::Normalize), [source])
                .unwrap_err(),
            ProvenanceBuildError::LimitExceeded {
                limit: ProvenanceLimit::ValidationWork,
                maximum: 2,
            }
        );
        assert_eq!(work_limited.node_count(), 1);
        assert_eq!(work_limited.edge_count(), 0);
    }

    #[test]
    fn validator_rejects_noncanonical_stored_depth() {
        let mut arena = arena_from_nodes(
            vec![
                source_node(source_identity("models/a.obj", "A")),
                operation_node(
                    operation(OperationKind::Normalize),
                    vec![ProvenanceNodeId(0)],
                    2,
                ),
            ],
            1,
        );
        arena.segments[0].get_mut().unwrap().nodes[1]
            .get_mut()
            .unwrap()
            .depth = 3;

        assert_eq!(
            arena.validate().unwrap_err().issue(),
            ProvenanceValidationIssue::StoredDepthMismatch
        );
    }

    #[test]
    fn footprint_uses_capacities_and_includes_arc_layout() {
        let mut resource = String::with_capacity(400);
        resource.push_str("models/capacity.obj");
        let resource_capacity = resource.capacity();
        let source = SourceIdentity::new(
            ResourceIdentity(resource),
            FormatId::new("org.baozi.obj").unwrap(),
            SourceObjectLocator::indexed(SourceObjectKind::Mesh, 0),
        );

        let mut evidence = Vec::with_capacity(MAX_OPERATION_EVIDENCE_SUMMARIES);
        evidence.push(
            OperationEvidenceSummary::new(NamespacedName::new("obj:capacity").unwrap(), 1, None)
                .unwrap(),
        );
        let evidence_capacity = evidence.capacity();
        let operation = OperationIdentity {
            kind: OperationKind::Normalize,
            options_digest: OptionsDigest::sha256([0; DIGEST_LEN]),
            evidence,
        };
        let mut parents = Vec::with_capacity(128);
        parents.push(ProvenanceNodeId(0));
        let parent_capacity = parents.capacity();
        let arena = arena_from_nodes(
            vec![source_node(source), operation_node(operation, parents, 2)],
            1,
        );

        let footprint = arena.retained_footprint().unwrap();
        let stats = arena.validate().unwrap().stats();
        let arc_bytes = SharedStorage::<ProvenanceNode>::checked_layout_footprint().unwrap();
        let expected_shared = arc_bytes
            .checked_mul(2)
            .unwrap()
            .checked_add(resource_capacity)
            .unwrap()
            .checked_add(evidence_capacity * std::mem::size_of::<OperationEvidenceSummary>())
            .unwrap()
            .checked_add(parent_capacity * std::mem::size_of::<Provenance>())
            .unwrap();

        assert_eq!(footprint.shared_node_bytes(), expected_shared);
        assert_eq!(stats.retained_footprint(), footprint);
        assert!(footprint.total_bytes() > resource_capacity);
    }

    #[test]
    fn arena_node_growth_uses_amortized_capacity_instead_of_exact_reallocation() {
        const NODE_COUNT: usize = 16_384;

        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let identity = source_identity("models/repeated.obj", "Repeated");
        let mut capacity = builder.local_nodes.capacity();
        let mut capacity_growths = 0usize;
        for _ in 0..NODE_COUNT {
            builder.add_source(identity.try_clone().unwrap()).unwrap();
            let next_capacity = builder.local_nodes.capacity();
            if next_capacity != capacity {
                capacity_growths += 1;
                capacity = next_capacity;
            }
        }

        assert_eq!(builder.node_count(), NODE_COUNT);
        assert!(builder.local_nodes.capacity() >= NODE_COUNT);
        assert!(capacity_growths <= usize::BITS as usize);
    }

    #[test]
    fn evidence_is_bounded_unique_and_canonically_ordered() {
        let second =
            OperationEvidenceSummary::new(NamespacedName::new("obj:warning").unwrap(), 2, None)
                .unwrap();
        let first = OperationEvidenceSummary::new(
            NamespacedName::new("obj:repair").unwrap(),
            1,
            Some(EvidenceDigest::blake3([3; DIGEST_LEN])),
        )
        .unwrap();
        let identity = OperationIdentity::new(
            OperationKind::Repair,
            OptionsDigest::sha256([4; DIGEST_LEN]),
            [second.clone(), first],
        )
        .unwrap();
        assert_eq!(
            identity
                .evidence()
                .map(|summary| summary.code().as_str())
                .collect::<Vec<_>>(),
            ["obj:repair", "obj:warning"]
        );

        assert_eq!(
            OperationIdentity::new(
                OperationKind::Repair,
                OptionsDigest::sha256([4; DIGEST_LEN]),
                [second.clone(), second],
            )
            .unwrap_err(),
            OperationIdentityError::DuplicateEvidenceCode
        );
    }

    #[test]
    fn loose_evidence_and_parent_size_hints_do_not_retain_empty_slots() {
        let evidence =
            OperationEvidenceSummary::new(NamespacedName::new("obj:warning").unwrap(), 1, None)
                .unwrap();
        let identity = OperationIdentity::new(
            OperationKind::Repair,
            OptionsDigest::sha256([5; DIGEST_LEN]),
            std::iter::repeat_n(evidence, MAX_OPERATION_EVIDENCE_SUMMARIES).filter(|_| false),
        )
        .unwrap();
        assert_eq!(identity.evidence.capacity(), 0);

        let limits = ProvenanceLimits::default();
        let mut builder = ProvenanceArenaBuilder::try_with_limits(limits).unwrap();
        let source = builder
            .add_source(source_identity("models/source.obj", "source"))
            .unwrap();
        let operation = builder
            .add_operation(
                identity,
                std::iter::repeat_n(source, limits.max_parents_per_operation()).filter(|_| false),
            )
            .unwrap();
        let stored: &ProvenanceNode = builder.local_nodes[operation.node.0].borrow();
        let ProvenanceNodePayload::Operation { parents, .. } = &stored.payload else {
            panic!("the inserted provenance node must be an operation");
        };
        assert_eq!(parents.capacity(), 0);
    }
}
