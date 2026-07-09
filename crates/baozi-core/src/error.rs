use crate::diagnostic::SourceLocation;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, BaoziError>;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum BaoziError {
    #[error("io error while reading {asset}: {message}")]
    Io { asset: String, message: String },

    #[error("unsupported format: {hint}")]
    UnsupportedFormat { hint: String },

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
    pub fn io(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Io {
            asset: source.into(),
            message: message.into(),
        }
    }

    pub fn unsupported_format(hint: impl Into<String>) -> Self {
        Self::UnsupportedFormat { hint: hint.into() }
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
