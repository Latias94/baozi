use std::borrow::Borrow;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use thiserror::Error;

/// Maximum encoded length of a canonical namespaced identifier.
pub const MAX_NAMESPACED_NAME_LEN: usize = 64;

/// A bounded lowercase ASCII `namespace:name` identifier.
///
/// Namespace labels may contain lowercase alphanumeric bytes, dots, and
/// hyphens. The local name may additionally contain underscores. Construction
/// does not case-fold or otherwise guess a normalization.
#[derive(Clone)]
pub struct NamespacedName {
    bytes: [u8; MAX_NAMESPACED_NAME_LEN],
}

impl NamespacedName {
    pub fn new(value: impl AsRef<str>) -> Result<Self, NamespacedNameError> {
        let value = value.as_ref();
        validate_namespaced_name(value)?;
        let mut bytes = [0; MAX_NAMESPACED_NAME_LEN];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self { bytes })
    }

    pub fn as_str(&self) -> &str {
        let len = self
            .bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(MAX_NAMESPACED_NAME_LEN);
        std::str::from_utf8(&self.bytes[..len])
            .expect("NamespacedName construction accepts only ASCII bytes")
    }
}

impl std::fmt::Debug for NamespacedName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("NamespacedName")
            .field(&self.as_str())
            .finish()
    }
}

impl PartialEq for NamespacedName {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for NamespacedName {}

impl PartialOrd for NamespacedName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NamespacedName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for NamespacedName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl AsRef<str> for NamespacedName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for NamespacedName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for NamespacedName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for NamespacedName {
    type Err = NamespacedNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NamespacedNameError {
    #[error("namespaced identifier is empty")]
    Empty,
    #[error("namespaced identifier exceeds {maximum} bytes")]
    TooLong { maximum: usize },
    #[error("namespaced identifier must contain exactly one separator")]
    Separator,
    #[error("namespaced identifier contains an invalid byte at offset {offset}")]
    InvalidByte { offset: usize },
}

pub(crate) fn validate_namespaced_name(value: &str) -> Result<(), NamespacedNameError> {
    if value.is_empty() {
        return Err(NamespacedNameError::Empty);
    }
    if value.len() > MAX_NAMESPACED_NAME_LEN {
        return Err(NamespacedNameError::TooLong {
            maximum: MAX_NAMESPACED_NAME_LEN,
        });
    }

    let separator = value
        .bytes()
        .position(|byte| byte == b':')
        .ok_or(NamespacedNameError::Separator)?;
    if separator == 0
        || separator + 1 == value.len()
        || value.as_bytes()[separator + 1..].contains(&b':')
    {
        return Err(NamespacedNameError::Separator);
    }

    for (offset, byte) in value.bytes().enumerate() {
        let valid = if offset < separator {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        } else if offset == separator {
            true
        } else {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        };
        if !valid {
            return Err(NamespacedNameError::InvalidByte { offset });
        }
    }

    validate_namespace_components(&value.as_bytes()[..separator])?;
    if value.as_bytes()[separator + 1].is_ascii_punctuation()
        || value.as_bytes()[value.len() - 1].is_ascii_punctuation()
    {
        return Err(NamespacedNameError::InvalidByte { offset: separator });
    }

    Ok(())
}

fn validate_namespace_components(namespace: &[u8]) -> Result<(), NamespacedNameError> {
    let mut component_start = 0;
    for component_end in namespace
        .iter()
        .enumerate()
        .filter_map(|(offset, byte)| (*byte == b'.').then_some(offset))
        .chain(std::iter::once(namespace.len()))
    {
        let Some(component) = namespace.get(component_start..component_end) else {
            return Err(NamespacedNameError::InvalidByte {
                offset: component_start,
            });
        };
        if component.is_empty() {
            return Err(NamespacedNameError::InvalidByte {
                offset: component_start,
            });
        }
        if component
            .first()
            .is_some_and(|byte| byte.is_ascii_punctuation())
        {
            return Err(NamespacedNameError::InvalidByte {
                offset: component_start,
            });
        }
        if component
            .last()
            .is_some_and(|byte| byte.is_ascii_punctuation())
        {
            return Err(NamespacedNameError::InvalidByte {
                offset: component_end - 1,
            });
        }
        component_start = component_end + 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_names() {
        for value in ["obj:smoothing_group", "gltf.ext:source-value", "x1:a2"] {
            assert_eq!(NamespacedName::new(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn rejects_noncanonical_names() {
        for value in [
            "",
            "bare",
            ":name",
            "owner:",
            "owner::name",
            "Owner:name",
            "owner:Name",
            "owner/name:value",
            "owner:.name",
            "owner:name-",
        ] {
            assert!(NamespacedName::new(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn rejects_empty_or_punctuated_namespace_components() {
        assert!(NamespacedName::new("vendor.ext:name").is_ok());

        for value in [
            "vendor..ext:name",
            "vendor.-ext:name",
            "vendor.ext-:name",
            "vendor-.ext:name",
        ] {
            assert!(NamespacedName::new(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn ordering_and_hashing_use_the_canonical_text() {
        let first = NamespacedName::new("a:first").unwrap();
        let second = NamespacedName::new("a:second").unwrap();
        assert!(first < second);
        assert_eq!(first, NamespacedName::new("a:first").unwrap());
    }
}
