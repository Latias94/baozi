//! Bounded descriptive values and declarative extension schemas.
//!
//! Generic extension values deliberately cannot contain scene handles,
//! provenance-arena addresses, nested maps, executable callbacks, or backend
//! objects. Behavior-bearing descriptors refer to a declarative schema in the
//! scene-owned [`ExtensionSchemaSnapshot`] instead of storing behavior in
//! metadata.

use crate::attribute::AttributeDomain;
use crate::identity::SceneEntityKind;
use crate::math::{Color, Mat4, Vec2, Vec3, Vec4};
use crate::namespaced::{NamespacedName, NamespacedNameError};
use crate::provenance::{MAX_SOURCE_LOCATOR_LEN, Provenance};
use baozi_operation::{CapacityPlan, GrowableCapacityPlan};
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};
use thiserror::Error;

/// Maximum UTF-8 bytes in one generic extension string.
pub const MAX_EXTENSION_STRING_BYTES: usize = 4 * 1024;

/// Maximum bytes in one generic extension byte payload.
pub const MAX_EXTENSION_BYTE_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Maximum bytes in one canonical media type.
pub const MAX_EXTENSION_MEDIA_TYPE_BYTES: usize = 128;

/// Maximum logical elements in one generic dense extension array.
pub const MAX_EXTENSION_ARRAY_ELEMENTS: usize = 65_536;

/// Maximum declarative schemas retained by one scene snapshot.
pub const MAX_EXTENSION_SCHEMAS: usize = 4_096;

/// Maximum retained bytes in one registered packed attribute payload.
pub const MAX_PACKED_ATTRIBUTE_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

/// Maximum logical elements validated in one registered packed payload.
pub const MAX_PACKED_ATTRIBUTE_ELEMENTS: usize = 16_777_216;

/// A bounded canonical media type without parameters.
///
/// Both type and subtype use lowercase RFC token bytes, separated by exactly
/// one `/`. Parameters are intentionally excluded so equal media types have
/// one spelling.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionMediaType {
    bytes: [u8; MAX_EXTENSION_MEDIA_TYPE_BYTES],
    length: u8,
}

impl ExtensionMediaType {
    /// Validates and stores one canonical media type without allocating.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ExtensionMediaTypeError> {
        let value = value.as_ref();
        validate_media_type(value)?;

        let mut bytes = [0; MAX_EXTENSION_MEDIA_TYPE_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        let length = u8::try_from(value.len()).map_err(|_| ExtensionMediaTypeError::TooLong {
            actual: value.len(),
            maximum: MAX_EXTENSION_MEDIA_TYPE_BYTES,
        })?;
        Ok(Self { bytes, length })
    }

    /// Returns the canonical media type.
    pub fn as_str(&self) -> &str {
        let length = usize::from(self.length);
        std::str::from_utf8(&self.bytes[..length])
            .expect("ExtensionMediaType construction accepts only ASCII bytes")
    }
}

impl AsRef<str> for ExtensionMediaType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for ExtensionMediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExtensionMediaType")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for ExtensionMediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a media type could not enter the canonical extension model.
///
/// Errors contain only lengths and offsets and never echo rejected text.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionMediaTypeError {
    #[error("extension media type is empty")]
    Empty,

    #[error("extension media type is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("extension media type must contain exactly one non-empty separator")]
    Separator,

    #[error("extension media type contains a non-canonical byte at offset {offset}")]
    InvalidByte { offset: usize },
}

fn validate_media_type(value: &str) -> Result<(), ExtensionMediaTypeError> {
    if value.is_empty() {
        return Err(ExtensionMediaTypeError::Empty);
    }
    if value.len() > MAX_EXTENSION_MEDIA_TYPE_BYTES {
        return Err(ExtensionMediaTypeError::TooLong {
            actual: value.len(),
            maximum: MAX_EXTENSION_MEDIA_TYPE_BYTES,
        });
    }

    let Some(separator) = value.bytes().position(|byte| byte == b'/') else {
        return Err(ExtensionMediaTypeError::Separator);
    };
    if separator == 0
        || separator + 1 == value.len()
        || value.as_bytes()[separator + 1..].contains(&b'/')
    {
        return Err(ExtensionMediaTypeError::Separator);
    }

    for (offset, byte) in value.bytes().enumerate() {
        if offset == separator {
            continue;
        }
        let valid = byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(
                byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            );
        if !valid {
            return Err(ExtensionMediaTypeError::InvalidByte { offset });
        }
    }

    Ok(())
}

/// The scalar type stored by one dense generic extension array.
///
/// String, byte, map, reference, and nested-array elements are absent by
/// design. In particular, no variant can encode a scene or provenance ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionArrayElementType {
    Bool,
    I64,
    U64,
    F64,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Mat4,
}

/// The closed category of one validated generic extension value.
///
/// This enum is descriptive only. It contains no scene-reference category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionValueKind {
    Bool,
    I64,
    U64,
    F64,
    String,
    Bytes,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Mat4,
    DenseArray(ExtensionArrayElementType),
}

#[derive(PartialEq)]
enum ExtensionArrayStorage {
    Bool(Vec<bool>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F64(Vec<f64>),
    Vec2(Vec<Vec2>),
    Vec3(Vec<Vec3>),
    Vec4(Vec<Vec4>),
    Color(Vec<Color>),
    Mat4(Vec<Mat4>),
}

impl ExtensionArrayStorage {
    fn element_type(&self) -> ExtensionArrayElementType {
        match self {
            Self::Bool(_) => ExtensionArrayElementType::Bool,
            Self::I64(_) => ExtensionArrayElementType::I64,
            Self::U64(_) => ExtensionArrayElementType::U64,
            Self::F64(_) => ExtensionArrayElementType::F64,
            Self::Vec2(_) => ExtensionArrayElementType::Vec2,
            Self::Vec3(_) => ExtensionArrayElementType::Vec3,
            Self::Vec4(_) => ExtensionArrayElementType::Vec4,
            Self::Color(_) => ExtensionArrayElementType::Color,
            Self::Mat4(_) => ExtensionArrayElementType::Mat4,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Bool(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::Vec2(values) => values.len(),
            Self::Vec3(values) => values.len(),
            Self::Vec4(values) => values.len(),
            Self::Color(values) => values.len(),
            Self::Mat4(values) => values.len(),
        }
    }

    fn payload_bytes(&self) -> usize {
        let element_size = match self.element_type() {
            ExtensionArrayElementType::Bool => std::mem::size_of::<bool>(),
            ExtensionArrayElementType::I64 => std::mem::size_of::<i64>(),
            ExtensionArrayElementType::U64 => std::mem::size_of::<u64>(),
            ExtensionArrayElementType::F64 => std::mem::size_of::<f64>(),
            ExtensionArrayElementType::Vec2 => std::mem::size_of::<Vec2>(),
            ExtensionArrayElementType::Vec3 => std::mem::size_of::<Vec3>(),
            ExtensionArrayElementType::Vec4 => std::mem::size_of::<Vec4>(),
            ExtensionArrayElementType::Color => std::mem::size_of::<Color>(),
            ExtensionArrayElementType::Mat4 => std::mem::size_of::<Mat4>(),
        };
        self.len().saturating_mul(element_size)
    }

    fn retained_allocation_bytes(&self) -> usize {
        match self {
            Self::Bool(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<bool>()),
            Self::I64(values) => values.capacity().saturating_mul(std::mem::size_of::<i64>()),
            Self::U64(values) => values.capacity().saturating_mul(std::mem::size_of::<u64>()),
            Self::F64(values) => values.capacity().saturating_mul(std::mem::size_of::<f64>()),
            Self::Vec2(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<Vec2>()),
            Self::Vec3(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<Vec3>()),
            Self::Vec4(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<Vec4>()),
            Self::Color(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<Color>()),
            Self::Mat4(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<Mat4>()),
        }
    }

    fn as_ref(&self) -> ExtensionArrayRef<'_> {
        match self {
            Self::Bool(values) => ExtensionArrayRef::Bool(values),
            Self::I64(values) => ExtensionArrayRef::I64(values),
            Self::U64(values) => ExtensionArrayRef::U64(values),
            Self::F64(values) => ExtensionArrayRef::F64(values),
            Self::Vec2(values) => ExtensionArrayRef::Vec2(values),
            Self::Vec3(values) => ExtensionArrayRef::Vec3(values),
            Self::Vec4(values) => ExtensionArrayRef::Vec4(values),
            Self::Color(values) => ExtensionArrayRef::Color(values),
            Self::Mat4(values) => ExtensionArrayRef::Mat4(values),
        }
    }
}

#[derive(PartialEq)]
enum ExtensionValueStorage {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Bytes {
        media_type: ExtensionMediaType,
        payload: Vec<u8>,
    },
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Color(Color),
    Mat4(Mat4),
    DenseArray(ExtensionArrayStorage),
}

/// One bounded generic extension or metadata value.
///
/// The representation is private so finite-number and allocation bounds cannot
/// be bypassed with enum construction. Heap-bearing values intentionally do
/// not implement `Clone`; use [`ExtensionValue::try_clone`] so every internal
/// copy remains fallible. No public constructor accepts a scene handle,
/// provenance attachment, arbitrary map, nested array, or backend object.
#[derive(PartialEq)]
pub struct ExtensionValue {
    storage: ExtensionValueStorage,
}

/// Backward-independent name for descriptive metadata values.
pub type MetadataValue = ExtensionValue;

impl ExtensionValue {
    pub const fn from_bool(value: bool) -> Self {
        Self {
            storage: ExtensionValueStorage::Bool(value),
        }
    }

    pub const fn from_i64(value: i64) -> Self {
        Self {
            storage: ExtensionValueStorage::I64(value),
        }
    }

    pub const fn from_u64(value: u64) -> Self {
        Self {
            storage: ExtensionValueStorage::U64(value),
        }
    }

    /// Constructs a finite `f64` value.
    pub fn try_f64(value: f64) -> Result<Self, ExtensionValueError> {
        if !value.is_finite() {
            return Err(ExtensionValueError::NonFiniteScalar {
                kind: ExtensionValueKind::F64,
            });
        }
        Ok(Self {
            storage: ExtensionValueStorage::F64(value),
        })
    }

    /// Validates and fallibly copies one bounded UTF-8 string.
    pub fn try_string(value: impl AsRef<str>) -> Result<Self, ExtensionValueError> {
        let value = value.as_ref();
        if value.len() > MAX_EXTENSION_STRING_BYTES {
            return Err(ExtensionValueError::StringTooLong {
                actual: value.len(),
                maximum: MAX_EXTENSION_STRING_BYTES,
            });
        }

        let mut owned = String::new();
        try_reserve_string(&mut owned, value.len(), ExtensionAllocation::String)?;
        owned.push_str(value);
        Ok(Self {
            storage: ExtensionValueStorage::String(owned),
        })
    }

    /// Validates a media type and fallibly copies one bounded byte payload.
    pub fn try_bytes(
        media_type: impl AsRef<str>,
        payload: impl AsRef<[u8]>,
    ) -> Result<Self, ExtensionValueError> {
        Self::try_bytes_with_media_type(ExtensionMediaType::new(media_type)?, payload)
    }

    /// Fallibly copies bytes under an already validated media type.
    pub fn try_bytes_with_media_type(
        media_type: ExtensionMediaType,
        payload: impl AsRef<[u8]>,
    ) -> Result<Self, ExtensionValueError> {
        let payload = payload.as_ref();
        if payload.len() > MAX_EXTENSION_BYTE_PAYLOAD_BYTES {
            return Err(ExtensionValueError::BytePayloadTooLong {
                actual: payload.len(),
                maximum: MAX_EXTENSION_BYTE_PAYLOAD_BYTES,
            });
        }
        let payload = try_copy_slice(payload, ExtensionAllocation::BytePayload)?;
        Ok(Self {
            storage: ExtensionValueStorage::Bytes {
                media_type,
                payload,
            },
        })
    }

    pub fn try_vec2(value: Vec2) -> Result<Self, ExtensionValueError> {
        require_finite(value.is_finite(), ExtensionValueKind::Vec2)?;
        Ok(Self {
            storage: ExtensionValueStorage::Vec2(value),
        })
    }

    pub fn try_vec3(value: Vec3) -> Result<Self, ExtensionValueError> {
        require_finite(value.is_finite(), ExtensionValueKind::Vec3)?;
        Ok(Self {
            storage: ExtensionValueStorage::Vec3(value),
        })
    }

    pub fn try_vec4(value: Vec4) -> Result<Self, ExtensionValueError> {
        require_finite(value.is_finite(), ExtensionValueKind::Vec4)?;
        Ok(Self {
            storage: ExtensionValueStorage::Vec4(value),
        })
    }

    pub fn try_color(value: Color) -> Result<Self, ExtensionValueError> {
        require_finite(color_is_finite(value), ExtensionValueKind::Color)?;
        Ok(Self {
            storage: ExtensionValueStorage::Color(value),
        })
    }

    pub fn try_mat4(value: Mat4) -> Result<Self, ExtensionValueError> {
        require_finite(value.is_finite(), ExtensionValueKind::Mat4)?;
        Ok(Self {
            storage: ExtensionValueStorage::Mat4(value),
        })
    }

    pub fn try_i64_array(values: &[i64]) -> Result<Self, ExtensionValueError> {
        Ok(Self::from_array(ExtensionArrayStorage::I64(
            try_copy_array(values)?,
        )))
    }

    pub fn try_bool_array(values: &[bool]) -> Result<Self, ExtensionValueError> {
        Ok(Self::from_array(ExtensionArrayStorage::Bool(
            try_copy_array(values)?,
        )))
    }

    pub fn try_u64_array(values: &[u64]) -> Result<Self, ExtensionValueError> {
        Ok(Self::from_array(ExtensionArrayStorage::U64(
            try_copy_array(values)?,
        )))
    }

    pub fn try_f64_array(values: &[f64]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::F64, |value| {
            value.is_finite()
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::F64(
            try_copy_array(values)?,
        )))
    }

    pub fn try_vec2_array(values: &[Vec2]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::Vec2, |value| {
            value.is_finite()
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::Vec2(
            try_copy_array(values)?,
        )))
    }

    pub fn try_vec3_array(values: &[Vec3]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::Vec3, |value| {
            value.is_finite()
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::Vec3(
            try_copy_array(values)?,
        )))
    }

    pub fn try_vec4_array(values: &[Vec4]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::Vec4, |value| {
            value.is_finite()
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::Vec4(
            try_copy_array(values)?,
        )))
    }

    pub fn try_color_array(values: &[Color]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::Color, |value| {
            color_is_finite(*value)
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::Color(
            try_copy_array(values)?,
        )))
    }

    pub fn try_mat4_array(values: &[Mat4]) -> Result<Self, ExtensionValueError> {
        validate_finite_array(values, ExtensionArrayElementType::Mat4, |value| {
            value.is_finite()
        })?;
        Ok(Self::from_array(ExtensionArrayStorage::Mat4(
            try_copy_array(values)?,
        )))
    }

    fn from_array(storage: ExtensionArrayStorage) -> Self {
        Self {
            storage: ExtensionValueStorage::DenseArray(storage),
        }
    }

    /// Fallibly copies this value without exposing its private representation.
    pub fn try_clone(&self) -> Result<Self, ExtensionValueError> {
        match self.as_ref() {
            ExtensionValueRef::Bool(value) => Ok(Self::from_bool(value)),
            ExtensionValueRef::I64(value) => Ok(Self::from_i64(value)),
            ExtensionValueRef::U64(value) => Ok(Self::from_u64(value)),
            ExtensionValueRef::F64(value) => Self::try_f64(value),
            ExtensionValueRef::String(value) => Self::try_string(value),
            ExtensionValueRef::Bytes {
                media_type,
                payload,
            } => Self::try_bytes_with_media_type(media_type.clone(), payload),
            ExtensionValueRef::Vec2(value) => Self::try_vec2(value),
            ExtensionValueRef::Vec3(value) => Self::try_vec3(value),
            ExtensionValueRef::Vec4(value) => Self::try_vec4(value),
            ExtensionValueRef::Color(value) => Self::try_color(value),
            ExtensionValueRef::Mat4(value) => Self::try_mat4(value),
            ExtensionValueRef::DenseArray(array) => match array {
                ExtensionArrayRef::Bool(values) => Self::try_bool_array(values),
                ExtensionArrayRef::I64(values) => Self::try_i64_array(values),
                ExtensionArrayRef::U64(values) => Self::try_u64_array(values),
                ExtensionArrayRef::F64(values) => Self::try_f64_array(values),
                ExtensionArrayRef::Vec2(values) => Self::try_vec2_array(values),
                ExtensionArrayRef::Vec3(values) => Self::try_vec3_array(values),
                ExtensionArrayRef::Vec4(values) => Self::try_vec4_array(values),
                ExtensionArrayRef::Color(values) => Self::try_color_array(values),
                ExtensionArrayRef::Mat4(values) => Self::try_mat4_array(values),
            },
        }
    }

    pub fn kind(&self) -> ExtensionValueKind {
        match &self.storage {
            ExtensionValueStorage::Bool(_) => ExtensionValueKind::Bool,
            ExtensionValueStorage::I64(_) => ExtensionValueKind::I64,
            ExtensionValueStorage::U64(_) => ExtensionValueKind::U64,
            ExtensionValueStorage::F64(_) => ExtensionValueKind::F64,
            ExtensionValueStorage::String(_) => ExtensionValueKind::String,
            ExtensionValueStorage::Bytes { .. } => ExtensionValueKind::Bytes,
            ExtensionValueStorage::Vec2(_) => ExtensionValueKind::Vec2,
            ExtensionValueStorage::Vec3(_) => ExtensionValueKind::Vec3,
            ExtensionValueStorage::Vec4(_) => ExtensionValueKind::Vec4,
            ExtensionValueStorage::Color(_) => ExtensionValueKind::Color,
            ExtensionValueStorage::Mat4(_) => ExtensionValueKind::Mat4,
            ExtensionValueStorage::DenseArray(array) => {
                ExtensionValueKind::DenseArray(array.element_type())
            }
        }
    }

    /// Returns the logical number of cells charged to array accounting.
    pub fn cell_count(&self) -> usize {
        match &self.storage {
            ExtensionValueStorage::DenseArray(array) => array.len(),
            _ => 1,
        }
    }

    /// Returns heap-backed payload bytes, excluding fixed-size enum storage.
    pub fn dynamic_payload_bytes(&self) -> usize {
        match &self.storage {
            ExtensionValueStorage::String(value) => value.len(),
            ExtensionValueStorage::Bytes { payload, .. } => payload.len(),
            ExtensionValueStorage::DenseArray(array) => array.payload_bytes(),
            _ => 0,
        }
    }

    /// Returns retained heap capacity for canonical allocation accounting.
    pub fn retained_allocation_bytes(&self) -> usize {
        match &self.storage {
            ExtensionValueStorage::String(value) => value.capacity(),
            ExtensionValueStorage::Bytes { payload, .. } => payload.capacity(),
            ExtensionValueStorage::DenseArray(array) => array.retained_allocation_bytes(),
            _ => 0,
        }
    }

    /// Borrows the closed validated value without exposing mutable storage.
    pub fn as_ref(&self) -> ExtensionValueRef<'_> {
        match &self.storage {
            ExtensionValueStorage::Bool(value) => ExtensionValueRef::Bool(*value),
            ExtensionValueStorage::I64(value) => ExtensionValueRef::I64(*value),
            ExtensionValueStorage::U64(value) => ExtensionValueRef::U64(*value),
            ExtensionValueStorage::F64(value) => ExtensionValueRef::F64(*value),
            ExtensionValueStorage::String(value) => ExtensionValueRef::String(value),
            ExtensionValueStorage::Bytes {
                media_type,
                payload,
            } => ExtensionValueRef::Bytes {
                media_type,
                payload,
            },
            ExtensionValueStorage::Vec2(value) => ExtensionValueRef::Vec2(*value),
            ExtensionValueStorage::Vec3(value) => ExtensionValueRef::Vec3(*value),
            ExtensionValueStorage::Vec4(value) => ExtensionValueRef::Vec4(*value),
            ExtensionValueStorage::Color(value) => ExtensionValueRef::Color(*value),
            ExtensionValueStorage::Mat4(value) => ExtensionValueRef::Mat4(*value),
            ExtensionValueStorage::DenseArray(array) => {
                ExtensionValueRef::DenseArray(array.as_ref())
            }
        }
    }
}

impl fmt::Debug for ExtensionValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionValue")
            .field("kind", &self.kind())
            .field("cells", &self.cell_count())
            .field("dynamic_payload_bytes", &self.dynamic_payload_bytes())
            .field(
                "retained_allocation_bytes",
                &self.retained_allocation_bytes(),
            )
            .finish()
    }
}

/// A borrowed view over one validated generic extension value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtensionValueRef<'a> {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(&'a str),
    Bytes {
        media_type: &'a ExtensionMediaType,
        payload: &'a [u8],
    },
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Color(Color),
    Mat4(Mat4),
    DenseArray(ExtensionArrayRef<'a>),
}

/// A borrowed typed view over one non-nested dense extension array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtensionArrayRef<'a> {
    Bool(&'a [bool]),
    I64(&'a [i64]),
    U64(&'a [u64]),
    F64(&'a [f64]),
    Vec2(&'a [Vec2]),
    Vec3(&'a [Vec3]),
    Vec4(&'a [Vec4]),
    Color(&'a [Color]),
    Mat4(&'a [Mat4]),
}

impl ExtensionArrayRef<'_> {
    pub fn element_type(self) -> ExtensionArrayElementType {
        match self {
            Self::Bool(_) => ExtensionArrayElementType::Bool,
            Self::I64(_) => ExtensionArrayElementType::I64,
            Self::U64(_) => ExtensionArrayElementType::U64,
            Self::F64(_) => ExtensionArrayElementType::F64,
            Self::Vec2(_) => ExtensionArrayElementType::Vec2,
            Self::Vec3(_) => ExtensionArrayElementType::Vec3,
            Self::Vec4(_) => ExtensionArrayElementType::Vec4,
            Self::Color(_) => ExtensionArrayElementType::Color,
            Self::Mat4(_) => ExtensionArrayElementType::Mat4,
        }
    }

    pub fn len(self) -> usize {
        match self {
            Self::Bool(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::Vec2(values) => values.len(),
            Self::Vec3(values) => values.len(),
            Self::Vec4(values) => values.len(),
            Self::Color(values) => values.len(),
            Self::Mat4(values) => values.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Why a generic extension value could not be constructed.
///
/// Errors report only categories, sizes, and offsets; rejected text and byte
/// payloads are never included.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionValueError {
    #[error("extension value {kind:?} contains a non-finite number")]
    NonFiniteScalar { kind: ExtensionValueKind },

    #[error("extension array {element_type:?} contains a non-finite number at index {index}")]
    NonFiniteArrayElement {
        element_type: ExtensionArrayElementType,
        index: usize,
    },

    #[error("extension string is {actual} bytes; the maximum is {maximum}")]
    StringTooLong { actual: usize, maximum: usize },

    #[error("extension byte payload is {actual} bytes; the maximum is {maximum}")]
    BytePayloadTooLong { actual: usize, maximum: usize },

    #[error("extension dense array has {actual} elements; the maximum is {maximum}")]
    ArrayTooLong { actual: usize, maximum: usize },

    #[error(transparent)]
    InvalidMediaType(#[from] ExtensionMediaTypeError),

    #[error("extension {resource:?} allocation failed")]
    AllocationFailed { resource: ExtensionAllocation },
}

/// Heap-backed extension resource whose fallible allocation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionAllocation {
    String,
    BytePayload,
    DenseArray,
}

fn require_finite(finite: bool, kind: ExtensionValueKind) -> Result<(), ExtensionValueError> {
    if finite {
        Ok(())
    } else {
        Err(ExtensionValueError::NonFiniteScalar { kind })
    }
}

fn color_is_finite(value: Color) -> bool {
    value.r.is_finite() && value.g.is_finite() && value.b.is_finite() && value.a.is_finite()
}

fn validate_finite_array<T>(
    values: &[T],
    element_type: ExtensionArrayElementType,
    finite: impl Fn(&T) -> bool,
) -> Result<(), ExtensionValueError> {
    validate_array_len(values.len())?;
    if let Some((index, _)) = values.iter().enumerate().find(|(_, value)| !finite(value)) {
        return Err(ExtensionValueError::NonFiniteArrayElement {
            element_type,
            index,
        });
    }
    Ok(())
}

fn validate_array_len(length: usize) -> Result<(), ExtensionValueError> {
    if length > MAX_EXTENSION_ARRAY_ELEMENTS {
        Err(ExtensionValueError::ArrayTooLong {
            actual: length,
            maximum: MAX_EXTENSION_ARRAY_ELEMENTS,
        })
    } else {
        Ok(())
    }
}

fn try_copy_array<T: Copy>(values: &[T]) -> Result<Vec<T>, ExtensionValueError> {
    validate_array_len(values.len())?;
    try_copy_slice(values, ExtensionAllocation::DenseArray)
}

fn try_copy_slice<T: Copy>(
    values: &[T],
    resource: ExtensionAllocation,
) -> Result<Vec<T>, ExtensionValueError> {
    let mut output = Vec::new();
    try_reserve_vec(&mut output, values.len(), resource)?;
    output.extend_from_slice(values);
    Ok(output)
}

fn try_reserve_vec<T>(
    values: &mut Vec<T>,
    additional: usize,
    resource: ExtensionAllocation,
) -> Result<(), ExtensionValueError> {
    let desired = values
        .len()
        .checked_add(additional)
        .ok_or(ExtensionValueError::AllocationFailed { resource })?;
    let plan = GrowableCapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    if desired > values.capacity() {
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    }
    plan.finish(values.len(), values.capacity())
        .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    Ok(())
}

fn try_reserve_string(
    value: &mut String,
    additional: usize,
    resource: ExtensionAllocation,
) -> Result<(), ExtensionValueError> {
    let desired = value
        .len()
        .checked_add(additional)
        .ok_or(ExtensionValueError::AllocationFailed { resource })?;
    let plan = GrowableCapacityPlan::for_type::<u8>(value.len(), value.capacity(), desired)
        .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    if desired > value.capacity() {
        value
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    }
    plan.finish(value.len(), value.capacity())
        .map_err(|_| ExtensionValueError::AllocationFailed { resource })?;
    Ok(())
}

/// Whether an entry is descriptive metadata or an unpromoted source fact.
///
/// Both classes remain non-behavioral. A source fact that changes rendering,
/// topology, deformation, or animation must use a typed descriptor backed by
/// an [`ExtensionSchemaId`], or be reported as unsupported behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionEntryClassification {
    DescriptiveMetadata,
    SourceExtension,
}

/// One provenance-bearing, owner-local extension entry.
///
/// Collection owners must call [`validate_unique_extension_entry_keys`] before
/// publication. The constructor requires a validated [`NamespacedName`], so a
/// bare or malformed key cannot enter this value.
#[derive(PartialEq)]
pub struct ExtensionEntry {
    key: NamespacedName,
    value: ExtensionValue,
    provenance: Provenance,
    classification: ExtensionEntryClassification,
}

impl ExtensionEntry {
    pub fn new(
        key: NamespacedName,
        value: ExtensionValue,
        provenance: Provenance,
        classification: ExtensionEntryClassification,
    ) -> Self {
        Self {
            key,
            value,
            provenance,
            classification,
        }
    }

    pub fn descriptive_metadata(
        key: NamespacedName,
        value: ExtensionValue,
        provenance: Provenance,
    ) -> Self {
        Self::new(
            key,
            value,
            provenance,
            ExtensionEntryClassification::DescriptiveMetadata,
        )
    }

    pub fn source_extension(
        key: NamespacedName,
        value: ExtensionValue,
        provenance: Provenance,
    ) -> Self {
        Self::new(
            key,
            value,
            provenance,
            ExtensionEntryClassification::SourceExtension,
        )
    }

    pub fn key(&self) -> &NamespacedName {
        &self.key
    }

    pub fn value(&self) -> &ExtensionValue {
        &self.value
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn classification(&self) -> ExtensionEntryClassification {
        self.classification
    }

    /// Fallibly copies this entry and its heap-backed value.
    pub fn try_clone(&self) -> Result<Self, ExtensionValueError> {
        Ok(Self {
            key: self.key.clone(),
            value: self.value.try_clone()?,
            provenance: self.provenance,
            classification: self.classification,
        })
    }
}

impl fmt::Debug for ExtensionEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionEntry")
            .field("key", &self.key)
            .field("value", &self.value)
            .field("provenance", &self.provenance)
            .field("classification", &self.classification)
            .finish()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionRetainedFootprintError {
    #[error("extension retained-footprint accounting overflowed")]
    Overflow,
}

pub(crate) enum ControlledRetainedFootprintError<E> {
    AccountingOverflow,
    Control(E),
}

pub(crate) fn extension_entries_retained_allocation_bytes(
    entries: &Vec<ExtensionEntry>,
) -> Result<usize, ExtensionRetainedFootprintError> {
    match extension_entries_retained_allocation_bytes_controlled(entries, &mut || {
        Ok::<(), Infallible>(())
    }) {
        Ok(bytes) => Ok(bytes),
        Err(ControlledRetainedFootprintError::AccountingOverflow) => {
            Err(ExtensionRetainedFootprintError::Overflow)
        }
        Err(ControlledRetainedFootprintError::Control(error)) => match error {},
    }
}

pub(crate) fn extension_entries_retained_allocation_bytes_controlled<E>(
    entries: &Vec<ExtensionEntry>,
    charge_work: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, ControlledRetainedFootprintError<E>> {
    let mut bytes = entries
        .capacity()
        .checked_mul(std::mem::size_of::<ExtensionEntry>())
        .ok_or(ControlledRetainedFootprintError::AccountingOverflow)?;
    for entry in entries {
        charge_work().map_err(ControlledRetainedFootprintError::Control)?;
        bytes = bytes
            .checked_add(entry.value().retained_allocation_bytes())
            .ok_or(ControlledRetainedFootprintError::AccountingOverflow)?;
    }
    Ok(bytes)
}

/// Deterministically validates owner-local extension-key uniqueness.
///
/// The first duplicate in input order is returned. Hash-table iteration order
/// is never observed, and every scratch-table growth uses `try_reserve`.
pub fn validate_unique_extension_entry_keys<'a>(
    entries: impl IntoIterator<Item = &'a ExtensionEntry>,
) -> Result<(), ExtensionEntryValidationError> {
    let mut first_indices: HashMap<&NamespacedName, usize> = HashMap::new();
    for (duplicate_index, entry) in entries.into_iter().enumerate() {
        if let Some(&first_index) = first_indices.get(entry.key()) {
            return Err(ExtensionEntryValidationError::DuplicateKey {
                first_index,
                duplicate_index,
            });
        }
        first_indices
            .try_reserve(1)
            .map_err(|_| ExtensionEntryValidationError::ScratchAllocationFailed)?;
        first_indices.insert(entry.key(), duplicate_index);
    }
    Ok(())
}

/// Why an owner-local extension collection failed validation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionEntryValidationError {
    #[error("extension key at index {duplicate_index} duplicates index {first_index}")]
    DuplicateKey {
        first_index: usize,
        duplicate_index: usize,
    },

    #[error("extension-key validation scratch allocation failed")]
    ScratchAllocationFailed,
}

/// Stable identity of one versioned declarative extension schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionSchemaId {
    name: NamespacedName,
    version: NonZeroU32,
}

impl ExtensionSchemaId {
    /// Validates a canonical namespaced schema name and positive version.
    pub fn new(name: impl AsRef<str>, version: u32) -> Result<Self, ExtensionSchemaIdError> {
        let name = NamespacedName::new(name)?;
        let version = NonZeroU32::new(version).ok_or(ExtensionSchemaIdError::ZeroVersion)?;
        Ok(Self { name, version })
    }

    /// Combines already validated identity components.
    pub const fn from_parts(name: NamespacedName, version: NonZeroU32) -> Self {
        Self { name, version }
    }

    pub const fn name(&self) -> &NamespacedName {
        &self.name
    }

    pub const fn version(&self) -> NonZeroU32 {
        self.version
    }
}

/// Why an extension schema identity was not canonical.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSchemaIdError {
    #[error(transparent)]
    InvalidName(#[from] NamespacedNameError),

    #[error("extension schema version must be positive")]
    ZeroVersion,
}

/// The canonical owner class of a behavior-bearing typed descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionSchemaOwner {
    Scene,
    SceneSpace,
    SourceIdentity,
    Entity(SceneEntityKind),
    Attribute,
    TextureBinding,
    AnimationChannel,
    PrimitiveMorphDelta,
}

/// Byte order fixed by a registered packed attribute schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackedByteOrder {
    LittleEndian,
    BigEndian,
}

/// Core-executable validation algorithms admitted for packed attributes.
///
/// The enum is deliberately closed. A schema cannot smuggle an arbitrary
/// callback or an external codec identity into the canonical scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackedAttributeElementValidator {
    /// Two signed-normalized 16-bit octahedral coordinates. The non-canonical
    /// `i16::MIN` representation is rejected for both lanes.
    CanonicalOctahedralSnorm16,
}

/// Core-executable typed decoders admitted for packed attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackedAttributeTypedDecoder {
    /// Decodes two octahedral `i16` lanes into one finite unit `Vec3`.
    UnitVector3OctahedralSnorm16,
}

/// The closed logical result of a packed attribute typed decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackedAttributeDecodedKind {
    F32Vec3,
}

/// Pure-data executable contract for one registered packed attribute encoding.
///
/// Every field participates in schema equality and scene snapshots. The only
/// accepted combinations are implemented by core, so validator and decoder
/// behavior remains available after importer or registry teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackedAttributeEncodingSchema {
    element_width: NonZeroU16,
    logical_lanes: NonZeroU8,
    byte_order: PackedByteOrder,
    validator: PackedAttributeElementValidator,
    decoder: PackedAttributeTypedDecoder,
}

impl PackedAttributeEncodingSchema {
    /// Builds one core-understood packed contract and rejects mismatched bodies.
    pub fn new(
        element_width: u16,
        logical_lanes: u8,
        byte_order: PackedByteOrder,
        validator: PackedAttributeElementValidator,
        decoder: PackedAttributeTypedDecoder,
    ) -> Result<Self, PackedAttributeEncodingSchemaError> {
        let element_width = NonZeroU16::new(element_width)
            .ok_or(PackedAttributeEncodingSchemaError::ZeroElementWidth)?;
        let logical_lanes = NonZeroU8::new(logical_lanes)
            .filter(|lanes| lanes.get() <= 16)
            .ok_or(
                PackedAttributeEncodingSchemaError::InvalidLogicalLaneCount {
                    lanes: logical_lanes,
                },
            )?;
        let contract = Self {
            element_width,
            logical_lanes,
            byte_order,
            validator,
            decoder,
        };
        if contract.is_core_understood() {
            Ok(contract)
        } else {
            Err(PackedAttributeEncodingSchemaError::UnsupportedContract)
        }
    }

    /// The currently produced core encoding for finite unit-vector attributes.
    pub fn octahedral_unit_vector_snorm16(
        byte_order: PackedByteOrder,
    ) -> Result<Self, PackedAttributeEncodingSchemaError> {
        Self::new(
            4,
            3,
            byte_order,
            PackedAttributeElementValidator::CanonicalOctahedralSnorm16,
            PackedAttributeTypedDecoder::UnitVector3OctahedralSnorm16,
        )
    }

    const fn is_core_understood(self) -> bool {
        self.element_width.get() == 4
            && self.logical_lanes.get() == 3
            && matches!(
                (self.validator, self.decoder),
                (
                    PackedAttributeElementValidator::CanonicalOctahedralSnorm16,
                    PackedAttributeTypedDecoder::UnitVector3OctahedralSnorm16
                )
            )
    }

    pub const fn element_width(self) -> u16 {
        self.element_width.get()
    }

    pub const fn logical_lanes(self) -> u8 {
        self.logical_lanes.get()
    }

    pub const fn byte_order(self) -> PackedByteOrder {
        self.byte_order
    }

    pub const fn validator(self) -> PackedAttributeElementValidator {
        self.validator
    }

    pub const fn decoder(self) -> PackedAttributeTypedDecoder {
        self.decoder
    }

    pub const fn decoded_kind(self) -> PackedAttributeDecodedKind {
        match self.decoder {
            PackedAttributeTypedDecoder::UnitVector3OctahedralSnorm16 => {
                PackedAttributeDecodedKind::F32Vec3
            }
        }
    }

    /// Executes the schema-declared validator over a complete packed payload.
    pub fn validate_payload(self, bytes: &[u8]) -> Result<u32, PackedAttributeValidationError> {
        if bytes.len() > MAX_PACKED_ATTRIBUTE_PAYLOAD_BYTES {
            return Err(PackedAttributeValidationError::PayloadTooLarge {
                actual: bytes.len(),
                maximum: MAX_PACKED_ATTRIBUTE_PAYLOAD_BYTES,
            });
        }
        let element_width = usize::from(self.element_width.get());
        if !bytes.len().is_multiple_of(element_width) {
            return Err(PackedAttributeValidationError::ByteCountNotDivisible {
                byte_count: bytes.len(),
                element_width: self.element_width.get(),
            });
        }
        let element_count = bytes.len() / element_width;
        if element_count > MAX_PACKED_ATTRIBUTE_ELEMENTS {
            return Err(PackedAttributeValidationError::ElementCountTooLarge {
                actual: element_count,
                maximum: MAX_PACKED_ATTRIBUTE_ELEMENTS,
            });
        }
        let element_count = u32::try_from(element_count).map_err(|_| {
            PackedAttributeValidationError::ElementCountTooLarge {
                actual: element_count,
                maximum: MAX_PACKED_ATTRIBUTE_ELEMENTS,
            }
        })?;

        for (element_index, element) in bytes.chunks_exact(element_width).enumerate() {
            self.validate_element(element, element_index)?;
        }
        Ok(element_count)
    }

    /// Executes the schema-declared typed decoder for one element.
    pub fn decode_vec3_element(self, bytes: &[u8]) -> Result<Vec3, PackedAttributeValidationError> {
        self.validate_element(bytes, 0)?;
        Ok(self.decode_vec3_body(bytes))
    }

    pub(crate) fn decode_validated_vec3_element(
        self,
        bytes: &[u8],
    ) -> Result<Vec3, PackedAttributeValidationError> {
        if bytes.len() != usize::from(self.element_width.get()) {
            return Err(PackedAttributeValidationError::ElementWidthMismatch {
                actual: bytes.len(),
                expected: self.element_width.get(),
            });
        }
        Ok(self.decode_vec3_body(bytes))
    }

    fn decode_vec3_body(self, bytes: &[u8]) -> Vec3 {
        match self.decoder {
            PackedAttributeTypedDecoder::UnitVector3OctahedralSnorm16 => {
                let x = f32::from(read_packed_i16(self.byte_order, &bytes[..2])) / 32_767.0;
                let y = f32::from(read_packed_i16(self.byte_order, &bytes[2..])) / 32_767.0;
                decode_octahedral_unit_vector(x, y)
            }
        }
    }

    fn validate_element(
        self,
        bytes: &[u8],
        element_index: usize,
    ) -> Result<(), PackedAttributeValidationError> {
        if bytes.len() != usize::from(self.element_width.get()) {
            return Err(PackedAttributeValidationError::ElementWidthMismatch {
                actual: bytes.len(),
                expected: self.element_width.get(),
            });
        }
        match self.validator {
            PackedAttributeElementValidator::CanonicalOctahedralSnorm16 => {
                for (lane, lane_bytes) in bytes.chunks_exact(2).enumerate() {
                    if read_packed_i16(self.byte_order, lane_bytes) == i16::MIN {
                        return Err(PackedAttributeValidationError::ReservedBitPattern {
                            element_index,
                            lane,
                        });
                    }
                }
                Ok(())
            }
        }
    }
}

/// Why a declarative packed attribute contract was not core-executable.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PackedAttributeEncodingSchemaError {
    #[error("packed attribute element width must be positive")]
    ZeroElementWidth,

    #[error("packed attribute logical lane count {lanes} is outside 1..=16")]
    InvalidLogicalLaneCount { lanes: u8 },

    #[error("packed attribute validator and typed decoder contract is not implemented by core")]
    UnsupportedContract,
}

/// Why bytes failed one core-executable packed attribute schema.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PackedAttributeValidationError {
    #[error("packed attribute payload is {actual} bytes; the maximum is {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },

    #[error(
        "packed attribute byte count {byte_count} is not divisible by element width {element_width}"
    )]
    ByteCountNotDivisible {
        byte_count: usize,
        element_width: u16,
    },

    #[error("packed attribute element count {actual} exceeds the maximum {maximum}")]
    ElementCountTooLarge { actual: usize, maximum: usize },

    #[error("packed attribute element has width {actual}; expected {expected}")]
    ElementWidthMismatch { actual: usize, expected: u16 },

    #[error("packed attribute element {element_index} lane {lane} uses a reserved bit pattern")]
    ReservedBitPattern { element_index: usize, lane: usize },
}

fn read_packed_i16(byte_order: PackedByteOrder, bytes: &[u8]) -> i16 {
    let pair = [bytes[0], bytes[1]];
    match byte_order {
        PackedByteOrder::LittleEndian => i16::from_le_bytes(pair),
        PackedByteOrder::BigEndian => i16::from_be_bytes(pair),
    }
}

fn decode_octahedral_unit_vector(mut x: f32, mut y: f32) -> Vec3 {
    let z = 1.0 - x.abs() - y.abs();
    if z < 0.0 {
        let previous_x = x;
        x = (1.0 - y.abs()) * sign_not_zero(previous_x);
        y = (1.0 - previous_x.abs()) * sign_not_zero(y);
    }
    let inverse_length = (x.mul_add(x, y.mul_add(y, z * z))).sqrt().recip();
    Vec3::new(x * inverse_length, y * inverse_length, z * inverse_length)
}

const fn sign_not_zero(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

/// Optional closed behavior carried by one declarative extension schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionSchemaBehavior {
    None,
    Canonical(ExtensionBehaviorKind),
    PackedAttribute(PackedAttributeEncodingSchema),
}

/// The exact core-owned use authorized by a behavior schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionBehaviorKind {
    SourceObjectLocator,
    SceneSpaceTangentBasis,
    SceneSpaceTangentHandednessEncoding,
    MaterialShadingModel,
    MaterialAlphaMode,
    TextureRole,
    TextureCoordinateSource,
    TextureChannelMapping,
    TextureColorInterpretation,
    TextureBlendOperation,
    AttributeSignedNormalization,
    AttributeUnsignedNormalization,
    AnimationTimeDomain,
    AnimationExtrapolation,
}

impl ExtensionBehaviorKind {
    pub const fn expected_owner(self) -> ExtensionSchemaOwner {
        match self {
            Self::SourceObjectLocator => ExtensionSchemaOwner::SourceIdentity,
            Self::SceneSpaceTangentBasis | Self::SceneSpaceTangentHandednessEncoding => {
                ExtensionSchemaOwner::SceneSpace
            }
            Self::MaterialShadingModel | Self::MaterialAlphaMode => {
                ExtensionSchemaOwner::Entity(SceneEntityKind::Material)
            }
            Self::TextureRole
            | Self::TextureCoordinateSource
            | Self::TextureChannelMapping
            | Self::TextureColorInterpretation
            | Self::TextureBlendOperation => ExtensionSchemaOwner::TextureBinding,
            Self::AttributeSignedNormalization | Self::AttributeUnsignedNormalization => {
                ExtensionSchemaOwner::Attribute
            }
            Self::AnimationTimeDomain => {
                ExtensionSchemaOwner::Entity(SceneEntityKind::AnimationClip)
            }
            Self::AnimationExtrapolation => ExtensionSchemaOwner::AnimationChannel,
        }
    }
}

/// The closed value shape admitted by one declarative schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionValueSchema {
    Bool,
    I64,
    U64,
    F64,
    String {
        max_bytes: u32,
    },
    Bytes {
        max_bytes: u32,
        media_type: Option<ExtensionMediaType>,
    },
    Vec2,
    Vec3,
    Vec4,
    Color,
    Mat4,
    DenseArray {
        element_type: ExtensionArrayElementType,
        max_elements: u32,
    },
}

impl ExtensionValueSchema {
    pub fn kind(&self) -> ExtensionValueKind {
        match self {
            Self::Bool => ExtensionValueKind::Bool,
            Self::I64 => ExtensionValueKind::I64,
            Self::U64 => ExtensionValueKind::U64,
            Self::F64 => ExtensionValueKind::F64,
            Self::String { .. } => ExtensionValueKind::String,
            Self::Bytes { .. } => ExtensionValueKind::Bytes,
            Self::Vec2 => ExtensionValueKind::Vec2,
            Self::Vec3 => ExtensionValueKind::Vec3,
            Self::Vec4 => ExtensionValueKind::Vec4,
            Self::Color => ExtensionValueKind::Color,
            Self::Mat4 => ExtensionValueKind::Mat4,
            Self::DenseArray { element_type, .. } => ExtensionValueKind::DenseArray(*element_type),
        }
    }
}

/// Which canonical collection controls a descriptor's logical cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionCardinality {
    ExactlyOne,
    OptionalOne,
    PerEntity(SceneEntityKind),
    PerAttributeDomain(AttributeDomain),
}

impl ExtensionCardinality {
    const fn requires_remap(self) -> bool {
        matches!(self, Self::PerEntity(_) | Self::PerAttributeDomain(_))
    }
}

/// Canonical-reference qualification declared by a typed descriptor schema.
///
/// Generic [`ExtensionValue`] has no reference-bearing variant; this rule is
/// for a typed descriptor's separate canonical reference field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionReferenceRule {
    None,
    SameGeneration(SceneEntityKind),
    SameGenerationAndLogicalMeshOwner(SceneEntityKind),
}

impl ExtensionReferenceRule {
    const fn has_reference(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Declarative obligation applied when an edit changes indexed authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionRemapRule {
    Preserve,
    RemapCardinality,
    RemapCanonicalReference,
    RemapCardinalityAndReference,
    RejectAffectedEdit,
}

/// Declarative invalidation trigger for descriptor-derived data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionInvalidationRule {
    Never,
    OnOwnerChange,
    OnReferencedEntityChange,
    OnOwnerOrReferencedEntityChange,
    OnAnyStructuralEdit,
}

/// One validated callback-free declarative extension schema.
///
/// Every field is core-owned data. The type contains no registry, importer,
/// codec, trait object, native handle, or process-local type identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtensionSchema {
    id: ExtensionSchemaId,
    owner: ExtensionSchemaOwner,
    value: ExtensionValueSchema,
    behavior: ExtensionSchemaBehavior,
    cardinality: ExtensionCardinality,
    reference: ExtensionReferenceRule,
    remap: ExtensionRemapRule,
    invalidation: ExtensionInvalidationRule,
}

impl ExtensionSchema {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ExtensionSchemaId,
        owner: ExtensionSchemaOwner,
        value: ExtensionValueSchema,
        cardinality: ExtensionCardinality,
        reference: ExtensionReferenceRule,
        remap: ExtensionRemapRule,
        invalidation: ExtensionInvalidationRule,
    ) -> Result<Self, ExtensionSchemaError> {
        Self::new_with_behavior(
            id,
            owner,
            value,
            ExtensionSchemaBehavior::None,
            cardinality,
            reference,
            remap,
            invalidation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_behavior(
        id: ExtensionSchemaId,
        owner: ExtensionSchemaOwner,
        value: ExtensionValueSchema,
        behavior: ExtensionSchemaBehavior,
        cardinality: ExtensionCardinality,
        reference: ExtensionReferenceRule,
        remap: ExtensionRemapRule,
        invalidation: ExtensionInvalidationRule,
    ) -> Result<Self, ExtensionSchemaError> {
        validate_schema_value_limits(&value)?;
        validate_schema_behavior(
            owner,
            &value,
            behavior,
            cardinality,
            reference,
            remap,
            invalidation,
        )?;
        validate_reference_rule(reference)?;
        validate_remap_rule(cardinality, reference, remap)?;
        validate_invalidation_rule(reference, invalidation)?;
        Ok(Self {
            id,
            owner,
            value,
            behavior,
            cardinality,
            reference,
            remap,
            invalidation,
        })
    }

    pub const fn id(&self) -> &ExtensionSchemaId {
        &self.id
    }

    pub const fn owner(&self) -> ExtensionSchemaOwner {
        self.owner
    }

    pub const fn value(&self) -> &ExtensionValueSchema {
        &self.value
    }

    pub const fn behavior(&self) -> ExtensionSchemaBehavior {
        self.behavior
    }

    pub const fn packed_attribute_encoding(&self) -> Option<PackedAttributeEncodingSchema> {
        match self.behavior {
            ExtensionSchemaBehavior::PackedAttribute(encoding) => Some(encoding),
            ExtensionSchemaBehavior::None | ExtensionSchemaBehavior::Canonical(_) => None,
        }
    }

    pub const fn cardinality(&self) -> ExtensionCardinality {
        self.cardinality
    }

    pub const fn reference(&self) -> ExtensionReferenceRule {
        self.reference
    }

    pub const fn remap(&self) -> ExtensionRemapRule {
        self.remap
    }

    pub const fn invalidation(&self) -> ExtensionInvalidationRule {
        self.invalidation
    }

    /// Checks one already core-valid value against this schema's tighter shape
    /// and size constraints without allocating.
    pub fn validate_value(&self, value: &ExtensionValue) -> Result<(), ExtensionSchemaValueError> {
        let expected = self.value.kind();
        let actual = value.kind();
        if expected != actual {
            return Err(ExtensionSchemaValueError::KindMismatch { expected, actual });
        }

        match (&self.value, value.as_ref()) {
            (ExtensionValueSchema::String { max_bytes }, ExtensionValueRef::String(value)) => {
                validate_schema_value_size(
                    ExtensionSchemaValueLimit::StringBytes,
                    value.len(),
                    *max_bytes,
                )
            }
            (
                ExtensionValueSchema::Bytes {
                    max_bytes,
                    media_type,
                },
                ExtensionValueRef::Bytes {
                    media_type: actual_media_type,
                    payload,
                },
            ) => {
                validate_schema_value_size(
                    ExtensionSchemaValueLimit::BytePayloadBytes,
                    payload.len(),
                    *max_bytes,
                )?;
                if media_type
                    .as_ref()
                    .is_some_and(|expected_media_type| expected_media_type != actual_media_type)
                {
                    return Err(ExtensionSchemaValueError::MediaTypeMismatch);
                }
                Ok(())
            }
            (
                ExtensionValueSchema::DenseArray { max_elements, .. },
                ExtensionValueRef::DenseArray(values),
            ) => validate_schema_value_size(
                ExtensionSchemaValueLimit::DenseArrayElements,
                values.len(),
                *max_elements,
            ),
            _ => Ok(()),
        }
    }
}

/// A declarative schema definition was internally inconsistent.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSchemaError {
    #[error("extension schema {limit:?} limit {actual} exceeds core maximum {maximum}")]
    LimitExceedsCore {
        limit: ExtensionSchemaLimit,
        actual: usize,
        maximum: usize,
    },

    #[error("owner-qualified extension reference kind {kind:?} has no owner-qualified handle")]
    InvalidOwnerQualifiedReference { kind: SceneEntityKind },

    #[error("extension remap rule is inconsistent with its cardinality/reference declaration")]
    InconsistentRemapRule,

    #[error("extension invalidation rule requires a canonical reference")]
    InconsistentInvalidationRule,

    #[error("packed attribute behavior requires an attribute-owned schema")]
    PackedAttributeOwnerRequired,

    #[error("packed attribute behavior does not match the schema's logical value shape")]
    PackedAttributeValueShapeMismatch,

    #[error("extension behavior {behavior:?} requires owner {expected:?}, found {actual:?}")]
    BehaviorOwnerMismatch {
        behavior: ExtensionBehaviorKind,
        expected: ExtensionSchemaOwner,
        actual: ExtensionSchemaOwner,
    },

    #[error("source-object locator schema does not match the closed canonical contract")]
    SourceObjectLocatorContractMismatch,
}

/// A core hard limit that a declarative schema may only tighten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionSchemaLimit {
    StringBytes,
    BytePayloadBytes,
    DenseArrayElements,
}

fn validate_schema_value_limits(value: &ExtensionValueSchema) -> Result<(), ExtensionSchemaError> {
    match value {
        ExtensionValueSchema::String { max_bytes } => validate_schema_limit(
            ExtensionSchemaLimit::StringBytes,
            *max_bytes,
            MAX_EXTENSION_STRING_BYTES,
        ),
        ExtensionValueSchema::Bytes { max_bytes, .. } => validate_schema_limit(
            ExtensionSchemaLimit::BytePayloadBytes,
            *max_bytes,
            MAX_EXTENSION_BYTE_PAYLOAD_BYTES,
        ),
        ExtensionValueSchema::DenseArray { max_elements, .. } => validate_schema_limit(
            ExtensionSchemaLimit::DenseArrayElements,
            *max_elements,
            MAX_EXTENSION_ARRAY_ELEMENTS,
        ),
        _ => Ok(()),
    }
}

fn validate_schema_behavior(
    owner: ExtensionSchemaOwner,
    value: &ExtensionValueSchema,
    behavior: ExtensionSchemaBehavior,
    cardinality: ExtensionCardinality,
    reference: ExtensionReferenceRule,
    remap: ExtensionRemapRule,
    invalidation: ExtensionInvalidationRule,
) -> Result<(), ExtensionSchemaError> {
    match behavior {
        ExtensionSchemaBehavior::None => Ok(()),
        ExtensionSchemaBehavior::Canonical(behavior) => {
            let expected = behavior.expected_owner();
            if owner != expected {
                return Err(ExtensionSchemaError::BehaviorOwnerMismatch {
                    behavior,
                    expected,
                    actual: owner,
                });
            }
            if behavior == ExtensionBehaviorKind::SourceObjectLocator
                && !matches!(
                    value,
                    ExtensionValueSchema::String { max_bytes }
                        if usize::try_from(*max_bytes).is_ok_and(|maximum| maximum <= MAX_SOURCE_LOCATOR_LEN)
                )
            {
                return Err(ExtensionSchemaError::SourceObjectLocatorContractMismatch);
            }
            if behavior == ExtensionBehaviorKind::SourceObjectLocator
                && (cardinality != ExtensionCardinality::ExactlyOne
                    || reference != ExtensionReferenceRule::None
                    || remap != ExtensionRemapRule::Preserve
                    || invalidation != ExtensionInvalidationRule::Never)
            {
                return Err(ExtensionSchemaError::SourceObjectLocatorContractMismatch);
            }
            Ok(())
        }
        ExtensionSchemaBehavior::PackedAttribute(packed) => {
            if owner != ExtensionSchemaOwner::Attribute {
                return Err(ExtensionSchemaError::PackedAttributeOwnerRequired);
            }
            let value_shape_matches = match packed.decoded_kind() {
                PackedAttributeDecodedKind::F32Vec3 => {
                    matches!(value, ExtensionValueSchema::Vec3)
                }
            };
            if value_shape_matches {
                Ok(())
            } else {
                Err(ExtensionSchemaError::PackedAttributeValueShapeMismatch)
            }
        }
    }
}

fn validate_schema_limit(
    limit: ExtensionSchemaLimit,
    actual: u32,
    maximum: usize,
) -> Result<(), ExtensionSchemaError> {
    let actual = usize::try_from(actual).map_err(|_| ExtensionSchemaError::LimitExceedsCore {
        limit,
        actual: usize::MAX,
        maximum,
    })?;
    if actual > maximum {
        Err(ExtensionSchemaError::LimitExceedsCore {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn validate_reference_rule(reference: ExtensionReferenceRule) -> Result<(), ExtensionSchemaError> {
    if let ExtensionReferenceRule::SameGenerationAndLogicalMeshOwner(kind) = reference
        && !matches!(
            kind,
            SceneEntityKind::Primitive | SceneEntityKind::MorphTarget
        )
    {
        return Err(ExtensionSchemaError::InvalidOwnerQualifiedReference { kind });
    }
    Ok(())
}

fn validate_remap_rule(
    cardinality: ExtensionCardinality,
    reference: ExtensionReferenceRule,
    remap: ExtensionRemapRule,
) -> Result<(), ExtensionSchemaError> {
    let cardinality_remap = cardinality.requires_remap();
    let reference_remap = reference.has_reference();
    let consistent = match remap {
        ExtensionRemapRule::Preserve => !cardinality_remap && !reference_remap,
        ExtensionRemapRule::RemapCardinality => cardinality_remap && !reference_remap,
        ExtensionRemapRule::RemapCanonicalReference => !cardinality_remap && reference_remap,
        ExtensionRemapRule::RemapCardinalityAndReference => cardinality_remap && reference_remap,
        ExtensionRemapRule::RejectAffectedEdit => true,
    };
    if consistent {
        Ok(())
    } else {
        Err(ExtensionSchemaError::InconsistentRemapRule)
    }
}

fn validate_invalidation_rule(
    reference: ExtensionReferenceRule,
    invalidation: ExtensionInvalidationRule,
) -> Result<(), ExtensionSchemaError> {
    let requires_reference = matches!(
        invalidation,
        ExtensionInvalidationRule::OnReferencedEntityChange
            | ExtensionInvalidationRule::OnOwnerOrReferencedEntityChange
    );
    if requires_reference && !reference.has_reference() {
        Err(ExtensionSchemaError::InconsistentInvalidationRule)
    } else {
        Ok(())
    }
}

fn validate_schema_value_size(
    limit: ExtensionSchemaValueLimit,
    actual: usize,
    maximum: u32,
) -> Result<(), ExtensionSchemaValueError> {
    let maximum =
        usize::try_from(maximum).map_err(|_| ExtensionSchemaValueError::LimitExceeded {
            limit,
            actual,
            maximum: usize::MAX,
        })?;
    if actual > maximum {
        Err(ExtensionSchemaValueError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

/// Why a core-valid value did not satisfy one declarative schema.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSchemaValueError {
    #[error("extension schema expects {expected:?}, found {actual:?}")]
    KindMismatch {
        expected: ExtensionValueKind,
        actual: ExtensionValueKind,
    },

    #[error("extension value {limit:?} is {actual}; the schema maximum is {maximum}")]
    LimitExceeded {
        limit: ExtensionSchemaValueLimit,
        actual: usize,
        maximum: usize,
    },

    #[error("extension byte payload media type does not match its schema")]
    MediaTypeMismatch,
}

/// Schema-local limit checked against one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionSchemaValueLimit {
    StringBytes,
    BytePayloadBytes,
    DenseArrayElements,
}

/// Builder for an immutable canonically ordered schema snapshot.
#[derive(Debug, Default)]
pub struct ExtensionSchemaSnapshotBuilder {
    schemas: Vec<ExtensionSchema>,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_SCHEMA_CAPACITY_QUALIFICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_schema_capacity_qualification_for_test() {
    FAIL_NEXT_SCHEMA_CAPACITY_QUALIFICATION.with(|flag| flag.set(true));
}

#[cfg(test)]
fn schema_capacity_for_qualification(actual: usize, envelope: usize) -> usize {
    if FAIL_NEXT_SCHEMA_CAPACITY_QUALIFICATION.with(|flag| flag.replace(false)) {
        envelope.saturating_add(1)
    } else {
        actual
    }
}

#[cfg(not(test))]
fn schema_capacity_for_qualification(actual: usize, _envelope: usize) -> usize {
    actual
}

impl ExtensionSchemaSnapshotBuilder {
    pub const fn new() -> Self {
        Self {
            schemas: Vec::new(),
        }
    }

    /// Inserts one schema while maintaining canonical ID/version order.
    ///
    /// Equal IDs are rejected even when their bodies are equal. A distinct
    /// error identifies equal IDs with conflicting bodies.
    pub fn try_insert(
        &mut self,
        schema: ExtensionSchema,
    ) -> Result<(), ExtensionSchemaSnapshotError> {
        match self
            .schemas
            .binary_search_by(|existing| existing.id().cmp(schema.id()))
        {
            Ok(existing_index) => {
                if self.schemas[existing_index] == schema {
                    Err(ExtensionSchemaSnapshotError::DuplicateSchemaId)
                } else {
                    Err(ExtensionSchemaSnapshotError::ConflictingSchemaBody)
                }
            }
            Err(insertion_index) => {
                if self.schemas.len() == MAX_EXTENSION_SCHEMAS {
                    return Err(ExtensionSchemaSnapshotError::SchemaLimitExceeded {
                        maximum: MAX_EXTENSION_SCHEMAS,
                    });
                }
                try_insert_schema(&mut self.schemas, insertion_index, schema)?;
                Ok(())
            }
        }
    }

    pub fn build(self) -> ExtensionSchemaSnapshot {
        ExtensionSchemaSnapshot {
            schemas: self.schemas,
        }
    }
}

/// Immutable, core-owned declarative schemas retained by one scene.
///
/// The snapshot owns every byte and contains no external component reference,
/// so registry/importer teardown cannot change lookup or schema meaning.
pub struct ExtensionSchemaSnapshot {
    schemas: Vec<ExtensionSchema>,
}

impl ExtensionSchemaSnapshot {
    pub const fn empty() -> Self {
        Self {
            schemas: Vec::new(),
        }
    }

    pub fn try_from_schemas(
        schemas: impl IntoIterator<Item = ExtensionSchema>,
    ) -> Result<Self, ExtensionSchemaSnapshotError> {
        let mut source = schemas.into_iter();
        let (lower, _) = source.size_hint();
        if lower > MAX_EXTENSION_SCHEMAS {
            return Err(ExtensionSchemaSnapshotError::SchemaLimitExceeded {
                maximum: MAX_EXTENSION_SCHEMAS,
            });
        }

        let mut schemas = Vec::new();
        if lower > 0 {
            try_reserve_schema_vec(&mut schemas, lower)?;
        }
        for schema in source.by_ref() {
            if schemas.len() == MAX_EXTENSION_SCHEMAS {
                return Err(ExtensionSchemaSnapshotError::SchemaLimitExceeded {
                    maximum: MAX_EXTENSION_SCHEMAS,
                });
            }
            if schemas.len() == schemas.capacity() {
                let desired = schemas
                    .capacity()
                    .checked_mul(2)
                    .unwrap_or(MAX_EXTENSION_SCHEMAS)
                    .clamp(1, MAX_EXTENSION_SCHEMAS);
                let additional = desired - schemas.len();
                try_reserve_schema_vec(&mut schemas, additional)?;
            }
            schemas.push(schema);
        }

        schemas.sort_unstable_by(|left, right| left.id().cmp(right.id()));
        validate_sorted_schema_ids(&schemas)?;
        Ok(Self { schemas })
    }

    /// Fallibly duplicates the immutable owned schema data.
    pub fn try_clone(&self) -> Result<Self, ExtensionSchemaSnapshotError> {
        let mut schemas = Vec::new();
        try_reserve_schema_vec(&mut schemas, self.schemas.len())?;
        schemas.extend(self.schemas.iter().cloned());
        Ok(Self { schemas })
    }

    pub fn get(&self, id: &ExtensionSchemaId) -> Option<&ExtensionSchema> {
        self.schemas
            .binary_search_by(|schema| schema.id().cmp(id))
            .ok()
            .and_then(|index| self.schemas.get(index))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ExtensionSchema> {
        self.schemas.iter()
    }

    pub fn len(&self) -> usize {
        self.schemas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }

    /// Returns retained vector capacity for canonical allocation accounting.
    pub fn retained_allocation_bytes(&self) -> usize {
        self.schemas
            .capacity()
            .saturating_mul(std::mem::size_of::<ExtensionSchema>())
    }
}

impl Default for ExtensionSchemaSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Debug for ExtensionSchemaSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionSchemaSnapshot")
            .field("schemas", &self.schemas)
            .finish()
    }
}

impl PartialEq for ExtensionSchemaSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.schemas == other.schemas
    }
}

impl Eq for ExtensionSchemaSnapshot {}

/// Why a declarative schema snapshot could not be built or copied.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSchemaSnapshotError {
    #[error("extension schema ID is duplicated")]
    DuplicateSchemaId,

    #[error("extension schema ID has a conflicting body")]
    ConflictingSchemaBody,

    #[error("extension schema count exceeds {maximum}")]
    SchemaLimitExceeded { maximum: usize },

    #[error("extension schema snapshot allocation failed")]
    AllocationFailed,
}

fn try_insert_schema(
    schemas: &mut Vec<ExtensionSchema>,
    insertion_index: usize,
    schema: ExtensionSchema,
) -> Result<(), ExtensionSchemaSnapshotError> {
    let desired = schemas
        .len()
        .checked_add(1)
        .ok_or(ExtensionSchemaSnapshotError::AllocationFailed)?;
    let plan = GrowableCapacityPlan::for_type::<ExtensionSchema>(
        schemas.len(),
        schemas.capacity(),
        desired,
    )
    .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    plan.verify_source(schemas.len(), schemas.capacity())
        .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;

    if desired <= schemas.capacity() {
        schemas.insert(insertion_index, schema);
        return Ok(());
    }

    let replacement_plan =
        CapacityPlan::for_type::<ExtensionSchema>(0, 0, plan.envelope_capacity())
            .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(replacement_plan.allocator_additional())
        .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    let qualified_capacity = schema_capacity_for_qualification(
        replacement.capacity(),
        replacement_plan.envelope_capacity(),
    );
    replacement_plan
        .finish(replacement.len(), qualified_capacity)
        .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;

    replacement.append(schemas);
    replacement.insert(insertion_index, schema);
    *schemas = replacement;
    Ok(())
}

fn validate_sorted_schema_ids(
    schemas: &[ExtensionSchema],
) -> Result<(), ExtensionSchemaSnapshotError> {
    let mut group_start = 0usize;
    while group_start < schemas.len() {
        let mut group_end = group_start + 1;
        while group_end < schemas.len() && schemas[group_end].id() == schemas[group_start].id() {
            group_end += 1;
        }
        if group_end - group_start > 1 {
            let first = &schemas[group_start];
            if schemas[group_start + 1..group_end]
                .iter()
                .any(|schema| schema != first)
            {
                return Err(ExtensionSchemaSnapshotError::ConflictingSchemaBody);
            }
            return Err(ExtensionSchemaSnapshotError::DuplicateSchemaId);
        }
        group_start = group_end;
    }
    Ok(())
}

fn try_reserve_schema_vec(
    schemas: &mut Vec<ExtensionSchema>,
    additional: usize,
) -> Result<(), ExtensionSchemaSnapshotError> {
    let desired = schemas
        .len()
        .checked_add(additional)
        .ok_or(ExtensionSchemaSnapshotError::AllocationFailed)?;
    let plan = GrowableCapacityPlan::for_type::<ExtensionSchema>(
        schemas.len(),
        schemas.capacity(),
        desired,
    )
    .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    if desired > schemas.capacity() {
        schemas
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    }
    plan.finish(schemas.len(), schemas.capacity())
        .map_err(|_| ExtensionSchemaSnapshotError::AllocationFailed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::test_provenance_fixture;
    use std::cell::Cell;
    use std::rc::Rc;

    fn plain_schema(name: &str, value: ExtensionValueSchema) -> ExtensionSchema {
        ExtensionSchema::new(
            ExtensionSchemaId::new(name, 1).unwrap(),
            ExtensionSchemaOwner::Scene,
            value,
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        )
        .unwrap()
    }

    fn packed_attribute_schema(name: &str, byte_order: PackedByteOrder) -> ExtensionSchema {
        ExtensionSchema::new_with_behavior(
            ExtensionSchemaId::new(name, 1).unwrap(),
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::Vec3,
            ExtensionSchemaBehavior::PackedAttribute(
                PackedAttributeEncodingSchema::octahedral_unit_vector_snorm16(byte_order).unwrap(),
            ),
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        )
        .unwrap()
    }

    #[test]
    fn shared_namespaced_grammar_rejects_bare_and_malformed_schema_names() {
        for value in ["bare", "Owner:name", "owner:", "owner::name"] {
            assert!(matches!(
                ExtensionSchemaId::new(value, 1),
                Err(ExtensionSchemaIdError::InvalidName(_))
            ));
        }
        assert_eq!(
            ExtensionSchemaId::new("test:schema", 0),
            Err(ExtensionSchemaIdError::ZeroVersion)
        );
    }

    #[test]
    fn validates_canonical_bounded_media_types() {
        let media_type = ExtensionMediaType::new("application/vnd.gltf.buffer+octet-stream")
            .expect("canonical media type");
        assert_eq!(
            media_type.as_str(),
            "application/vnd.gltf.buffer+octet-stream"
        );
        assert_eq!(
            ExtensionMediaType::new("application/x`baozi")
                .expect("backtick is an RFC token byte")
                .as_str(),
            "application/x`baozi"
        );

        for value in [
            "",
            "application",
            "/json",
            "application/",
            "Application/json",
            "application/json; charset=utf-8",
            "application//json",
        ] {
            assert!(
                ExtensionMediaType::new(value).is_err(),
                "accepted {value:?}"
            );
        }

        let oversized = format!("a/{}", "b".repeat(MAX_EXTENSION_MEDIA_TYPE_BYTES));
        assert!(matches!(
            ExtensionMediaType::new(oversized),
            Err(ExtensionMediaTypeError::TooLong { .. })
        ));
    }

    #[test]
    fn enforces_finite_scalar_vector_color_matrix_and_array_values() {
        assert!(ExtensionValue::try_f64(f64::NAN).is_err());
        assert!(ExtensionValue::try_vec2(Vec2::new(f32::INFINITY, 0.0)).is_err());
        assert!(ExtensionValue::try_vec3(Vec3::new(0.0, f32::NAN, 0.0)).is_err());
        assert!(ExtensionValue::try_vec4(Vec4::new(0.0, 0.0, 0.0, f32::NAN)).is_err());
        assert!(ExtensionValue::try_color(Color::linear_rgba(0.0, 0.0, f32::NAN, 1.0)).is_err());

        let mut matrix = Mat4::IDENTITY;
        matrix.cols[2][1] = f32::INFINITY;
        assert!(ExtensionValue::try_mat4(matrix).is_err());

        assert_eq!(
            ExtensionValue::try_f64_array(&[0.0, f64::NEG_INFINITY]),
            Err(ExtensionValueError::NonFiniteArrayElement {
                element_type: ExtensionArrayElementType::F64,
                index: 1,
            })
        );
        assert!(
            ExtensionValue::try_vec3_array(&[Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0),]).is_err()
        );
    }

    #[test]
    fn enforces_string_byte_and_array_hard_bounds_before_copy() {
        let maximum_string = "s".repeat(MAX_EXTENSION_STRING_BYTES);
        assert!(ExtensionValue::try_string(&maximum_string).is_ok());
        let oversized_string = "s".repeat(MAX_EXTENSION_STRING_BYTES + 1);
        assert!(matches!(
            ExtensionValue::try_string(&oversized_string),
            Err(ExtensionValueError::StringTooLong { .. })
        ));

        let maximum_bytes = vec![0; MAX_EXTENSION_BYTE_PAYLOAD_BYTES];
        assert!(ExtensionValue::try_bytes("application/octet-stream", &maximum_bytes).is_ok());
        let oversized_bytes = vec![0; MAX_EXTENSION_BYTE_PAYLOAD_BYTES + 1];
        assert!(matches!(
            ExtensionValue::try_bytes("application/octet-stream", &oversized_bytes),
            Err(ExtensionValueError::BytePayloadTooLong { .. })
        ));

        let maximum_array = vec![0_u64; MAX_EXTENSION_ARRAY_ELEMENTS];
        assert!(ExtensionValue::try_u64_array(&maximum_array).is_ok());
        let oversized_array = vec![0_u64; MAX_EXTENSION_ARRAY_ELEMENTS + 1];
        assert!(matches!(
            ExtensionValue::try_u64_array(&oversized_array),
            Err(ExtensionValueError::ArrayTooLong { .. })
        ));

        let mut spare_capacity = Vec::with_capacity(1_024);
        spare_capacity.push(7_u8);
        let compact = ExtensionValue::try_bytes("application/octet-stream", &spare_capacity)
            .expect("bounded bytes should copy into canonical capacity");
        assert_eq!(compact.dynamic_payload_bytes(), 1);
        assert!(compact.retained_allocation_bytes() <= 1);
    }

    #[test]
    fn errors_and_debug_do_not_echo_hostile_value_text() {
        let secret = "do-not-echo-secret";
        let hostile = secret.repeat(MAX_EXTENSION_STRING_BYTES);
        let error = ExtensionValue::try_string(&hostile).unwrap_err();
        assert!(!error.to_string().contains(secret));

        let value = ExtensionValue::try_string(secret).unwrap();
        assert!(!format!("{value:?}").contains(secret));
    }

    #[test]
    fn dense_arrays_are_typed_flat_and_fallibly_cloneable() {
        let value =
            ExtensionValue::try_vec2_array(&[Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)]).unwrap();
        assert_eq!(
            value.kind(),
            ExtensionValueKind::DenseArray(ExtensionArrayElementType::Vec2)
        );
        assert_eq!(value.cell_count(), 2);
        assert!(value.retained_allocation_bytes() >= value.dynamic_payload_bytes());
        assert_eq!(value.try_clone().unwrap(), value);
        let ExtensionValueRef::DenseArray(array) = value.as_ref() else {
            panic!("expected dense array");
        };
        assert_eq!(array.element_type(), ExtensionArrayElementType::Vec2);
        assert_eq!(array.len(), 2);
    }

    #[test]
    fn entry_classification_is_descriptive_and_keys_are_unique() {
        let (_arena, provenance) = test_provenance_fixture();
        let first = ExtensionEntry::descriptive_metadata(
            NamespacedName::new("test:author").unwrap(),
            ExtensionValue::try_string("Baozi").unwrap(),
            provenance,
        );
        let second = ExtensionEntry::source_extension(
            NamespacedName::new("test:author").unwrap(),
            ExtensionValue::from_bool(true),
            provenance,
        );
        assert_eq!(
            first.classification(),
            ExtensionEntryClassification::DescriptiveMetadata
        );
        assert_eq!(
            validate_unique_extension_entry_keys([&first, &second]),
            Err(ExtensionEntryValidationError::DuplicateKey {
                first_index: 0,
                duplicate_index: 1,
            })
        );

        let clone = first.try_clone().unwrap();
        assert_eq!(clone, first);
        assert_eq!(clone.provenance(), provenance);
    }

    #[test]
    fn controlled_retained_footprint_stops_at_the_failed_entry() {
        let (_arena, provenance) = test_provenance_fixture();
        let entries = vec![
            ExtensionEntry::descriptive_metadata(
                NamespacedName::new("test:first").unwrap(),
                ExtensionValue::from_bool(true),
                provenance,
            ),
            ExtensionEntry::descriptive_metadata(
                NamespacedName::new("test:second").unwrap(),
                ExtensionValue::from_bool(true),
                provenance,
            ),
            ExtensionEntry::descriptive_metadata(
                NamespacedName::new("test:third").unwrap(),
                ExtensionValue::from_bool(true),
                provenance,
            ),
        ];
        let mut visited = 0;

        let error = extension_entries_retained_allocation_bytes_controlled(&entries, &mut || {
            visited += 1;
            if visited == 2 {
                Err("cancelled")
            } else {
                Ok(())
            }
        })
        .expect_err("the second callback should stop traversal");

        assert!(matches!(
            error,
            ControlledRetainedFootprintError::Control("cancelled")
        ));
        assert_eq!(visited, 2);
    }

    #[test]
    fn schema_limits_only_tighten_core_bounds_and_validate_values() {
        let invalid = ExtensionSchema::new(
            ExtensionSchemaId::new("test:string", 1).unwrap(),
            ExtensionSchemaOwner::Scene,
            ExtensionValueSchema::String {
                max_bytes: u32::try_from(MAX_EXTENSION_STRING_BYTES + 1).unwrap(),
            },
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        );
        assert!(matches!(
            invalid,
            Err(ExtensionSchemaError::LimitExceedsCore {
                limit: ExtensionSchemaLimit::StringBytes,
                ..
            })
        ));

        let schema = plain_schema("test:string", ExtensionValueSchema::String { max_bytes: 3 });
        assert!(
            schema
                .validate_value(&ExtensionValue::try_string("abc").unwrap())
                .is_ok()
        );
        assert!(matches!(
            schema.validate_value(&ExtensionValue::try_string("abcd").unwrap()),
            Err(ExtensionSchemaValueError::LimitExceeded {
                limit: ExtensionSchemaValueLimit::StringBytes,
                ..
            })
        ));
        assert!(matches!(
            schema.validate_value(&ExtensionValue::from_u64(1)),
            Err(ExtensionSchemaValueError::KindMismatch { .. })
        ));
    }

    #[test]
    fn schema_reference_remap_and_invalidation_rules_are_closed_and_consistent() {
        let invalid_owner = ExtensionSchema::new(
            ExtensionSchemaId::new("test:reference", 1).unwrap(),
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::U64,
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::SameGenerationAndLogicalMeshOwner(SceneEntityKind::Material),
            ExtensionRemapRule::RemapCanonicalReference,
            ExtensionInvalidationRule::OnReferencedEntityChange,
        );
        assert_eq!(
            invalid_owner,
            Err(ExtensionSchemaError::InvalidOwnerQualifiedReference {
                kind: SceneEntityKind::Material,
            })
        );

        let invalid_remap = ExtensionSchema::new(
            ExtensionSchemaId::new("test:domain", 1).unwrap(),
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::F64,
            ExtensionCardinality::PerAttributeDomain(AttributeDomain::Vertex),
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::OnOwnerChange,
        );
        assert_eq!(
            invalid_remap,
            Err(ExtensionSchemaError::InconsistentRemapRule)
        );

        let valid = ExtensionSchema::new(
            ExtensionSchemaId::new("test:domain-reference", 1).unwrap(),
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::DenseArray {
                element_type: ExtensionArrayElementType::U64,
                max_elements: 16,
            },
            ExtensionCardinality::PerAttributeDomain(AttributeDomain::Corner),
            ExtensionReferenceRule::SameGeneration(SceneEntityKind::Material),
            ExtensionRemapRule::RemapCardinalityAndReference,
            ExtensionInvalidationRule::OnOwnerOrReferencedEntityChange,
        );
        assert!(valid.is_ok());
    }

    #[test]
    fn packed_attribute_schemas_are_closed_executable_and_body_bound() {
        assert_eq!(
            PackedAttributeEncodingSchema::new(
                2,
                3,
                PackedByteOrder::LittleEndian,
                PackedAttributeElementValidator::CanonicalOctahedralSnorm16,
                PackedAttributeTypedDecoder::UnitVector3OctahedralSnorm16,
            ),
            Err(PackedAttributeEncodingSchemaError::UnsupportedContract)
        );

        let encoding = PackedAttributeEncodingSchema::octahedral_unit_vector_snorm16(
            PackedByteOrder::LittleEndian,
        )
        .unwrap();
        assert_eq!(encoding.element_width(), 4);
        assert_eq!(encoding.logical_lanes(), 3);
        assert_eq!(encoding.validate_payload(&[0, 0, 0, 0]), Ok(1));
        assert!(matches!(
            encoding.validate_payload(&[0, 0x80, 0, 0]),
            Err(PackedAttributeValidationError::ReservedBitPattern { .. })
        ));
        assert_eq!(
            encoding.decode_vec3_element(&[0, 0, 0, 0]).unwrap(),
            Vec3::new(0.0, 0.0, 1.0)
        );
        assert_eq!(
            encoding
                .decode_vec3_element(&[0xff, 0x7f, 0xff, 0x7f])
                .unwrap(),
            Vec3::new(0.0, 0.0, -1.0)
        );

        let big_endian = PackedAttributeEncodingSchema::octahedral_unit_vector_snorm16(
            PackedByteOrder::BigEndian,
        )
        .unwrap();
        assert_eq!(big_endian.validate_payload(&[0x7f, 0xff, 0, 0]), Ok(1));
        assert_eq!(
            big_endian.decode_vec3_element(&[0x7f, 0xff, 0, 0]).unwrap(),
            Vec3::new(1.0, 0.0, 0.0)
        );
        assert_eq!(
            big_endian
                .decode_vec3_element(&[0x7f, 0xff, 0x7f, 0xff])
                .unwrap(),
            Vec3::new(0.0, 0.0, -1.0)
        );
        assert!(matches!(
            big_endian.validate_payload(&[0x80, 0, 0, 0]),
            Err(PackedAttributeValidationError::ReservedBitPattern { .. })
        ));

        let wrong_owner = ExtensionSchema::new_with_behavior(
            ExtensionSchemaId::new("test:wrong-packed-owner", 1).unwrap(),
            ExtensionSchemaOwner::Scene,
            ExtensionValueSchema::Vec3,
            ExtensionSchemaBehavior::PackedAttribute(encoding),
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        );
        assert_eq!(
            wrong_owner,
            Err(ExtensionSchemaError::PackedAttributeOwnerRequired)
        );

        let mut snapshot = ExtensionSchemaSnapshotBuilder::new();
        snapshot
            .try_insert(packed_attribute_schema(
                "test:packed-body",
                PackedByteOrder::LittleEndian,
            ))
            .unwrap();
        assert_eq!(
            snapshot.try_insert(packed_attribute_schema(
                "test:packed-body",
                PackedByteOrder::BigEndian,
            )),
            Err(ExtensionSchemaSnapshotError::ConflictingSchemaBody)
        );
    }

    #[test]
    fn schema_snapshot_rejects_duplicate_and_conflicting_ids() {
        let schema = plain_schema("test:schema", ExtensionValueSchema::Bool);
        let duplicate = schema.clone();
        let conflict = plain_schema("test:schema", ExtensionValueSchema::U64);
        let mut builder = ExtensionSchemaSnapshotBuilder::new();
        builder.try_insert(schema).unwrap();
        assert_eq!(
            builder.try_insert(duplicate),
            Err(ExtensionSchemaSnapshotError::DuplicateSchemaId)
        );
        assert_eq!(
            builder.try_insert(conflict),
            Err(ExtensionSchemaSnapshotError::ConflictingSchemaBody)
        );
    }

    #[test]
    fn schema_builder_capacity_qualification_failure_is_transactional_and_retryable() {
        let mut builder = ExtensionSchemaSnapshotBuilder::new();
        let before = (builder.schemas.len(), builder.schemas.capacity());

        fail_next_schema_capacity_qualification_for_test();
        assert_eq!(
            builder.try_insert(plain_schema(
                "test:qualification",
                ExtensionValueSchema::Bool,
            )),
            Err(ExtensionSchemaSnapshotError::AllocationFailed)
        );
        assert_eq!((builder.schemas.len(), builder.schemas.capacity()), before);

        builder
            .try_insert(plain_schema("test:retry", ExtensionValueSchema::Bool))
            .unwrap();
        let snapshot = builder.build();
        assert_eq!(snapshot.len(), 1);
        assert!(
            snapshot
                .get(&ExtensionSchemaId::new("test:retry", 1).unwrap())
                .is_some()
        );
    }

    #[test]
    fn schema_snapshot_bulk_build_sorts_once_and_classifies_duplicate_groups() {
        let schemas = (0..1_024).rev().map(|index| {
            plain_schema(
                &format!("test:schema-{index:04}"),
                ExtensionValueSchema::Bool,
            )
        });
        let snapshot = ExtensionSchemaSnapshot::try_from_schemas(schemas).unwrap();
        let ordered: Vec<_> = snapshot
            .iter()
            .map(|schema| schema.id().name().as_str())
            .collect();
        assert_eq!(ordered.first().copied(), Some("test:schema-0000"));
        assert_eq!(ordered.last().copied(), Some("test:schema-1023"));

        let duplicate = plain_schema("test:duplicate", ExtensionValueSchema::Bool);
        assert_eq!(
            ExtensionSchemaSnapshot::try_from_schemas([duplicate.clone(), duplicate]),
            Err(ExtensionSchemaSnapshotError::DuplicateSchemaId)
        );

        let base = plain_schema("test:conflict", ExtensionValueSchema::Bool);
        let conflict = plain_schema("test:conflict", ExtensionValueSchema::U64);
        assert_eq!(
            ExtensionSchemaSnapshot::try_from_schemas([base.clone(), base, conflict]),
            Err(ExtensionSchemaSnapshotError::ConflictingSchemaBody)
        );
    }

    #[test]
    fn schema_snapshot_has_canonical_order_lookup_and_empty_identity() {
        let mut builder = ExtensionSchemaSnapshotBuilder::new();
        builder
            .try_insert(plain_schema("zeta:schema", ExtensionValueSchema::Bool))
            .unwrap();
        builder
            .try_insert(plain_schema("alpha:schema", ExtensionValueSchema::Bool))
            .unwrap();
        builder
            .try_insert(
                ExtensionSchema::new(
                    ExtensionSchemaId::new("alpha:schema", 2).unwrap(),
                    ExtensionSchemaOwner::Scene,
                    ExtensionValueSchema::Bool,
                    ExtensionCardinality::ExactlyOne,
                    ExtensionReferenceRule::None,
                    ExtensionRemapRule::Preserve,
                    ExtensionInvalidationRule::Never,
                )
                .unwrap(),
            )
            .unwrap();
        let snapshot = builder.build();
        let ordered: Vec<_> = snapshot
            .iter()
            .map(|schema| (schema.id().name().as_str(), schema.id().version().get()))
            .collect();
        assert_eq!(
            ordered,
            [("alpha:schema", 1), ("alpha:schema", 2), ("zeta:schema", 1),]
        );
        assert!(
            snapshot
                .get(&ExtensionSchemaId::new("alpha:schema", 2).unwrap())
                .is_some()
        );
        assert!(
            snapshot.retained_allocation_bytes()
                >= snapshot
                    .len()
                    .saturating_mul(std::mem::size_of::<ExtensionSchema>())
        );
        assert_eq!(snapshot.try_clone().unwrap(), snapshot);
        assert!(ExtensionSchemaSnapshot::empty().is_empty());
    }

    #[test]
    fn scene_snapshot_remains_independent_after_schema_supplier_teardown() {
        struct Supplier {
            schema: Option<ExtensionSchema>,
            dropped: Rc<Cell<bool>>,
        }

        impl Drop for Supplier {
            fn drop(&mut self) {
                self.dropped.set(true);
            }
        }

        let dropped = Rc::new(Cell::new(false));
        let snapshot = {
            let mut supplier = Supplier {
                schema: Some(plain_schema("supplier:schema", ExtensionValueSchema::Bool)),
                dropped: Rc::clone(&dropped),
            };
            ExtensionSchemaSnapshot::try_from_schemas([supplier.schema.take().unwrap()]).unwrap()
        };

        assert!(dropped.get());
        assert!(
            snapshot
                .get(&ExtensionSchemaId::new("supplier:schema", 1).unwrap())
                .is_some()
        );
    }
}
