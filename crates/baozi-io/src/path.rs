use crate::PathError;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetPath(String);

impl AssetPath {
    pub const MAX_BYTES: usize = 4096;

    pub fn new(path: impl AsRef<str>) -> Result<Self, PathError> {
        normalize(path.as_ref(), &[]).map(Self)
    }

    pub fn resolve_from_file(&self, reference: &str) -> Result<Self, PathError> {
        let component_count = self
            .0
            .bytes()
            .filter(|byte| *byte == b'/')
            .count()
            .checked_add(1)
            .ok_or(PathError::AllocationFailed)?;
        let mut base = Vec::new();
        base.try_reserve_exact(component_count)
            .map_err(|_| PathError::AllocationFailed)?;
        for component in self.0.split('/') {
            base.push(component);
        }
        base.pop();
        normalize(reference, &base).map(Self)
    }

    pub fn join(&self, reference: &str) -> Result<Self, PathError> {
        self.resolve_from_file(reference)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the retained Baozi-owned string capacity in bytes.
    pub const fn capacity_bytes(&self) -> usize {
        self.0.capacity()
    }

    /// Fallibly copies this bounded logical path.
    pub fn try_clone(&self) -> Result<Self, PathError> {
        let mut path = String::new();
        path.try_reserve_exact(self.0.len())
            .map_err(|_| PathError::AllocationFailed)?;
        path.push_str(&self.0);
        Ok(Self(path))
    }

    pub fn extension(&self) -> Option<&str> {
        self.0
            .rsplit_once('/')
            .map_or(self.0.as_str(), |(_, file_name)| file_name)
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .filter(|extension| !extension.is_empty())
    }
}

impl fmt::Display for AssetPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn normalize(path: &str, base: &[&str]) -> Result<String, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > AssetPath::MAX_BYTES {
        return Err(PathError::TooLong {
            actual: path.len(),
            maximum: AssetPath::MAX_BYTES,
        });
    }
    if let Some((byte_index, _)) = path
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(PathError::ControlCharacter { byte_index });
    }

    let mut slashed = String::new();
    slashed
        .try_reserve_exact(path.len())
        .map_err(|_| PathError::AllocationFailed)?;
    for character in path.chars() {
        slashed.push(if character == '\\' { '/' } else { character });
    }
    reject_prefix(&slashed)?;

    let incoming_components = slashed
        .bytes()
        .filter(|byte| *byte == b'/')
        .count()
        .checked_add(1)
        .ok_or(PathError::AllocationFailed)?;
    let part_capacity = base
        .len()
        .checked_add(incoming_components)
        .ok_or(PathError::AllocationFailed)?;
    let mut parts = Vec::new();
    parts
        .try_reserve_exact(part_capacity)
        .map_err(|_| PathError::AllocationFailed)?;
    for part in base {
        parts.push(*part);
    }
    let mut raw_component_index = 0usize;
    for component in slashed.split('/') {
        if component.is_empty() {
            return Err(PathError::EmptyComponent {
                component: raw_component_index,
            });
        }
        if component == "." {
            raw_component_index += 1;
            continue;
        }
        if component == ".." {
            if parts.pop().is_none() {
                return Err(PathError::Escape);
            }
            raw_component_index += 1;
            continue;
        }

        validate_component(component, raw_component_index)?;
        parts.push(component);
        raw_component_index += 1;
    }

    if parts.is_empty() {
        return Err(PathError::Empty);
    }
    let normalized_len = parts
        .iter()
        .try_fold(parts.len().saturating_sub(1), |total, component| {
            total.checked_add(component.len())
        });
    let normalized_len = normalized_len.ok_or(PathError::AllocationFailed)?;
    if normalized_len > AssetPath::MAX_BYTES {
        return Err(PathError::TooLong {
            actual: normalized_len,
            maximum: AssetPath::MAX_BYTES,
        });
    }
    let mut normalized = String::new();
    normalized
        .try_reserve_exact(normalized_len)
        .map_err(|_| PathError::AllocationFailed)?;
    for (index, component) in parts.into_iter().enumerate() {
        if index != 0 {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    Ok(normalized)
}

fn reject_prefix(path: &str) -> Result<(), PathError> {
    if starts_with_ascii_case(path, "//?/")
        || starts_with_ascii_case(path, "//./")
        || starts_with_ascii_case(path, "/??/")
        || starts_with_ascii_case(path, "/device/")
        || starts_with_ascii_case(path, "globalroot/")
    {
        return Err(PathError::Namespace);
    }
    if path.starts_with("//") {
        return Err(PathError::Absolute);
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    if has_drive_prefix(path) {
        return Err(PathError::DriveQualified);
    }
    Ok(())
}

fn starts_with_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_component(component: &str, index: usize) -> Result<(), PathError> {
    if component.ends_with(['.', ' ']) {
        return Err(PathError::TrailingDotOrSpace { component: index });
    }
    if component.contains(':') {
        return Err(PathError::AlternateDataStream { component: index });
    }
    if component
        .chars()
        .any(|character| matches!(character, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return Err(PathError::InvalidCharacter { component: index });
    }
    // A capability backend cannot prove whether a tilde-bearing component is
    // a long name or a filesystem-generated 8.3 alias without a second,
    // platform-specific identity lookup. Reject it before any backend IO.
    if component.contains('~') {
        return Err(PathError::ShortNameAlias { component: index });
    }
    if is_reserved_device(component) {
        return Err(PathError::ReservedDevice { component: index });
    }
    if index == 0 && component.eq_ignore_ascii_case("GLOBALROOT") {
        return Err(PathError::Namespace);
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
    matches!(
        suffix,
        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
    ) || matches!(suffix, "\u{00b9}" | "\u{00b2}" | "\u{00b3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_relative_components_without_host_path_rules() {
        let path = AssetPath::new("models/./shared/../cube.obj").expect("valid path");
        assert_eq!(path.as_str(), "models/cube.obj");

        let resolved = path
            .resolve_from_file("materials/../cube.mtl")
            .expect("valid relative path");
        assert_eq!(resolved.as_str(), "models/cube.mtl");
        assert_eq!(path.try_clone().expect("fallible clone"), path);
    }

    #[test]
    fn rejects_escape_absolute_and_ambiguous_components() {
        for candidate in [
            "../secret",
            "/secret",
            "//server/share",
            "C:secret",
            "C:/secret",
            "folder//file",
            "file:stream",
            "folder/file. ",
            "folder/fi?le",
            "folder/PROGRA~1/file.bin",
        ] {
            assert!(
                AssetPath::new(candidate).is_err(),
                "{candidate:?} unexpectedly passed"
            );
        }
    }

    #[test]
    fn rejects_namespaces_and_reserved_devices_on_every_host() {
        for candidate in [
            r"\\?\C:\secret",
            r"\\.\NUL",
            r"\??\C:\secret",
            r"\Device\HarddiskVolume1\secret",
            "GLOBALROOT/device",
            "con",
            "NUL.txt",
            "com0.bin",
            "COM9",
            "LPT1.log",
            "CON .txt",
            "com1 .bin",
            "com\u{00b9}.txt",
            "PROGRA~1/file.bin",
        ] {
            assert!(
                AssetPath::new(candidate).is_err(),
                "{candidate:?} unexpectedly passed"
            );
        }
    }

    #[test]
    fn rejects_control_characters_and_hard_limit() {
        assert!(matches!(
            AssetPath::new("folder/line\nfeed"),
            Err(PathError::ControlCharacter { .. })
        ));
        let overlong = "a".repeat(AssetPath::MAX_BYTES + 1);
        assert!(matches!(
            AssetPath::new(overlong),
            Err(PathError::TooLong { .. })
        ));
    }
}
