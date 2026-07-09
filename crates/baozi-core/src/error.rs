use crate::diagnostic::SourceLocation;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, BaoziError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaoziErrorKind {
    Io,
    UnsupportedFormat,
    DuplicateFormatId,
    AmbiguousFormat,
    Parse,
    InvalidScene,
    PostProcess,
    FeatureUnsupported,
    LimitExceeded,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum BaoziError {
    #[error("io error while reading {asset}: {message}")]
    Io { asset: String, message: String },

    #[error("unsupported format: {hint}")]
    UnsupportedFormat { hint: String },

    #[error("duplicate format id: {id}")]
    DuplicateFormatId { id: String },

    #[error("ambiguous format for {hint}: {}", candidates.join(", "))]
    AmbiguousFormat {
        hint: String,
        candidates: Vec<String>,
    },

    #[error("parse error in {asset}{location}: {message}")]
    Parse {
        asset: String,
        location: SourceLocationDisplay,
        message: String,
    },

    #[error("invalid scene: {message}")]
    InvalidScene { message: String },

    #[error("postprocess {step} failed: {message}")]
    PostProcess { step: &'static str, message: String },

    #[error("feature unsupported by {format}: {feature}")]
    FeatureUnsupported {
        format: &'static str,
        feature: String,
    },

    #[error("configured limit exceeded: {limit}")]
    LimitExceeded { limit: &'static str },
}

impl BaoziError {
    pub const fn kind(&self) -> BaoziErrorKind {
        match self {
            Self::Io { .. } => BaoziErrorKind::Io,
            Self::UnsupportedFormat { .. } => BaoziErrorKind::UnsupportedFormat,
            Self::DuplicateFormatId { .. } => BaoziErrorKind::DuplicateFormatId,
            Self::AmbiguousFormat { .. } => BaoziErrorKind::AmbiguousFormat,
            Self::Parse { .. } => BaoziErrorKind::Parse,
            Self::InvalidScene { .. } => BaoziErrorKind::InvalidScene,
            Self::PostProcess { .. } => BaoziErrorKind::PostProcess,
            Self::FeatureUnsupported { .. } => BaoziErrorKind::FeatureUnsupported,
            Self::LimitExceeded { .. } => BaoziErrorKind::LimitExceeded,
        }
    }

    pub fn io(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Io {
            asset: source.into(),
            message: message.into(),
        }
    }

    pub fn unsupported_format(hint: impl Into<String>) -> Self {
        Self::UnsupportedFormat { hint: hint.into() }
    }

    pub fn duplicate_format_id(id: impl Into<String>) -> Self {
        Self::DuplicateFormatId { id: id.into() }
    }

    pub fn ambiguous_format(
        hint: impl Into<String>,
        candidates: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::AmbiguousFormat {
            hint: hint.into(),
            candidates: candidates.into_iter().map(Into::into).collect(),
        }
    }

    pub fn parse(
        source: impl Into<String>,
        location: Option<SourceLocation>,
        message: impl Into<String>,
    ) -> Self {
        Self::Parse {
            asset: source.into(),
            location: SourceLocationDisplay(location),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_variants_have_machine_readable_kinds() {
        let cases = [
            (BaoziError::io("asset", "denied"), BaoziErrorKind::Io),
            (
                BaoziError::unsupported_format("foo"),
                BaoziErrorKind::UnsupportedFormat,
            ),
            (
                BaoziError::duplicate_format_id("obj"),
                BaoziErrorKind::DuplicateFormatId,
            ),
            (
                BaoziError::ambiguous_format("mesh", ["a", "b"]),
                BaoziErrorKind::AmbiguousFormat,
            ),
            (
                BaoziError::parse("mesh", None, "bad"),
                BaoziErrorKind::Parse,
            ),
            (
                BaoziError::InvalidScene {
                    message: "bad scene".to_owned(),
                },
                BaoziErrorKind::InvalidScene,
            ),
            (
                BaoziError::PostProcess {
                    step: "Triangulate",
                    message: "bad".to_owned(),
                },
                BaoziErrorKind::PostProcess,
            ),
            (
                BaoziError::FeatureUnsupported {
                    format: "obj",
                    feature: "curves".to_owned(),
                },
                BaoziErrorKind::FeatureUnsupported,
            ),
            (
                BaoziError::LimitExceeded {
                    limit: "max_vertices",
                },
                BaoziErrorKind::LimitExceeded,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.kind(), expected);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceLocationDisplay(pub Option<SourceLocation>);

impl std::fmt::Display for SourceLocationDisplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(location) => write!(formatter, " at {location}"),
            None => Ok(()),
        }
    }
}
