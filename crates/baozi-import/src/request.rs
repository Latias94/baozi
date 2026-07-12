use crate::config::{FormatSelection, ImportConfig, ImportRequest};
use crate::format::BoundedFaultDetail;
use crate::options::{
    CompileAggregate, CompiledFormatOptions, FormatOptionEnvelope, FormatOptionLayers,
    FormatOptionSchemaId, OptionCompileError, OptionCompileLimits, OptionCompileScope,
};
use crate::registry::{
    FormatUnavailability, RegistrationInvocationError, RegistryEntry, RegistrySnapshot,
};
use baozi_core::FormatId;
use std::collections::{HashMap, hash_map::Entry};
use std::fmt;
use std::hash::Hash;
use std::num::NonZeroU32;
use std::sync::Arc;

/// Immutable, snapshot-bound result of complete pre-session request compilation.
#[derive(Clone)]
pub struct PreparedImportRequest {
    snapshot: RegistrySnapshot,
    config: ImportConfig,
    overrides: HashMap<usize, Arc<CompiledFormatOptions>>,
    override_order: Vec<usize>,
}

impl PreparedImportRequest {
    pub fn snapshot(&self) -> &RegistrySnapshot {
        &self.snapshot
    }

    pub const fn config(&self) -> &ImportConfig {
        &self.config
    }

    pub const fn limits(&self) -> &crate::limits::ResourceLimits {
        self.config.resource_limits()
    }

    /// Returns whether this request was compiled against `snapshot`'s publication.
    pub fn belongs_to(&self, snapshot: &RegistrySnapshot) -> bool {
        self.snapshot.has_same_identity(snapshot)
    }

    /// Returns overridden formats in stable `FormatId` order.
    pub fn overridden_formats(&self) -> impl ExactSizeIterator<Item = &FormatId> {
        self.override_order
            .iter()
            .map(|index| self.snapshot.format_id_at(*index))
    }

    pub(crate) fn entry(&self, id: &FormatId) -> Option<RegistryEntry> {
        self.snapshot.entry(id)
    }

    pub(crate) fn options_for(&self, entry: &RegistryEntry) -> Option<Arc<CompiledFormatOptions>> {
        if !entry.belongs_to(&self.snapshot) {
            return None;
        }
        Some(
            self.overrides
                .get(&entry.index())
                .cloned()
                .unwrap_or_else(|| entry.default_options()),
        )
    }
}

impl fmt::Debug for PreparedImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedImportRequest")
            .field("snapshot", &self.snapshot)
            .field("config", &self.config)
            .field("overrides", &OverriddenFormatsDebug(self))
            .finish()
    }
}

struct OverriddenFormatsDebug<'a>(&'a PreparedImportRequest);

impl fmt::Debug for OverriddenFormatsDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.overridden_formats())
            .finish()
    }
}

#[derive(Clone, Copy, Default)]
struct SubmittedLayers<'a> {
    preset: Option<&'a FormatOptionEnvelope>,
    caller: Option<&'a FormatOptionEnvelope>,
}

#[derive(Clone, Copy)]
enum SubmittedLayer {
    Preset,
    Caller,
}

struct ValidatedTarget<'a> {
    entry: RegistryEntry,
    layers: FormatOptionLayers<'a>,
}

impl RegistrySnapshot {
    /// Resolves common configuration and compiles every submitted format target before session or IO.
    pub fn compile_request(
        &self,
        request: &ImportRequest,
        limits: OptionCompileLimits,
    ) -> Result<PreparedImportRequest, RequestError> {
        self.compile_request_layers(
            request.resolved_config(),
            request.preset().format_options(),
            request.caller().format_options(),
            limits,
        )
    }

    fn compile_request_layers(
        &self,
        config: ImportConfig,
        preset: &[FormatOptionEnvelope],
        caller: &[FormatOptionEnvelope],
        limits: OptionCompileLimits,
    ) -> Result<PreparedImportRequest, RequestError> {
        self.validate_format_selection(config.format_selection())?;
        let envelope_count =
            preset
                .len()
                .checked_add(caller.len())
                .ok_or(RequestError::EnvelopeCountExceeded {
                    maximum: limits.max_envelopes(),
                })?;
        if envelope_count > limits.max_envelopes() {
            return Err(RequestError::EnvelopeCountExceeded {
                maximum: limits.max_envelopes(),
            });
        }
        let total_payload_bytes = preset
            .iter()
            .chain(caller)
            .try_fold(0_u64, |total, envelope| {
                let length = u64::try_from(envelope.payload_len()).ok()?;
                total.checked_add(length)
            })
            .ok_or(RequestError::PayloadBytesExceeded {
                maximum: limits.max_total_payload_bytes(),
            })?;
        if total_payload_bytes > limits.max_total_payload_bytes() {
            return Err(RequestError::PayloadBytesExceeded {
                maximum: limits.max_total_payload_bytes(),
            });
        }

        let mut grouped = HashMap::new();
        reserve_map(&mut grouped, envelope_count)?;
        let mut targets = Vec::new();
        reserve_vec(&mut targets, envelope_count)?;
        group_layer(preset, SubmittedLayer::Preset, &mut grouped, &mut targets);
        group_layer(caller, SubmittedLayer::Caller, &mut grouped, &mut targets);
        targets.sort_unstable();

        let mut work = Vec::new();
        reserve_vec(&mut work, targets.len())?;
        for target in targets {
            let submitted = *grouped
                .get(target)
                .expect("target order is populated with the grouping map");
            let preset_envelope = submitted.preset;
            let caller_envelope = submitted.caller;
            for envelope in [preset_envelope, caller_envelope].into_iter().flatten() {
                if envelope.schema().owner() != envelope.target() {
                    return Err(RequestError::WrongSchemaOwner {
                        target: envelope.target().clone(),
                    });
                }
            }

            let Some(entry) = self.entry(target) else {
                return match self.unavailable_reason(target) {
                    Some(reason) => Err(RequestError::FormatUnavailable {
                        format: target.clone(),
                        reason,
                    }),
                    None => Err(RequestError::UnknownFormat {
                        format: target.clone(),
                    }),
                };
            };
            if entry.is_poisoned() {
                return Err(RequestError::ExtensionPoisoned {
                    format: target.clone(),
                });
            }
            let Some(schema) = entry.option_schema() else {
                return Err(RequestError::OptionsUnsupported {
                    format: target.clone(),
                });
            };

            for envelope in [preset_envelope, caller_envelope].into_iter().flatten() {
                if envelope.schema() != schema.key() {
                    return Err(RequestError::UnknownSchema {
                        schema: envelope.schema().local().clone(),
                    });
                }
                if envelope.version() != schema.version() {
                    return Err(RequestError::UnsupportedVersion {
                        format: target.clone(),
                        expected: schema.version(),
                        actual: envelope.version(),
                    });
                }
            }

            work.push(ValidatedTarget {
                entry,
                layers: FormatOptionLayers::new(
                    preset_envelope.map(FormatOptionEnvelope::payload),
                    caller_envelope.map(FormatOptionEnvelope::payload),
                ),
            });
        }
        drop(grouped);

        let mut overrides = HashMap::new();
        reserve_map(&mut overrides, work.len())?;
        let mut override_order = Vec::new();
        reserve_vec(&mut override_order, work.len())?;
        let mut aggregate = CompileAggregate::default();
        for target in work {
            let format = target.entry.format_id();
            let mut scope = OptionCompileScope::new(format, limits, &mut aggregate);
            let compiled = match target.entry.compile_layers(&mut scope, target.layers) {
                Ok(Ok(compiled)) => compiled,
                Ok(Err(cause)) => {
                    return Err(RequestError::CodecRejected {
                        format: format.clone(),
                        cause,
                    });
                }
                Err(RegistrationInvocationError::ExtensionPanicked { detail }) => {
                    return Err(RequestError::CodecPanicked {
                        format: format.clone(),
                        detail,
                    });
                }
                Err(RegistrationInvocationError::ExtensionPoisoned) => {
                    return Err(RequestError::ExtensionPoisoned {
                        format: format.clone(),
                    });
                }
            };
            if compiled.runtime_type_key() != target.entry.expected_runtime_type_key() {
                return Err(RequestError::RuntimeTypeMismatch {
                    format: format.clone(),
                });
            }
            let index = target.entry.index();
            let replaced = overrides.insert(index, Arc::new(compiled));
            debug_assert!(replaced.is_none());
            override_order.push(index);
        }

        Ok(PreparedImportRequest {
            snapshot: self.clone(),
            config,
            overrides,
            override_order,
        })
    }

    fn validate_format_selection(&self, selection: &FormatSelection) -> Result<(), RequestError> {
        let (FormatSelection::Preferred(format) | FormatSelection::Forced(format)) = selection
        else {
            return Ok(());
        };
        let Some(entry) = self.entry(format) else {
            return match self.unavailable_reason(format) {
                Some(reason) => Err(RequestError::FormatUnavailable {
                    format: format.clone(),
                    reason,
                }),
                None => Err(RequestError::UnknownFormat {
                    format: format.clone(),
                }),
            };
        };
        if entry.is_poisoned() {
            return Err(RequestError::ExtensionPoisoned {
                format: format.clone(),
            });
        }
        Ok(())
    }
}

fn group_layer<'a>(
    envelopes: &'a [FormatOptionEnvelope],
    layer: SubmittedLayer,
    grouped: &mut HashMap<&'a FormatId, SubmittedLayers<'a>>,
    targets: &mut Vec<&'a FormatId>,
) {
    for envelope in envelopes {
        let submitted = match grouped.entry(envelope.target()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                targets.push(envelope.target());
                entry.insert(SubmittedLayers::default())
            }
        };
        let slot = match layer {
            SubmittedLayer::Preset => &mut submitted.preset,
            SubmittedLayer::Caller => &mut submitted.caller,
        };
        let replaced = slot.replace(envelope);
        debug_assert!(replaced.is_none());
    }
}

fn reserve_map<K: Eq + Hash, V>(
    map: &mut HashMap<K, V>,
    additional: usize,
) -> Result<(), RequestError> {
    map.try_reserve(additional)
        .map_err(|_| RequestError::AllocationFailed)
}

fn reserve_vec<T>(values: &mut Vec<T>, additional: usize) -> Result<(), RequestError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| RequestError::AllocationFailed)
}

/// Typed failure produced before session creation and asset IO.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestError {
    AllocationFailed,
    RegistrySnapshotMismatch,
    EnvelopeCountExceeded {
        maximum: usize,
    },
    PayloadBytesExceeded {
        maximum: u64,
    },
    WrongSchemaOwner {
        target: FormatId,
    },
    UnknownFormat {
        format: FormatId,
    },
    FormatUnavailable {
        format: FormatId,
        reason: FormatUnavailability,
    },
    OptionsUnsupported {
        format: FormatId,
    },
    /// The submitted local schema after its owner was proven equal to the target.
    UnknownSchema {
        schema: FormatOptionSchemaId,
    },
    UnsupportedVersion {
        format: FormatId,
        expected: NonZeroU32,
        actual: NonZeroU32,
    },
    CodecRejected {
        format: FormatId,
        cause: OptionCompileError,
    },
    CodecPanicked {
        format: FormatId,
        detail: BoundedFaultDetail,
    },
    ExtensionPoisoned {
        format: FormatId,
    },
    RuntimeTypeMismatch {
        format: FormatId,
    },
}

const _: () = assert!(std::mem::size_of::<RequestError>() < 128);

impl RequestError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AllocationFailed => "allocation-failed",
            Self::RegistrySnapshotMismatch => "registry-snapshot-mismatch",
            Self::EnvelopeCountExceeded { .. } => "envelope-count-exceeded",
            Self::PayloadBytesExceeded { .. } => "payload-bytes-exceeded",
            Self::WrongSchemaOwner { .. } => "wrong-schema-owner",
            Self::UnknownFormat { .. } => "unknown-format",
            Self::FormatUnavailable { .. } => "format-unavailable",
            Self::OptionsUnsupported { .. } => "options-unsupported",
            Self::UnknownSchema { .. } => "unknown-schema",
            Self::UnsupportedVersion { .. } => "unsupported-version",
            Self::CodecRejected { .. } => "codec-rejected",
            Self::CodecPanicked { .. } => "codec-panicked",
            Self::ExtensionPoisoned { .. } => "extension-poisoned",
            Self::RuntimeTypeMismatch { .. } => "option-runtime-type-mismatch",
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => {
                formatter.write_str("format option request allocation failed")
            }
            Self::RegistrySnapshotMismatch => formatter
                .write_str("prepared import request belongs to a different registry snapshot"),
            Self::EnvelopeCountExceeded { maximum } => {
                write!(formatter, "format option envelope count exceeds {maximum}")
            }
            Self::PayloadBytesExceeded { maximum } => {
                write!(formatter, "format option payload bytes exceed {maximum}")
            }
            Self::WrongSchemaOwner { target } => write!(
                formatter,
                "format option schema owner does not match target {target}"
            ),
            Self::UnknownFormat { format } => write!(formatter, "unknown format {format}"),
            Self::FormatUnavailable { format, reason } => {
                write!(formatter, "format {format} is unavailable: {reason:?}")
            }
            Self::OptionsUnsupported { format } => {
                write!(formatter, "format {format} does not accept options")
            }
            Self::UnknownSchema { schema } => write!(formatter, "unknown option schema {schema}"),
            Self::UnsupportedVersion {
                format,
                expected,
                actual,
            } => write!(
                formatter,
                "unsupported option version {actual} for {format}; expected {expected}"
            ),
            Self::CodecRejected { format, cause } => {
                write!(formatter, "options for {format} were rejected: {cause}")
            }
            Self::CodecPanicked { format, detail } => {
                write!(formatter, "option codec for {format} panicked: {detail}")
            }
            Self::ExtensionPoisoned { format } => {
                write!(formatter, "format registration {format} is poisoned")
            }
            Self::RuntimeTypeMismatch { format } => {
                write!(
                    formatter,
                    "compiled option type does not match importer {format}"
                )
            }
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CodecRejected { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_capacity_overflow_is_a_typed_request_error() {
        let mut map = HashMap::<u8, u8>::new();
        assert!(matches!(
            reserve_map(&mut map, usize::MAX),
            Err(RequestError::AllocationFailed)
        ));

        let mut values = Vec::<u8>::new();
        assert!(matches!(
            reserve_vec(&mut values, usize::MAX),
            Err(RequestError::AllocationFailed)
        ));
    }
}
