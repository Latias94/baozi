use std::cmp::Ordering;
use std::fmt;

use thiserror::Error;

/// Stable category for a rejected repository-relative path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RepositoryPathErrorCode {
    Empty,
    TooLong,
    Absolute,
    Backslash,
    EmptyComponent,
    DotComponent,
    ComponentTooLong,
    LeadingOrTrailingSpace,
    TrailingDot,
    ControlCharacter,
    NonPortableUnicode,
    InvalidCharacter,
    ShortNameAlias,
    ReservedDevice,
    AllocationFailed,
}

/// Failure to construct a strict repository-relative path.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepositoryPathError {
    #[error("repository path is empty")]
    Empty,

    #[error("repository path is {actual} bytes; the hard maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("absolute repository paths are not allowed")]
    Absolute,

    #[error("repository paths must use forward slashes")]
    Backslash,

    #[error("empty repository path component {component} is not allowed")]
    EmptyComponent { component: usize },

    #[error("dot repository path component {component} is not allowed")]
    DotComponent { component: usize },

    #[error(
        "repository path component {component} is {actual} bytes; the hard maximum is {maximum}"
    )]
    ComponentTooLong {
        component: usize,
        actual: usize,
        maximum: usize,
    },

    #[error("repository path component {component} starts or ends with a space")]
    LeadingOrTrailingSpace { component: usize },

    #[error("repository path component {component} ends with a dot")]
    TrailingDot { component: usize },

    #[error(
        "control character at byte {byte_index} in repository path component {component} is not allowed"
    )]
    ControlCharacter { component: usize, byte_index: usize },

    #[error("non-ASCII spelling is not admitted by the portable repository profile")]
    NonPortableUnicode { component: usize },

    #[error("platform-ambiguous character is not allowed in component {component}")]
    InvalidCharacter { component: usize },

    #[error("Windows short-name alias syntax is not allowed in component {component}")]
    ShortNameAlias { component: usize },

    #[error("component {component} is a reserved Windows device alias")]
    ReservedDevice { component: usize },

    #[error("repository path storage allocation failed")]
    AllocationFailed,
}

impl RepositoryPathError {
    pub const fn code(&self) -> RepositoryPathErrorCode {
        match self {
            Self::Empty => RepositoryPathErrorCode::Empty,
            Self::TooLong { .. } => RepositoryPathErrorCode::TooLong,
            Self::Absolute => RepositoryPathErrorCode::Absolute,
            Self::Backslash => RepositoryPathErrorCode::Backslash,
            Self::EmptyComponent { .. } => RepositoryPathErrorCode::EmptyComponent,
            Self::DotComponent { .. } => RepositoryPathErrorCode::DotComponent,
            Self::ComponentTooLong { .. } => RepositoryPathErrorCode::ComponentTooLong,
            Self::LeadingOrTrailingSpace { .. } => RepositoryPathErrorCode::LeadingOrTrailingSpace,
            Self::TrailingDot { .. } => RepositoryPathErrorCode::TrailingDot,
            Self::ControlCharacter { .. } => RepositoryPathErrorCode::ControlCharacter,
            Self::NonPortableUnicode { .. } => RepositoryPathErrorCode::NonPortableUnicode,
            Self::InvalidCharacter { .. } => RepositoryPathErrorCode::InvalidCharacter,
            Self::ShortNameAlias { .. } => RepositoryPathErrorCode::ShortNameAlias,
            Self::ReservedDevice { .. } => RepositoryPathErrorCode::ReservedDevice,
            Self::AllocationFailed => RepositoryPathErrorCode::AllocationFailed,
        }
    }
}

/// A spelling-preserving UTF-8 repository path in the portable ASCII profile.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryPath(String);

impl RepositoryPath {
    pub const HARD_MAX_BYTES: usize = 4096;
    pub const HARD_MAX_COMPONENT_BYTES: usize = 255;

    pub fn new(path: impl AsRef<str>) -> Result<Self, RepositoryPathError> {
        let path = path.as_ref();
        validate(path)?;

        let mut retained = String::new();
        retained
            .try_reserve_exact(path.len())
            .map_err(|_| RepositoryPathError::AllocationFailed)?;
        retained.push_str(path);
        Ok(Self(retained))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn components(&self) -> impl Iterator<Item = &str> + Clone {
        self.0.split('/')
    }

    pub fn component_count(&self) -> usize {
        let separators = self.0.bytes().filter(|byte| *byte == b'/').count();
        if separators == usize::MAX {
            usize::MAX
        } else {
            separators + 1
        }
    }

    pub fn file_name(&self) -> &str {
        self.0
            .rsplit_once('/')
            .map_or(self.0.as_str(), |(_, name)| name)
    }

    pub fn try_clone(&self) -> Result<Self, RepositoryPathError> {
        let mut retained = String::new();
        retained
            .try_reserve_exact(self.0.len())
            .map_err(|_| RepositoryPathError::AllocationFailed)?;
        retained.push_str(&self.0);
        Ok(Self(retained))
    }

    pub(super) fn joined_len(&self, component: &str) -> Option<usize> {
        self.0
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(component.len()))
    }

    pub(super) fn join_component(&self, component: &str) -> Result<Self, RepositoryPathError> {
        validate_component(component, self.component_count())?;
        let length = self
            .joined_len(component)
            .ok_or(RepositoryPathError::AllocationFailed)?;
        if length > Self::HARD_MAX_BYTES {
            return Err(RepositoryPathError::TooLong {
                actual: length,
                maximum: Self::HARD_MAX_BYTES,
            });
        }

        let mut joined = String::new();
        joined
            .try_reserve_exact(length)
            .map_err(|_| RepositoryPathError::AllocationFailed)?;
        joined.push_str(&self.0);
        joined.push('/');
        joined.push_str(component);
        Ok(Self(joined))
    }

    pub(super) fn from_valid_prefix(prefix: &str) -> Result<Self, RepositoryPathError> {
        ::core::debug_assert!(validate(prefix).is_ok());
        let mut retained = String::new();
        retained
            .try_reserve_exact(prefix.len())
            .map_err(|_| RepositoryPathError::AllocationFailed)?;
        retained.push_str(prefix);
        Ok(Self(retained))
    }
}

impl fmt::Display for RepositoryPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub(super) fn validate_component(component: &str, index: usize) -> Result<(), RepositoryPathError> {
    if component.is_empty() {
        return Err(RepositoryPathError::EmptyComponent { component: index });
    }
    if ::core::matches!(component, "." | "..") {
        return Err(RepositoryPathError::DotComponent { component: index });
    }
    if component.len() > RepositoryPath::HARD_MAX_COMPONENT_BYTES {
        return Err(RepositoryPathError::ComponentTooLong {
            component: index,
            actual: component.len(),
            maximum: RepositoryPath::HARD_MAX_COMPONENT_BYTES,
        });
    }
    if component.starts_with(' ') || component.ends_with(' ') {
        return Err(RepositoryPathError::LeadingOrTrailingSpace { component: index });
    }
    if component.ends_with('.') {
        return Err(RepositoryPathError::TrailingDot { component: index });
    }
    if let Some((byte_index, _)) = component
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(RepositoryPathError::ControlCharacter {
            component: index,
            byte_index,
        });
    }
    if !component.is_ascii() {
        return Err(RepositoryPathError::NonPortableUnicode { component: index });
    }
    if component
        .chars()
        .any(|character| ::core::matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(RepositoryPathError::InvalidCharacter { component: index });
    }
    if component.contains('~') {
        return Err(RepositoryPathError::ShortNameAlias { component: index });
    }
    if is_reserved_device(component) {
        return Err(RepositoryPathError::ReservedDevice { component: index });
    }
    Ok(())
}

pub(super) fn portable_component_cmp(left: &str, right: &str) -> Ordering {
    folded_component_cmp(left, right).then_with(|| left.as_bytes().cmp(right.as_bytes()))
}

pub(super) fn is_portable_alias(left: &str, right: &str) -> bool {
    left != right && left.eq_ignore_ascii_case(right)
}

pub(super) fn portable_path_cmp(left: &RepositoryPath, right: &RepositoryPath) -> Ordering {
    let mut left_components = left.components();
    let mut right_components = right.components();
    loop {
        match (left_components.next(), right_components.next()) {
            (Some(left), Some(right)) => {
                let ordering = folded_component_cmp(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return left.as_str().cmp(right.as_str()),
        }
    }
}

fn folded_component_cmp(left: &str, right: &str) -> Ordering {
    left.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
}

pub(super) fn paths_portably_equal(left: &RepositoryPath, right: &RepositoryPath) -> bool {
    left.components()
        .zip(right.components())
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
        && left.component_count() == right.component_count()
}

pub(super) fn is_portable_path_prefix(parent: &RepositoryPath, child: &RepositoryPath) -> bool {
    let parent_components = parent.components();
    let mut child_components = child.components();
    let mut compared = 0usize;
    for parent_component in parent_components {
        let Some(child_component) = child_components.next() else {
            return false;
        };
        if !parent_component.eq_ignore_ascii_case(child_component) {
            return false;
        }
        compared += 1;
    }
    compared < child.component_count()
}

fn validate(path: &str) -> Result<(), RepositoryPathError> {
    if path.is_empty() {
        return Err(RepositoryPathError::Empty);
    }
    if path.len() > RepositoryPath::HARD_MAX_BYTES {
        return Err(RepositoryPathError::TooLong {
            actual: path.len(),
            maximum: RepositoryPath::HARD_MAX_BYTES,
        });
    }
    if path.starts_with('/') {
        return Err(RepositoryPathError::Absolute);
    }
    if path.contains('\\') {
        return Err(RepositoryPathError::Backslash);
    }

    for (index, component) in path.split('/').enumerate() {
        validate_component(component, index)?;
    }
    Ok(())
}

fn is_reserved_device(component: &str) -> bool {
    let basename = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(['.', ' ']);
    if ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"]
        .iter()
        .any(|reserved| basename.eq_ignore_ascii_case(reserved))
    {
        return true;
    }

    let Some((prefix, suffix)) = basename.get(..3).zip(basename.get(3..)) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case("COM") && !prefix.eq_ignore_ascii_case("LPT") {
        return false;
    }
    ::core::matches!(
        suffix,
        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
    ) || ::core::matches!(suffix, "\u{00b9}" | "\u{00b2}" | "\u{00b3}")
}
