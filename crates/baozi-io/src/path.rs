use baozi_core::{BaoziError, Result};
use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetUri(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetPath(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetScope {
    root: PathBuf,
}

impl AssetUri {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AssetPath {
    pub fn new(path: impl AsRef<str>) -> Result<Self> {
        let path = normalize_logical(path.as_ref())?;
        Ok(Self(path))
    }

    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn join(&self, relative: &str) -> Result<Self> {
        let base = if self.0.is_empty() {
            String::new()
        } else if let Some((prefix, _)) = self.0.rsplit_once('/') {
            prefix.to_owned()
        } else {
            String::new()
        };

        let joined = if base.is_empty() {
            relative.to_owned()
        } else {
            format!("{base}/{relative}")
        };
        Self::new(joined)
    }
}

impl fmt::Display for AssetPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AssetScope {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn to_filesystem_path(&self, asset_path: &AssetPath) -> PathBuf {
        asset_path
            .as_str()
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part))
    }
}

fn normalize_logical(path: &str) -> Result<String> {
    let normalized = path.replace('\\', "/");
    let as_path = Path::new(&normalized);
    if as_path.is_absolute() {
        return Err(BaoziError::io(path, "absolute asset paths are not allowed"));
    }

    let mut parts = Vec::new();
    for component in as_path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| BaoziError::io(path, "asset path is not valid UTF-8"))?;
                parts.push(part.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(BaoziError::io(path, "asset path escapes its scope"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(BaoziError::io(path, "asset path escapes its scope"));
            }
        }
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_scope_escape() {
        assert!(AssetPath::new("../secret").is_err());
    }

    #[test]
    fn resolves_sibling() {
        let base = AssetPath::new("models/cube.obj").unwrap();
        assert_eq!(base.join("cube.mtl").unwrap().as_str(), "models/cube.mtl");
    }
}
