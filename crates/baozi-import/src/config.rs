use crate::limits::{ResourceLimits, ResourceLimitsOverlay};
use crate::options::{FormatOptionEnvelope, FormatOptionEnvelopeError};
use baozi_core::FormatId;
use std::cmp::Ordering;
use std::fmt;

pub const MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER: usize = 64;
pub const MAX_DETECTOR_WORK_BETWEEN_POLLS: u64 = 4096;

/// Bounded detection configuration frozen before import IO begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DetectionOptions {
    max_probe_bytes: u64,
}

impl DetectionOptions {
    pub fn new(max_probe_bytes: u64) -> Result<Self, DetectionOptionsError> {
        if max_probe_bytes > MAX_DETECTOR_WORK_BETWEEN_POLLS {
            return Err(DetectionOptionsError::ProbeLimitExceeded {
                requested: max_probe_bytes,
                maximum: MAX_DETECTOR_WORK_BETWEEN_POLLS,
            });
        }
        Ok(Self { max_probe_bytes })
    }

    pub const fn max_probe_bytes(self) -> u64 {
        self.max_probe_bytes
    }
}

impl Default for DetectionOptions {
    fn default() -> Self {
        Self {
            max_probe_bytes: MAX_DETECTOR_WORK_BETWEEN_POLLS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionOptionsError {
    ProbeLimitExceeded { requested: u64, maximum: u64 },
}

impl fmt::Display for DetectionOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProbeLimitExceeded { requested, maximum } => write!(
                formatter,
                "detector probe limit {requested} exceeds cancellation bound {maximum}"
            ),
        }
    }
}

impl std::error::Error for DetectionOptionsError {}

/// Format-selection policy validated against the frozen registry publication.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum FormatSelection {
    #[default]
    Automatic,
    Preferred(FormatId),
    Forced(FormatId),
}

/// Common source-conformance policy frozen before import IO begins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConformancePolicy {
    #[default]
    Strict,
    AllowDeclaredRecovery,
}

/// A stable category of source-semantic or retained-evidence loss that requires caller authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImportLossKind {
    OptionalDependencyUnavailable,
    FormatRecovery,
    SourceSemanticLoss,
    ProvenanceEvidenceLoss,
}

/// Common authority for accepting explicitly classified import loss.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LossPolicy {
    allowed: u8,
}

impl LossPolicy {
    const OPTIONAL_DEPENDENCY_UNAVAILABLE: u8 = 1 << 0;
    const FORMAT_RECOVERY: u8 = 1 << 1;
    const SOURCE_SEMANTIC_LOSS: u8 = 1 << 2;
    const PROVENANCE_EVIDENCE_LOSS: u8 = 1 << 3;

    pub const fn preserve_or_fail() -> Self {
        Self { allowed: 0 }
    }

    pub const fn allow(mut self, kind: ImportLossKind) -> Self {
        self.allowed |= loss_kind_bit(kind);
        self
    }

    pub const fn deny(mut self, kind: ImportLossKind) -> Self {
        self.allowed &= !loss_kind_bit(kind);
        self
    }

    pub const fn allows(self, kind: ImportLossKind) -> bool {
        self.allowed & loss_kind_bit(kind) != 0
    }
}

const fn loss_kind_bit(kind: ImportLossKind) -> u8 {
    match kind {
        ImportLossKind::OptionalDependencyUnavailable => {
            LossPolicy::OPTIONAL_DEPENDENCY_UNAVAILABLE
        }
        ImportLossKind::FormatRecovery => LossPolicy::FORMAT_RECOVERY,
        ImportLossKind::SourceSemanticLoss => LossPolicy::SOURCE_SEMANTIC_LOSS,
        ImportLossKind::ProvenanceEvidenceLoss => LossPolicy::PROVENANCE_EVIDENCE_LOSS,
    }
}

/// Explicit common-configuration values supplied at one precedence layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportConfigOverlay {
    pub resource_limits: ResourceLimitsOverlay,
    pub conformance_policy: Option<ConformancePolicy>,
    pub loss_policy: Option<LossPolicy>,
    pub detection_options: Option<DetectionOptions>,
    pub format_selection: Option<FormatSelection>,
}

impl ImportConfigOverlay {
    pub fn with_resource_limits(mut self, resource_limits: ResourceLimitsOverlay) -> Self {
        self.resource_limits = resource_limits;
        self
    }

    pub fn with_conformance_policy(mut self, policy: ConformancePolicy) -> Self {
        self.conformance_policy = Some(policy);
        self
    }

    pub fn with_loss_policy(mut self, policy: LossPolicy) -> Self {
        self.loss_policy = Some(policy);
        self
    }

    pub fn with_detection_options(mut self, options: DetectionOptions) -> Self {
        self.detection_options = Some(options);
        self
    }

    pub fn with_format_selection(mut self, selection: FormatSelection) -> Self {
        self.format_selection = Some(selection);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.resource_limits.is_empty()
            && self.conformance_policy.is_none()
            && self.loss_policy.is_none()
            && self.detection_options.is_none()
            && self.format_selection.is_none()
    }
}

/// Fully resolved common import configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportConfig {
    resource_limits: ResourceLimits,
    conformance_policy: ConformancePolicy,
    loss_policy: LossPolicy,
    detection_options: DetectionOptions,
    format_selection: FormatSelection,
}

impl ImportConfig {
    /// Resolves built-in defaults, then preset values, then caller values.
    pub fn resolve(preset: &ImportConfigOverlay, caller: &ImportConfigOverlay) -> Self {
        let mut resolved = Self::default();
        resolved.apply_overlay(preset);
        resolved.apply_overlay(caller);
        resolved
    }

    pub const fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    pub const fn conformance_policy(&self) -> ConformancePolicy {
        self.conformance_policy
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.loss_policy
    }

    pub const fn detection_options(&self) -> &DetectionOptions {
        &self.detection_options
    }

    pub const fn format_selection(&self) -> &FormatSelection {
        &self.format_selection
    }

    fn apply_overlay(&mut self, overlay: &ImportConfigOverlay) {
        self.resource_limits.apply_overlay(&overlay.resource_limits);
        if let Some(policy) = overlay.conformance_policy {
            self.conformance_policy = policy;
        }
        if let Some(policy) = overlay.loss_policy {
            self.loss_policy = policy;
        }
        if let Some(options) = overlay.detection_options {
            self.detection_options = options;
        }
        if let Some(selection) = &overlay.format_selection {
            self.format_selection = selection.clone();
        }
    }
}

/// One inspectable common/format overlay at a single precedence level.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportRequestLayer {
    config: ImportConfigOverlay,
    format_options: Vec<FormatOptionEnvelope>,
}

impl ImportRequestLayer {
    pub fn new(config: ImportConfigOverlay) -> Self {
        Self {
            config,
            format_options: Vec::new(),
        }
    }

    pub const fn config(&self) -> &ImportConfigOverlay {
        &self.config
    }

    pub fn format_options(&self) -> &[FormatOptionEnvelope] {
        &self.format_options
    }

    pub fn try_with_format_option(
        mut self,
        envelope: FormatOptionEnvelope,
    ) -> Result<Self, ImportRequestBuildError> {
        self.try_push_format_option(envelope)?;
        Ok(self)
    }

    pub fn try_push_format_option(
        &mut self,
        envelope: FormatOptionEnvelope,
    ) -> Result<(), ImportRequestBuildError> {
        if self
            .format_options
            .iter()
            .any(|existing| existing.target() == envelope.target())
        {
            return Err(ImportRequestBuildError::DuplicateFormatOption {
                target: envelope.target().clone(),
            });
        }
        if self.format_options.len() >= MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER {
            return Err(ImportRequestBuildError::FormatOptionEnvelopeCountExceeded {
                maximum: MAX_FORMAT_OPTION_ENVELOPES_PER_LAYER,
            });
        }
        self.format_options
            .try_reserve_exact(1)
            .map_err(|_| ImportRequestBuildError::AllocationFailed)?;
        self.format_options.push(envelope);
        self.format_options.sort_unstable_by(compare_envelopes);
        Ok(())
    }

    fn try_clone(&self) -> Result<Self, ImportRequestBuildError> {
        let mut format_options = Vec::new();
        format_options
            .try_reserve_exact(self.format_options.len())
            .map_err(|_| ImportRequestBuildError::AllocationFailed)?;
        for envelope in &self.format_options {
            format_options.push(try_clone_envelope(envelope)?);
        }
        Ok(Self {
            config: self.config.clone(),
            format_options,
        })
    }
}

/// A reusable, inspectable preset overlay.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportPreset {
    layer: ImportRequestLayer,
}

impl ImportPreset {
    pub fn new(config: ImportConfigOverlay) -> Self {
        Self {
            layer: ImportRequestLayer::new(config),
        }
    }

    pub const fn layer(&self) -> &ImportRequestLayer {
        &self.layer
    }

    pub fn try_with_format_option(
        mut self,
        envelope: FormatOptionEnvelope,
    ) -> Result<Self, ImportRequestBuildError> {
        self.layer.try_push_format_option(envelope)?;
        Ok(self)
    }

    pub fn try_expand(&self) -> Result<ImportRequestLayer, ImportRequestBuildError> {
        self.layer.try_clone()
    }
}

/// Uncompiled import request with separate preset and caller precedence layers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportRequest {
    preset: ImportRequestLayer,
    caller: ImportRequestLayer,
}

impl ImportRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_layers(preset: ImportRequestLayer, caller: ImportRequestLayer) -> Self {
        Self { preset, caller }
    }

    pub fn try_with_preset(
        mut self,
        preset: &ImportPreset,
    ) -> Result<Self, ImportRequestBuildError> {
        self.preset = preset.try_expand()?;
        Ok(self)
    }

    pub fn with_caller_layer(mut self, caller: ImportRequestLayer) -> Self {
        self.caller = caller;
        self
    }

    pub const fn preset(&self) -> &ImportRequestLayer {
        &self.preset
    }

    pub const fn caller(&self) -> &ImportRequestLayer {
        &self.caller
    }

    pub fn resolved_config(&self) -> ImportConfig {
        ImportConfig::resolve(self.preset.config(), self.caller.config())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportRequestBuildError {
    AllocationFailed,
    FormatOptionEnvelopeCountExceeded { maximum: usize },
    DuplicateFormatOption { target: baozi_core::FormatId },
    InvalidEnvelope { cause: FormatOptionEnvelopeError },
}

impl fmt::Display for ImportRequestBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("import request allocation failed"),
            Self::FormatOptionEnvelopeCountExceeded { maximum } => write!(
                formatter,
                "format option envelope count exceeds {maximum} in one request layer"
            ),
            Self::DuplicateFormatOption { target } => {
                write!(
                    formatter,
                    "duplicate format options for {target} in one request layer"
                )
            }
            Self::InvalidEnvelope { cause } => write!(formatter, "invalid format options: {cause}"),
        }
    }
}

impl std::error::Error for ImportRequestBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidEnvelope { cause } => Some(cause),
            _ => None,
        }
    }
}

fn compare_envelopes(left: &FormatOptionEnvelope, right: &FormatOptionEnvelope) -> Ordering {
    left.target()
        .cmp(right.target())
        .then_with(|| left.schema().owner().cmp(right.schema().owner()))
        .then_with(|| left.schema().local().cmp(right.schema().local()))
        .then_with(|| left.version().cmp(&right.version()))
}

fn try_clone_envelope(
    envelope: &FormatOptionEnvelope,
) -> Result<FormatOptionEnvelope, ImportRequestBuildError> {
    FormatOptionEnvelope::new(
        envelope.target().clone(),
        envelope.schema().clone(),
        envelope.version(),
        envelope.canonical_payload_bytes(),
    )
    .map_err(|cause| ImportRequestBuildError::InvalidEnvelope { cause })
}
