use crate::namespaced::{NamespacedName, NamespacedNameError};

/// The owner-controlled cardinality domain of an attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeDomain {
    Vertex,
    Corner,
    Face,
    Instance,
}

/// A validated namespaced custom attribute semantic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomAttributeSemantic {
    name: NamespacedName,
}

impl CustomAttributeSemantic {
    pub fn new(value: impl AsRef<str>) -> Result<Self, NamespacedNameError> {
        Ok(Self {
            name: NamespacedName::new(value)?,
        })
    }

    pub fn from_name(name: NamespacedName) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &NamespacedName {
        &self.name
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }
}

/// Canonical built-in and extension attribute semantics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeSemantic {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(CustomAttributeSemantic),
}

impl AttributeSemantic {
    pub fn kind(&self) -> AttributeSemanticKind {
        match self {
            Self::Position => AttributeSemanticKind::Position,
            Self::Normal => AttributeSemanticKind::Normal,
            Self::Tangent => AttributeSemanticKind::Tangent,
            Self::Texcoord(_) => AttributeSemanticKind::Texcoord,
            Self::Color(_) => AttributeSemanticKind::Color,
            Self::SmoothingGroup => AttributeSemanticKind::SmoothingGroup,
            Self::Custom(_) => AttributeSemanticKind::Custom,
        }
    }
}

/// A text-free semantic category suitable for diagnostics and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeSemanticKind {
    Position,
    Normal,
    Tangent,
    Texcoord,
    Color,
    SmoothingGroup,
    Custom,
}

#[cfg(test)]
mod tests {
    use super::{AttributeSemantic, AttributeSemanticKind, CustomAttributeSemantic};

    #[test]
    fn custom_semantics_preserve_validated_names() {
        let semantic = CustomAttributeSemantic::new("ply:temperature").unwrap();

        assert_eq!(semantic.as_str(), "ply:temperature");
        assert_eq!(
            AttributeSemantic::Custom(semantic).kind(),
            AttributeSemanticKind::Custom
        );
    }

    #[test]
    fn custom_semantics_reject_unqualified_names() {
        assert!(CustomAttributeSemantic::new("temperature").is_err());
    }
}
