use crate::{DetectionEvidence, DetectionFailure, DetectionOutcome};
use baozi_core::BaoziError;
use std::fmt;

pub type ImportResult<T> = std::result::Result<T, ImportFailure>;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ImportFailure {
    UnsupportedFormat {
        hint: String,
        evidence: Vec<DetectionEvidence>,
    },
    Detection(DetectionFailure),
    Operation {
        error: BaoziError,
        evidence: Vec<DetectionEvidence>,
    },
}

impl ImportFailure {
    pub fn from_detection(failure: DetectionFailure) -> Self {
        match failure {
            DetectionFailure::NoMatch { hint, evidence }
                if evidence
                    .iter()
                    .all(|item| item.outcome() == &DetectionOutcome::NoMatch) =>
            {
                Self::UnsupportedFormat { hint, evidence }
            }
            failure => Self::Detection(failure),
        }
    }

    pub fn operation(error: BaoziError, evidence: Vec<DetectionEvidence>) -> Self {
        Self::Operation { error, evidence }
    }

    pub fn evidence(&self) -> &[DetectionEvidence] {
        match self {
            Self::UnsupportedFormat { evidence, .. } | Self::Operation { evidence, .. } => evidence,
            Self::Detection(DetectionFailure::NoMatch { evidence, .. })
            | Self::Detection(DetectionFailure::Ambiguous { evidence, .. }) => evidence,
            Self::Detection(DetectionFailure::PrimaryFault { .. }) => &[],
        }
    }

    pub fn operation_error(&self) -> Option<&BaoziError> {
        match self {
            Self::Operation { error, .. } => Some(error),
            Self::UnsupportedFormat { .. } | Self::Detection(_) => None,
        }
    }
}

impl From<BaoziError> for ImportFailure {
    fn from(error: BaoziError) -> Self {
        Self::operation(error, Vec::new())
    }
}

impl From<DetectionFailure> for ImportFailure {
    fn from(failure: DetectionFailure) -> Self {
        Self::from_detection(failure)
    }
}

impl fmt::Display for ImportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat { hint, .. } => {
                write!(formatter, "unsupported format: {hint}")
            }
            Self::Detection(failure) => failure.fmt(formatter),
            Self::Operation { error, .. } => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedFormat { .. } => None,
            Self::Detection(failure) => Some(failure),
            Self::Operation { error, .. } => Some(error),
        }
    }
}
