use std::borrow::Borrow;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use thiserror::Error;

/// Maximum encoded length of a stable format identifier.
pub const MAX_FORMAT_ID_LEN: usize = 64;

/// A stable, registry-independent source-format identity.
///
/// The grammar is one or more dot-separated labels. Each label contains
/// lowercase ASCII alphanumeric segments separated by single hyphens. The
/// value is bounded and contains no registry authority.
#[derive(Clone)]
pub struct FormatId {
    bytes: [u8; MAX_FORMAT_ID_LEN],
}

impl FormatId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, FormatIdError> {
        let value = value.as_ref();
        validate_format_id(value)?;
        let mut bytes = [0; MAX_FORMAT_ID_LEN];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self { bytes })
    }

    pub fn as_str(&self) -> &str {
        let len = self
            .bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(MAX_FORMAT_ID_LEN);
        std::str::from_utf8(&self.bytes[..len])
            .expect("FormatId construction accepts only ASCII bytes")
    }
}

impl std::fmt::Debug for FormatId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("FormatId")
            .field(&self.as_str())
            .finish()
    }
}

impl PartialEq for FormatId {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for FormatId {}

impl PartialOrd for FormatId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FormatId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for FormatId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl AsRef<str> for FormatId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for FormatId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for FormatId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FormatId {
    type Err = FormatIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<&str> for FormatId {
    type Error = FormatIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for FormatId {
    type Error = FormatIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Why a source-format identity failed its canonical grammar.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FormatIdError {
    #[error("format id must not be empty")]
    Empty,

    #[error("format id is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("format id byte at offset {offset} is not lowercase ASCII alphanumeric, '-' or '.'")]
    InvalidByte { offset: usize },

    #[error("format id has a leading, trailing, or repeated separator at byte offset {offset}")]
    InvalidSeparator { offset: usize },
}

pub(crate) fn validate_format_id(value: &str) -> Result<(), FormatIdError> {
    if value.is_empty() {
        return Err(FormatIdError::Empty);
    }
    if value.len() > MAX_FORMAT_ID_LEN {
        return Err(FormatIdError::TooLong {
            actual: value.len(),
            maximum: MAX_FORMAT_ID_LEN,
        });
    }

    let mut previous_was_separator = true;
    let mut last_separator_offset = None;
    for (offset, byte) in value.bytes().enumerate() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => {
                previous_was_separator = false;
                last_separator_offset = None;
            }
            b'-' | b'.' if !previous_was_separator => {
                previous_was_separator = true;
                last_separator_offset = Some(offset);
            }
            b'-' | b'.' => return Err(FormatIdError::InvalidSeparator { offset }),
            _ => return Err(FormatIdError::InvalidByte { offset }),
        }
    }

    if previous_was_separator {
        return Err(FormatIdError::InvalidSeparator {
            offset: last_separator_offset.unwrap_or(0),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_format_ids() {
        for value in [
            "obj",
            "gltf",
            "3mf",
            "wavefront-obj",
            "format2",
            "org.example.asset-format",
        ] {
            assert_eq!(FormatId::new(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn rejects_noncanonical_format_ids() {
        let cases = [
            ("", FormatIdError::Empty),
            ("GLTF", FormatIdError::InvalidByte { offset: 0 }),
            ("gltf_2", FormatIdError::InvalidByte { offset: 4 }),
            ("-obj", FormatIdError::InvalidSeparator { offset: 0 }),
            ("obj--binary", FormatIdError::InvalidSeparator { offset: 4 }),
            ("obj-", FormatIdError::InvalidSeparator { offset: 3 }),
            (
                "org..example.x",
                FormatIdError::InvalidSeparator { offset: 4 },
            ),
            (".org.x", FormatIdError::InvalidSeparator { offset: 0 }),
            (
                "org.example.",
                FormatIdError::InvalidSeparator { offset: 11 },
            ),
            ("对象", FormatIdError::InvalidByte { offset: 0 }),
        ];

        for (value, expected) in cases {
            assert_eq!(FormatId::new(value).unwrap_err(), expected);
        }
    }

    #[test]
    fn enforces_encoded_length_bound() {
        let maximum = "a".repeat(MAX_FORMAT_ID_LEN);
        assert_eq!(FormatId::new(&maximum).unwrap().as_str(), maximum);

        let value = "a".repeat(MAX_FORMAT_ID_LEN + 1);
        assert_eq!(
            FormatId::new(value).unwrap_err(),
            FormatIdError::TooLong {
                actual: MAX_FORMAT_ID_LEN + 1,
                maximum: MAX_FORMAT_ID_LEN,
            }
        );
    }

    #[test]
    fn cloning_and_string_conversion_need_no_identifier_allocation() {
        let value = FormatId::new("org.example.asset-format").unwrap();
        let cloned = value.clone();
        let from_string = FormatId::try_from(String::from("org.example.asset-format")).unwrap();

        assert_eq!(cloned, value);
        assert_eq!(from_string, value);
        assert_eq!(std::mem::size_of::<FormatId>(), MAX_FORMAT_ID_LEN);
    }

    #[test]
    fn borrowed_lookup_and_order_match_str_semantics() {
        use std::collections::{BTreeMap, HashMap};

        let short = FormatId::new("org.example.a").unwrap();
        let long = FormatId::new("org.example.aa").unwrap();
        assert!(short < long);

        let mut ordered = BTreeMap::new();
        ordered.insert(long.clone(), 2);
        ordered.insert(short.clone(), 1);
        assert_eq!(ordered.get("org.example.a"), Some(&1));

        let mut hashed = HashMap::new();
        hashed.insert(short, 1);
        assert_eq!(hashed.get("org.example.a"), Some(&1));
    }
}
