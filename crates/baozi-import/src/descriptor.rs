use crate::dependency::{DependencyRequirement, DependencyRoleId};
use baozi_core::FormatId;
use std::collections::BTreeMap;
use std::fmt;
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::num::NonZeroU64;

const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_EXTENSION_COUNT: usize = 32;
const MAX_EXTENSION_BYTES: usize = 32;
const MAX_MEDIA_TYPE_COUNT: usize = 32;
const MAX_MEDIA_TYPE_BYTES: usize = 128;
const MAX_DEPENDENCY_ROLE_COUNT: usize = 32;
const MAX_NOTES_BYTES: usize = 1_024;
const MAX_DOCS_PATH_BYTES: usize = 256;

/// Maturity claimed by one registered format implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FormatMaturity {
    Experimental,
    Beta,
    Stable,
    Deprecated,
}

/// Degree to which one format capability is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CapabilityStatus {
    Supported,
    Partial,
    ParsedLossy,
    IgnoredWithDiagnostic,
    Unsupported,
    Unknown,
}

/// Canonical capability dimensions advertised by a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FormatCapability {
    Geometry,
    Hierarchy,
    Materials,
    Textures,
    CamerasLights,
    Animation,
    Skinning,
    MorphTargets,
    Metadata,
    CompressionContainers,
    CoordinatesUnits,
    Diagnostics,
    ResourceLimits,
}

/// Primary encoding family consumed by a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatEncoding {
    Text,
    Binary,
    TextOrBinary,
}

/// A source encoding admitted by a validated text profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextSourceEncoding {
    Utf8,
}

/// How a UTF-8 byte order mark at raw byte offset zero is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextBomPolicy {
    Forbidden,
    Accepted,
    AcceptedWithDiagnostic,
}

/// One raw newline form recognized by a text grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextNewline {
    Lf,
    CrLf,
    Cr,
}

/// Relationship between ASCII grammar bytes and decoded textual values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextLexicalMode {
    Utf8Document,
    AsciiSyntaxUtf8Values,
    AsciiOnly,
}

/// Canonical UTF-8 decoding behavior selectable by a format-owned option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextDecodingMode {
    RejectInvalid,
    ReplaceInvalidUtf8,
}

/// Handling of decoded control characters outside declared newline forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TextControlPolicy {
    /// Reject every control character, including horizontal tab.
    RejectAll,
    /// Permit horizontal tab while still rejecting NUL and all other controls.
    AllowHorizontalTab,
}

trait TextProfileSetValue: Copy + 'static {
    const VALUES: &'static [Self];

    fn bit(self) -> u8;
}

impl TextProfileSetValue for TextSourceEncoding {
    const VALUES: &'static [Self] = &[Self::Utf8];

    fn bit(self) -> u8 {
        match self {
            Self::Utf8 => 1 << 0,
        }
    }
}

impl TextProfileSetValue for TextNewline {
    const VALUES: &'static [Self] = &[Self::Lf, Self::CrLf, Self::Cr];

    fn bit(self) -> u8 {
        match self {
            Self::Lf => 1 << 0,
            Self::CrLf => 1 << 1,
            Self::Cr => 1 << 2,
        }
    }
}

impl TextProfileSetValue for TextDecodingMode {
    const VALUES: &'static [Self] = &[Self::RejectInvalid, Self::ReplaceInvalidUtf8];

    fn bit(self) -> u8 {
        match self {
            Self::RejectInvalid => 1 << 0,
            Self::ReplaceInvalidUtf8 => 1 << 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextProfileSet<T> {
    bits: u8,
    marker: PhantomData<fn() -> T>,
}

impl<T: TextProfileSetValue> TextProfileSet<T> {
    const fn empty() -> Self {
        Self {
            bits: 0,
            marker: PhantomData,
        }
    }

    fn from_values(values: impl IntoIterator<Item = T>) -> Result<Self, T> {
        let mut set = Self::empty();
        for value in values {
            if !set.insert(value) {
                return Err(value);
            }
        }
        Ok(set)
    }

    const fn is_empty(self) -> bool {
        self.bits == 0
    }

    fn insert(&mut self, value: T) -> bool {
        let bit = value.bit();
        let inserted = self.bits & bit == 0;
        self.bits |= bit;
        inserted
    }

    fn contains(self, value: T) -> bool {
        self.bits & value.bit() != 0
    }

    const fn iter(self) -> TextProfileSetIter<T> {
        TextProfileSetIter {
            remaining: self.bits,
            marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
struct TextProfileSetIter<T> {
    remaining: u8,
    marker: PhantomData<fn() -> T>,
}

impl<T: TextProfileSetValue> Iterator for TextProfileSetIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining != 0 {
            let index = self.remaining.trailing_zeros() as usize;
            self.remaining &= self.remaining - 1;
            if let Some(value) = T::VALUES.get(index) {
                return Some(*value);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len();
        (remaining, Some(remaining))
    }
}

impl<T: TextProfileSetValue> ExactSizeIterator for TextProfileSetIter<T> {
    fn len(&self) -> usize {
        self.remaining.count_ones() as usize
    }
}

impl<T: TextProfileSetValue> FusedIterator for TextProfileSetIter<T> {}

/// A required immutable text-profile hard-cap dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextHardCapDimension {
    RawBytes,
    DecodedUtf8Bytes,
    LineIndexBytes,
    Tokens,
    CanonicalStringBytes,
}

/// Immutable format hard caps that may only lower request and backend limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextHardCaps {
    max_raw_bytes: NonZeroU64,
    max_decoded_utf8_bytes: NonZeroU64,
    max_line_index_bytes: NonZeroU64,
    max_tokens: NonZeroU64,
    max_canonical_string_bytes: NonZeroU64,
}

impl TextHardCaps {
    pub fn new(
        max_raw_bytes: u64,
        max_decoded_utf8_bytes: u64,
        max_line_index_bytes: u64,
        max_tokens: u64,
        max_canonical_string_bytes: u64,
    ) -> Result<Self, TextProfileError> {
        Ok(Self {
            max_raw_bytes: required_hard_cap(max_raw_bytes, TextHardCapDimension::RawBytes)?,
            max_decoded_utf8_bytes: required_hard_cap(
                max_decoded_utf8_bytes,
                TextHardCapDimension::DecodedUtf8Bytes,
            )?,
            max_line_index_bytes: required_hard_cap(
                max_line_index_bytes,
                TextHardCapDimension::LineIndexBytes,
            )?,
            max_tokens: required_hard_cap(max_tokens, TextHardCapDimension::Tokens)?,
            max_canonical_string_bytes: required_hard_cap(
                max_canonical_string_bytes,
                TextHardCapDimension::CanonicalStringBytes,
            )?,
        })
    }

    pub const fn max_raw_bytes(self) -> u64 {
        self.max_raw_bytes.get()
    }

    pub const fn max_decoded_utf8_bytes(self) -> u64 {
        self.max_decoded_utf8_bytes.get()
    }

    pub const fn max_line_index_bytes(self) -> u64 {
        self.max_line_index_bytes.get()
    }

    pub const fn max_tokens(self) -> u64 {
        self.max_tokens.get()
    }

    pub const fn max_canonical_string_bytes(self) -> u64 {
        self.max_canonical_string_bytes.get()
    }
}

fn required_hard_cap(
    value: u64,
    dimension: TextHardCapDimension,
) -> Result<NonZeroU64, TextProfileError> {
    NonZeroU64::new(value).ok_or(TextProfileError::ZeroHardCap { dimension })
}

/// Immutable text decoding, lexical, and hard-cap claims for one format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextProfile {
    source_encodings: TextProfileSet<TextSourceEncoding>,
    bom_policy: TextBomPolicy,
    newlines: TextProfileSet<TextNewline>,
    lexical_mode: TextLexicalMode,
    decoding_modes: TextProfileSet<TextDecodingMode>,
    control_policy: TextControlPolicy,
    hard_caps: TextHardCaps,
}

impl TextProfile {
    pub fn builder(hard_caps: TextHardCaps) -> TextProfileBuilder {
        TextProfileBuilder::new(hard_caps)
    }

    pub fn source_encodings(&self) -> impl ExactSizeIterator<Item = TextSourceEncoding> + '_ {
        self.source_encodings.iter()
    }

    pub const fn bom_policy(&self) -> TextBomPolicy {
        self.bom_policy
    }

    pub fn newlines(&self) -> impl ExactSizeIterator<Item = TextNewline> + '_ {
        self.newlines.iter()
    }

    pub const fn lexical_mode(&self) -> TextLexicalMode {
        self.lexical_mode
    }

    pub fn decoding_modes(&self) -> impl ExactSizeIterator<Item = TextDecodingMode> + '_ {
        self.decoding_modes.iter()
    }

    pub fn allows_decoding_mode(&self, mode: TextDecodingMode) -> bool {
        self.decoding_modes.contains(mode)
    }

    pub const fn control_policy(&self) -> TextControlPolicy {
        self.control_policy
    }

    pub const fn hard_caps(&self) -> &TextHardCaps {
        &self.hard_caps
    }
}

/// Fallible construction surface for a [`TextProfile`].
#[derive(Debug)]
pub struct TextProfileBuilder {
    source_encodings: Result<TextProfileSet<TextSourceEncoding>, TextSourceEncoding>,
    bom_policy: Option<TextBomPolicy>,
    newlines: Result<TextProfileSet<TextNewline>, TextNewline>,
    lexical_mode: Option<TextLexicalMode>,
    decoding_modes: Result<TextProfileSet<TextDecodingMode>, TextDecodingMode>,
    control_policy: Option<TextControlPolicy>,
    hard_caps: TextHardCaps,
}

impl TextProfileBuilder {
    fn new(hard_caps: TextHardCaps) -> Self {
        Self {
            source_encodings: Ok(TextProfileSet::empty()),
            bom_policy: None,
            newlines: Ok(TextProfileSet::empty()),
            lexical_mode: None,
            decoding_modes: Ok(TextProfileSet::empty()),
            control_policy: None,
            hard_caps,
        }
    }

    pub fn source_encodings(
        mut self,
        encodings: impl IntoIterator<Item = TextSourceEncoding>,
    ) -> Self {
        self.source_encodings = TextProfileSet::from_values(encodings);
        self
    }

    pub const fn bom_policy(mut self, policy: TextBomPolicy) -> Self {
        self.bom_policy = Some(policy);
        self
    }

    pub fn newlines(mut self, newlines: impl IntoIterator<Item = TextNewline>) -> Self {
        self.newlines = TextProfileSet::from_values(newlines);
        self
    }

    pub const fn lexical_mode(mut self, mode: TextLexicalMode) -> Self {
        self.lexical_mode = Some(mode);
        self
    }

    pub fn decoding_modes(mut self, modes: impl IntoIterator<Item = TextDecodingMode>) -> Self {
        self.decoding_modes = TextProfileSet::from_values(modes);
        self
    }

    pub const fn control_policy(mut self, policy: TextControlPolicy) -> Self {
        self.control_policy = Some(policy);
        self
    }

    pub fn build(self) -> Result<TextProfile, TextProfileError> {
        let source_encodings = self
            .source_encodings
            .map_err(|encoding| TextProfileError::DuplicateSourceEncoding { encoding })?;
        if source_encodings.is_empty() {
            return Err(TextProfileError::EmptySet {
                field: TextProfileField::SourceEncodings,
            });
        }
        let newlines = self
            .newlines
            .map_err(|newline| TextProfileError::DuplicateNewline { newline })?;
        if newlines.is_empty() {
            return Err(TextProfileError::EmptySet {
                field: TextProfileField::Newlines,
            });
        }
        let decoding_modes = self
            .decoding_modes
            .map_err(|mode| TextProfileError::DuplicateDecodingMode { mode })?;
        if decoding_modes.is_empty() {
            return Err(TextProfileError::EmptySet {
                field: TextProfileField::DecodingModes,
            });
        }
        let bom_policy = self.bom_policy.ok_or(TextProfileError::MissingField {
            field: TextProfileField::BomPolicy,
        })?;
        let lexical_mode = self.lexical_mode.ok_or(TextProfileError::MissingField {
            field: TextProfileField::LexicalMode,
        })?;
        let control_policy = self.control_policy.ok_or(TextProfileError::MissingField {
            field: TextProfileField::ControlPolicy,
        })?;

        if !decoding_modes.contains(TextDecodingMode::RejectInvalid) {
            return Err(TextProfileError::RequiredDecodingModeMissing {
                mode: TextDecodingMode::RejectInvalid,
            });
        }
        if lexical_mode == TextLexicalMode::AsciiOnly
            && decoding_modes.contains(TextDecodingMode::ReplaceInvalidUtf8)
        {
            return Err(TextProfileError::RecoveryNotSupported {
                mode: TextDecodingMode::ReplaceInvalidUtf8,
                lexical_mode,
            });
        }

        Ok(TextProfile {
            source_encodings,
            bom_policy,
            newlines,
            lexical_mode,
            decoding_modes,
            control_policy,
            hard_caps: self.hard_caps,
        })
    }
}

/// Text-profile field used by structured validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextProfileField {
    SourceEncodings,
    BomPolicy,
    Newlines,
    LexicalMode,
    DecodingModes,
    ControlPolicy,
}

/// Why immutable text decoding claims could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextProfileError {
    MissingField {
        field: TextProfileField,
    },
    EmptySet {
        field: TextProfileField,
    },
    DuplicateSourceEncoding {
        encoding: TextSourceEncoding,
    },
    DuplicateNewline {
        newline: TextNewline,
    },
    DuplicateDecodingMode {
        mode: TextDecodingMode,
    },
    ZeroHardCap {
        dimension: TextHardCapDimension,
    },
    RequiredDecodingModeMissing {
        mode: TextDecodingMode,
    },
    RecoveryNotSupported {
        mode: TextDecodingMode,
        lexical_mode: TextLexicalMode,
    },
}

impl fmt::Display for TextProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { field } => {
                write!(formatter, "missing text profile field {field:?}")
            }
            Self::EmptySet { field } => write!(formatter, "text profile field {field:?} is empty"),
            Self::DuplicateSourceEncoding { encoding } => {
                write!(
                    formatter,
                    "text profile repeats source encoding {encoding:?}"
                )
            }
            Self::DuplicateNewline { newline } => {
                write!(formatter, "text profile repeats newline form {newline:?}")
            }
            Self::DuplicateDecodingMode { mode } => {
                write!(formatter, "text profile repeats decoding mode {mode:?}")
            }
            Self::ZeroHardCap { dimension } => {
                write!(
                    formatter,
                    "text profile hard cap {dimension:?} must be nonzero"
                )
            }
            Self::RequiredDecodingModeMissing { mode } => {
                write!(
                    formatter,
                    "text profile is missing required decoding mode {mode:?}"
                )
            }
            Self::RecoveryNotSupported { mode, lexical_mode } => write!(
                formatter,
                "text decoding mode {mode:?} is incompatible with lexical mode {lexical_mode:?}"
            ),
        }
    }
}

impl std::error::Error for TextProfileError {}

/// Immutable, validated claims associated with one format identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDescriptor {
    id: FormatId,
    display_name: Box<str>,
    extensions: Box<[Box<str>]>,
    media_types: Box<[Box<str>]>,
    encoding: FormatEncoding,
    text_profile: Option<TextProfile>,
    maturity: FormatMaturity,
    capabilities: BTreeMap<FormatCapability, CapabilityStatus>,
    dependency_roles: BTreeMap<DependencyRoleId, DependencyRequirement>,
    notes: Box<str>,
    docs_path: Option<Box<str>>,
}

impl FormatDescriptor {
    /// Starts a validated descriptor for a stable format identity.
    pub fn builder(id: FormatId, display_name: impl Into<String>) -> FormatDescriptorBuilder {
        FormatDescriptorBuilder::new(id, display_name.into())
    }

    pub fn id(&self) -> &FormatId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn extensions(&self) -> impl ExactSizeIterator<Item = &str> {
        self.extensions.iter().map(Box::as_ref)
    }

    pub fn media_types(&self) -> impl ExactSizeIterator<Item = &str> {
        self.media_types.iter().map(Box::as_ref)
    }

    pub const fn encoding(&self) -> FormatEncoding {
        self.encoding
    }

    pub const fn text_profile(&self) -> Option<&TextProfile> {
        self.text_profile.as_ref()
    }

    pub const fn maturity(&self) -> FormatMaturity {
        self.maturity
    }

    pub fn capabilities(
        &self,
    ) -> impl ExactSizeIterator<Item = (FormatCapability, CapabilityStatus)> + '_ {
        self.capabilities
            .iter()
            .map(|(capability, status)| (*capability, *status))
    }

    pub fn capability_status(&self, capability: FormatCapability) -> Option<CapabilityStatus> {
        self.capabilities.get(&capability).copied()
    }

    pub fn dependency_roles(
        &self,
    ) -> impl ExactSizeIterator<Item = (&DependencyRoleId, DependencyRequirement)> {
        self.dependency_roles
            .iter()
            .map(|(role, requirement)| (role, *requirement))
    }

    pub(crate) fn dependency_requirement(
        &self,
        role: &DependencyRoleId,
    ) -> Option<DependencyRequirement> {
        self.dependency_roles.get(role).copied()
    }

    pub fn notes(&self) -> &str {
        &self.notes
    }

    pub fn docs_path(&self) -> Option<&str> {
        self.docs_path.as_deref()
    }
}

/// Fallible construction surface for a [`FormatDescriptor`].
#[derive(Debug)]
pub struct FormatDescriptorBuilder {
    id: FormatId,
    display_name: String,
    extensions: Vec<String>,
    media_types: Vec<String>,
    encoding: Option<FormatEncoding>,
    text_profile: Option<TextProfile>,
    maturity: FormatMaturity,
    capabilities: Vec<(FormatCapability, CapabilityStatus)>,
    dependency_roles: Vec<(DependencyRoleId, DependencyRequirement)>,
    notes: String,
    docs_path: Option<String>,
}

impl FormatDescriptorBuilder {
    fn new(id: FormatId, display_name: String) -> Self {
        Self {
            id,
            display_name,
            extensions: Vec::new(),
            media_types: Vec::new(),
            encoding: None,
            text_profile: None,
            maturity: FormatMaturity::Experimental,
            capabilities: Vec::new(),
            dependency_roles: Vec::new(),
            notes: String::new(),
            docs_path: None,
        }
    }

    pub fn extensions(mut self, extensions: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extensions = extensions.into_iter().map(Into::into).collect();
        self
    }

    pub fn media_types(mut self, media_types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.media_types = media_types.into_iter().map(Into::into).collect();
        self
    }

    pub const fn encoding(mut self, encoding: FormatEncoding) -> Self {
        self.encoding = Some(encoding);
        self
    }

    pub fn text_profile(mut self, profile: TextProfile) -> Self {
        self.text_profile = Some(profile);
        self
    }

    pub const fn maturity(mut self, maturity: FormatMaturity) -> Self {
        self.maturity = maturity;
        self
    }

    pub fn capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = (FormatCapability, CapabilityStatus)>,
    ) -> Self {
        self.capabilities = capabilities.into_iter().collect();
        self
    }

    pub fn dependency_roles(
        mut self,
        roles: impl IntoIterator<Item = (DependencyRoleId, DependencyRequirement)>,
    ) -> Self {
        self.dependency_roles = roles.into_iter().collect();
        self
    }

    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = notes.into();
        self
    }

    pub fn docs_path(mut self, docs_path: impl Into<String>) -> Self {
        self.docs_path = Some(docs_path.into());
        self
    }

    pub fn build(self) -> Result<FormatDescriptor, DescriptorError> {
        validate_bounded_text(
            DescriptorField::DisplayName,
            &self.display_name,
            MAX_DISPLAY_NAME_BYTES,
            false,
        )?;

        let extensions = validate_extensions(self.extensions)?;
        let media_types = validate_media_types(self.media_types)?;
        let capabilities = validate_capabilities(self.capabilities)?;
        let dependency_roles = validate_dependency_roles(self.dependency_roles)?;
        validate_bounded_text(DescriptorField::Notes, &self.notes, MAX_NOTES_BYTES, true)?;
        if let Some(path) = self.docs_path.as_deref() {
            validate_docs_path(path)?;
        }
        let encoding = self.encoding.ok_or(DescriptorError::MissingField {
            field: DescriptorField::Encoding,
        })?;
        if matches!(
            encoding,
            FormatEncoding::Text | FormatEncoding::TextOrBinary
        ) && self.text_profile.is_none()
        {
            return Err(DescriptorError::MissingField {
                field: DescriptorField::TextProfile,
            });
        }

        Ok(FormatDescriptor {
            id: self.id,
            display_name: self.display_name.into_boxed_str(),
            extensions,
            media_types,
            encoding,
            text_profile: self.text_profile,
            maturity: self.maturity,
            capabilities,
            dependency_roles,
            notes: self.notes.into_boxed_str(),
            docs_path: self.docs_path.map(String::into_boxed_str),
        })
    }
}

/// Descriptor field used by structured validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorField {
    DisplayName,
    Extensions,
    MediaTypes,
    Encoding,
    TextProfile,
    Capabilities,
    DependencyRoles,
    Notes,
    DocsPath,
}

/// Why immutable format claims could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescriptorError {
    MissingField {
        field: DescriptorField,
    },
    EmptyField {
        field: DescriptorField,
    },
    TooManyValues {
        field: DescriptorField,
        actual: usize,
        maximum: usize,
    },
    ValueTooLong {
        field: DescriptorField,
        actual: usize,
        maximum: usize,
    },
    InvalidValue {
        field: DescriptorField,
        value: Box<str>,
    },
    DuplicateValue {
        field: DescriptorField,
        value: Box<str>,
    },
    DuplicateCapability {
        capability: FormatCapability,
    },
    DuplicateDependencyRole {
        role: DependencyRoleId,
    },
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { field } => write!(formatter, "missing descriptor field {field:?}"),
            Self::EmptyField { field } => write!(formatter, "descriptor field {field:?} is empty"),
            Self::TooManyValues {
                field,
                actual,
                maximum,
            } => write!(
                formatter,
                "descriptor field {field:?} has {actual} values; the maximum is {maximum}"
            ),
            Self::ValueTooLong {
                field,
                actual,
                maximum,
            } => write!(
                formatter,
                "descriptor field {field:?} is {actual} bytes; the maximum is {maximum}"
            ),
            Self::InvalidValue { field, value } => {
                write!(
                    formatter,
                    "descriptor field {field:?} contains invalid value {value:?}"
                )
            }
            Self::DuplicateValue { field, value } => {
                write!(
                    formatter,
                    "descriptor field {field:?} repeats value {value:?}"
                )
            }
            Self::DuplicateCapability { capability } => {
                write!(formatter, "descriptor repeats capability {capability:?}")
            }
            Self::DuplicateDependencyRole { role } => {
                write!(formatter, "descriptor repeats dependency role {role}")
            }
        }
    }
}

impl std::error::Error for DescriptorError {}

fn validate_extensions(values: Vec<String>) -> Result<Box<[Box<str>]>, DescriptorError> {
    validate_value_count(
        DescriptorField::Extensions,
        values.len(),
        MAX_EXTENSION_COUNT,
    )?;
    let mut normalized = BTreeMap::<String, ()>::new();
    for value in values {
        if value.is_empty() {
            return Err(DescriptorError::EmptyField {
                field: DescriptorField::Extensions,
            });
        }
        if value.len() > MAX_EXTENSION_BYTES {
            return Err(DescriptorError::ValueTooLong {
                field: DescriptorField::Extensions,
                actual: value.len(),
                maximum: MAX_EXTENSION_BYTES,
            });
        }
        if !is_canonical_label(&value) {
            return Err(DescriptorError::InvalidValue {
                field: DescriptorField::Extensions,
                value: value.into_boxed_str(),
            });
        }
        if normalized.insert(value.clone(), ()).is_some() {
            return Err(DescriptorError::DuplicateValue {
                field: DescriptorField::Extensions,
                value: value.into_boxed_str(),
            });
        }
    }
    Ok(normalized
        .into_keys()
        .map(String::into_boxed_str)
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

fn validate_media_types(values: Vec<String>) -> Result<Box<[Box<str>]>, DescriptorError> {
    validate_value_count(
        DescriptorField::MediaTypes,
        values.len(),
        MAX_MEDIA_TYPE_COUNT,
    )?;
    let mut normalized = BTreeMap::<String, ()>::new();
    for value in values {
        if value.len() > MAX_MEDIA_TYPE_BYTES {
            return Err(DescriptorError::ValueTooLong {
                field: DescriptorField::MediaTypes,
                actual: value.len(),
                maximum: MAX_MEDIA_TYPE_BYTES,
            });
        }
        let mut parts = value.split('/');
        let valid = matches!((parts.next(), parts.next(), parts.next()), (Some(left), Some(right), None)
            if !left.is_empty() && !right.is_empty() && left.bytes().all(is_media_type_byte)
                && right.bytes().all(is_media_type_byte));
        if !valid {
            return Err(DescriptorError::InvalidValue {
                field: DescriptorField::MediaTypes,
                value: value.into_boxed_str(),
            });
        }
        if normalized.insert(value.clone(), ()).is_some() {
            return Err(DescriptorError::DuplicateValue {
                field: DescriptorField::MediaTypes,
                value: value.into_boxed_str(),
            });
        }
    }
    Ok(normalized
        .into_keys()
        .map(String::into_boxed_str)
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

fn validate_capabilities(
    values: Vec<(FormatCapability, CapabilityStatus)>,
) -> Result<BTreeMap<FormatCapability, CapabilityStatus>, DescriptorError> {
    let mut capabilities = BTreeMap::new();
    for (capability, status) in values {
        if capabilities.insert(capability, status).is_some() {
            return Err(DescriptorError::DuplicateCapability { capability });
        }
    }
    Ok(capabilities)
}

fn validate_dependency_roles(
    values: Vec<(DependencyRoleId, DependencyRequirement)>,
) -> Result<BTreeMap<DependencyRoleId, DependencyRequirement>, DescriptorError> {
    validate_value_count(
        DescriptorField::DependencyRoles,
        values.len(),
        MAX_DEPENDENCY_ROLE_COUNT,
    )?;
    let mut roles = BTreeMap::new();
    for (role, requirement) in values {
        if roles.insert(role.clone(), requirement).is_some() {
            return Err(DescriptorError::DuplicateDependencyRole { role });
        }
    }
    Ok(roles)
}

fn validate_value_count(
    field: DescriptorField,
    actual: usize,
    maximum: usize,
) -> Result<(), DescriptorError> {
    if actual > maximum {
        Err(DescriptorError::TooManyValues {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn validate_bounded_text(
    field: DescriptorField,
    value: &str,
    maximum: usize,
    allow_empty: bool,
) -> Result<(), DescriptorError> {
    if !allow_empty && value.is_empty() {
        return Err(DescriptorError::EmptyField { field });
    }
    if value.len() > maximum {
        return Err(DescriptorError::ValueTooLong {
            field,
            actual: value.len(),
            maximum,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(DescriptorError::InvalidValue {
            field,
            value: value.into(),
        });
    }
    Ok(())
}

fn validate_docs_path(path: &str) -> Result<(), DescriptorError> {
    validate_bounded_text(DescriptorField::DocsPath, path, MAX_DOCS_PATH_BYTES, false)?;
    let invalid = path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if invalid {
        return Err(DescriptorError::InvalidValue {
            field: DescriptorField::DocsPath,
            value: path.into(),
        });
    }
    Ok(())
}

fn is_media_type_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase()
        || byte.is_ascii_digit()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
        )
}

fn is_canonical_label(value: &str) -> bool {
    let mut previous_separator = true;
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => previous_separator = false,
            b'-' if !previous_separator => previous_separator = true,
            _ => return false,
        }
    }
    !previous_separator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_hard_caps() -> TextHardCaps {
        TextHardCaps::new(1024, 2048, 512, 128, 4096).unwrap()
    }

    fn text_profile_builder() -> TextProfileBuilder {
        TextProfile::builder(text_hard_caps())
            .source_encodings([TextSourceEncoding::Utf8])
            .bom_policy(TextBomPolicy::Forbidden)
            .newlines([TextNewline::Lf, TextNewline::CrLf])
            .lexical_mode(TextLexicalMode::AsciiSyntaxUtf8Values)
            .decoding_modes([
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ])
            .control_policy(TextControlPolicy::AllowHorizontalTab)
    }

    fn text_profile() -> TextProfile {
        text_profile_builder().build().unwrap()
    }

    fn descriptor() -> FormatDescriptorBuilder {
        FormatDescriptor::builder(FormatId::new("obj").unwrap(), "Wavefront OBJ")
            .extensions(["obj"])
            .media_types(["model/obj", "text/plain"])
            .encoding(FormatEncoding::Text)
            .text_profile(text_profile())
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
    }

    #[test]
    fn text_descriptors_require_a_validated_text_profile() {
        assert!(matches!(
            FormatDescriptor::builder(FormatId::new("text").unwrap(), "Text")
                .encoding(FormatEncoding::Text)
                .build(),
            Err(DescriptorError::MissingField {
                field: DescriptorField::TextProfile,
            })
        ));
        assert!(matches!(
            FormatDescriptor::builder(FormatId::new("mixed").unwrap(), "Mixed")
                .encoding(FormatEncoding::TextOrBinary)
                .build(),
            Err(DescriptorError::MissingField {
                field: DescriptorField::TextProfile,
            })
        ));

        let descriptor = descriptor().build().unwrap();
        assert_eq!(descriptor.text_profile(), Some(&text_profile()));
    }

    #[test]
    fn binary_descriptors_may_omit_or_declare_a_text_profile() {
        let without_profile = FormatDescriptor::builder(FormatId::new("binary").unwrap(), "Binary")
            .encoding(FormatEncoding::Binary)
            .build()
            .unwrap();
        assert_eq!(without_profile.text_profile(), None);

        let with_profile =
            FormatDescriptor::builder(FormatId::new("container").unwrap(), "Container")
                .encoding(FormatEncoding::Binary)
                .text_profile(text_profile())
                .build()
                .unwrap();
        assert_eq!(with_profile.text_profile(), Some(&text_profile()));
    }

    #[test]
    fn text_profile_retains_every_explicit_contract_dimension() {
        let profile = text_profile();
        assert_eq!(
            profile.source_encodings().collect::<Vec<_>>(),
            [TextSourceEncoding::Utf8]
        );
        assert_eq!(profile.bom_policy(), TextBomPolicy::Forbidden);
        assert_eq!(
            profile.newlines().collect::<Vec<_>>(),
            [TextNewline::Lf, TextNewline::CrLf]
        );
        assert_eq!(
            profile.lexical_mode(),
            TextLexicalMode::AsciiSyntaxUtf8Values
        );
        assert_eq!(
            profile.decoding_modes().collect::<Vec<_>>(),
            [
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ]
        );
        assert_eq!(
            profile.control_policy(),
            TextControlPolicy::AllowHorizontalTab
        );
        assert_eq!(profile.hard_caps(), &text_hard_caps());
    }

    #[test]
    fn profile_set_iteration_is_stable_and_exact_sized() {
        assert!(!std::mem::needs_drop::<TextProfile>());
        assert!(!std::mem::needs_drop::<TextProfileBuilder>());

        let profile = text_profile_builder()
            .newlines([TextNewline::Cr, TextNewline::CrLf, TextNewline::Lf])
            .decoding_modes([
                TextDecodingMode::ReplaceInvalidUtf8,
                TextDecodingMode::RejectInvalid,
            ])
            .build()
            .unwrap();

        let mut newlines = profile.newlines();
        assert_eq!(newlines.len(), 3);
        assert_eq!(newlines.next(), Some(TextNewline::Lf));
        assert_eq!(newlines.len(), 2);
        assert_eq!(
            newlines.collect::<Vec<_>>(),
            [TextNewline::CrLf, TextNewline::Cr]
        );
        assert_eq!(
            profile.decoding_modes().collect::<Vec<_>>(),
            [
                TextDecodingMode::RejectInvalid,
                TextDecodingMode::ReplaceInvalidUtf8,
            ]
        );
    }

    #[test]
    fn every_text_hard_cap_is_explicitly_nonzero() {
        for (dimension, result) in [
            (
                TextHardCapDimension::RawBytes,
                TextHardCaps::new(0, 1, 1, 1, 1),
            ),
            (
                TextHardCapDimension::DecodedUtf8Bytes,
                TextHardCaps::new(1, 0, 1, 1, 1),
            ),
            (
                TextHardCapDimension::LineIndexBytes,
                TextHardCaps::new(1, 1, 0, 1, 1),
            ),
            (
                TextHardCapDimension::Tokens,
                TextHardCaps::new(1, 1, 1, 0, 1),
            ),
            (
                TextHardCapDimension::CanonicalStringBytes,
                TextHardCaps::new(1, 1, 1, 1, 0),
            ),
        ] {
            assert_eq!(result, Err(TextProfileError::ZeroHardCap { dimension }));
        }
    }

    #[test]
    fn profile_sets_must_be_non_empty() {
        for (field, result) in [
            (
                TextProfileField::SourceEncodings,
                text_profile_builder().source_encodings([]).build(),
            ),
            (
                TextProfileField::Newlines,
                text_profile_builder().newlines([]).build(),
            ),
            (
                TextProfileField::DecodingModes,
                text_profile_builder().decoding_modes([]).build(),
            ),
        ] {
            assert_eq!(result, Err(TextProfileError::EmptySet { field }));
        }
    }

    #[test]
    fn profile_sets_reject_duplicates_without_unbounded_collection() {
        assert_eq!(
            text_profile_builder()
                .source_encodings(std::iter::repeat(TextSourceEncoding::Utf8))
                .build(),
            Err(TextProfileError::DuplicateSourceEncoding {
                encoding: TextSourceEncoding::Utf8,
            })
        );
        assert_eq!(
            text_profile_builder()
                .newlines([TextNewline::Cr, TextNewline::Lf, TextNewline::Cr])
                .build(),
            Err(TextProfileError::DuplicateNewline {
                newline: TextNewline::Cr,
            })
        );
        assert_eq!(
            text_profile_builder()
                .decoding_modes([
                    TextDecodingMode::ReplaceInvalidUtf8,
                    TextDecodingMode::RejectInvalid,
                    TextDecodingMode::ReplaceInvalidUtf8,
                ])
                .build(),
            Err(TextProfileError::DuplicateDecodingMode {
                mode: TextDecodingMode::ReplaceInvalidUtf8,
            })
        );
    }

    #[test]
    fn profile_recovery_claims_require_strict_mode_and_compatible_lexing() {
        assert!(matches!(
            text_profile_builder()
                .lexical_mode(TextLexicalMode::AsciiOnly)
                .build(),
            Err(TextProfileError::RecoveryNotSupported {
                mode: TextDecodingMode::ReplaceInvalidUtf8,
                lexical_mode: TextLexicalMode::AsciiOnly,
            })
        ));
        assert!(matches!(
            text_profile_builder()
                .decoding_modes([TextDecodingMode::ReplaceInvalidUtf8])
                .build(),
            Err(TextProfileError::RequiredDecodingModeMissing {
                mode: TextDecodingMode::RejectInvalid,
            })
        ));
    }

    #[test]
    fn construction_normalizes_stable_collection_order() {
        let descriptor = descriptor()
            .extensions(["z", "obj", "a"])
            .media_types(["text/plain", "model/obj"])
            .build()
            .unwrap();

        assert_eq!(
            descriptor.extensions().collect::<Vec<_>>(),
            ["a", "obj", "z"]
        );
        assert_eq!(
            descriptor.media_types().collect::<Vec<_>>(),
            ["model/obj", "text/plain"]
        );
    }

    #[test]
    fn duplicate_and_noncanonical_routing_claims_fail() {
        assert!(matches!(
            descriptor().extensions(["obj", "obj"]).build(),
            Err(DescriptorError::DuplicateValue {
                field: DescriptorField::Extensions,
                ..
            })
        ));
        assert!(matches!(
            descriptor().extensions([".OBJ"]).build(),
            Err(DescriptorError::InvalidValue {
                field: DescriptorField::Extensions,
                ..
            })
        ));
        assert!(matches!(
            descriptor().extensions(["obj--binary"]).build(),
            Err(DescriptorError::InvalidValue {
                field: DescriptorField::Extensions,
                ..
            })
        ));
        assert!(matches!(
            descriptor()
                .capabilities([
                    (FormatCapability::Geometry, CapabilityStatus::Supported),
                    (FormatCapability::Geometry, CapabilityStatus::Partial),
                ])
                .build(),
            Err(DescriptorError::DuplicateCapability {
                capability: FormatCapability::Geometry
            })
        ));
    }

    #[test]
    fn docs_path_cannot_escape_the_repository_view() {
        assert!(matches!(
            descriptor().docs_path("../private.md").build(),
            Err(DescriptorError::InvalidValue {
                field: DescriptorField::DocsPath,
                ..
            })
        ));
    }

    #[test]
    fn dependency_roles_freeze_requiredness_and_reject_duplicates() {
        let role = DependencyRoleId::new("material.library").unwrap();
        let built_descriptor = descriptor()
            .dependency_roles([(role.clone(), DependencyRequirement::Optional)])
            .build()
            .unwrap();
        assert_eq!(
            built_descriptor.dependency_requirement(&role),
            Some(DependencyRequirement::Optional)
        );

        assert!(matches!(
            descriptor()
                .dependency_roles([
                    (role.clone(), DependencyRequirement::Optional),
                    (role.clone(), DependencyRequirement::Required),
                ])
                .build(),
            Err(DescriptorError::DuplicateDependencyRole { role: duplicate })
                if duplicate == role
        ));
    }
}
