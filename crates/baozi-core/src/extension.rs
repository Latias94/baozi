//! Bounded borrowed metadata values and extension identities.

use crate::math::{Color, Mat4, Vec2, Vec3, Vec4};
use crate::namespaced::{NamespacedName, NamespacedNameError};
use std::fmt;
use std::num::NonZeroU32;
use thiserror::Error;

/// Maximum bytes in one canonical media type.
pub const MAX_EXTENSION_MEDIA_TYPE_BYTES: usize = 128;

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

pub(crate) fn validate_media_type(value: &str) -> Result<(), ExtensionMediaTypeError> {
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

#[cfg(test)]
mod tests {
    use super::{ExtensionMediaType, ExtensionMediaTypeError, ExtensionSchemaId};

    #[test]
    fn media_type_requires_one_canonical_separator() {
        assert_eq!(
            ExtensionMediaType::new("Image/PNG"),
            Err(ExtensionMediaTypeError::InvalidByte { offset: 0 })
        );
        assert_eq!(
            ExtensionMediaType::new("image/png").unwrap().as_str(),
            "image/png"
        );
    }

    #[test]
    fn schema_identity_requires_a_positive_version() {
        assert!(ExtensionSchemaId::new("org.baozi:test", 1).is_ok());
        assert!(ExtensionSchemaId::new("org.baozi:test", 0).is_err());
    }
}
