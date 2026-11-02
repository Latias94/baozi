use crate::config::{FormatSelection, ImportConfig, ImportRequest};
use crate::dependency::DependencyResolver;
use crate::format::BoundedFaultDetail;
use crate::ledger::ImportOperationControl;
use crate::options::{
    CompileAggregate, FormatOptionEnvelope, FormatOptionLayers, FormatOptionSchemaId,
    OptionCompileError, OptionCompileLimits, OptionCompileScope,
};
use crate::outcome::{ImportReport, ImportResult};
use crate::registry::{
    FormatUnavailability, PreparedInvocation, RegistrationInvocationError, RegistryEntry,
    RegistrySnapshot,
};
use baozi_core::FormatId;
use baozi_io::{AssetIo, AssetPath};
use baozi_operation::{
    PhysicalStorageAccount, PhysicalStorageAccountCreateError, PhysicalStorageAccountFactory,
    PhysicalStorageAccountLimits,
};
use std::collections::{HashMap, hash_map::Entry};
use std::fmt;
use std::hash::Hash;
use std::num::{NonZeroU32, NonZeroU64};

const MAX_LIVE_STORAGE_ACCOUNTS_PER_PLAN: u64 = 1024;

/// Immutable, snapshot-bound result of complete pre-session request compilation.
///
/// A plan owns its registry and can execute after the builder or facade that created it is
/// dropped. Clones share compiled invocations and the live storage-account limit.
#[derive(Clone)]
pub struct ImportPlan {
    snapshot: RegistrySnapshot,
    config: ImportConfig,
    overrides: HashMap<usize, PreparedInvocation>,
    override_order: Vec<usize>,
    account_factory: PhysicalStorageAccountFactory,
}

impl ImportPlan {
    pub fn snapshot(&self) -> &RegistrySnapshot {
        &self.snapshot
    }

    pub const fn config(&self) -> &ImportConfig {
        &self.config
    }

    pub const fn limits(&self) -> &crate::limits::ResourceLimits {
        self.config.resource_limits()
    }

    /// Imports an immutable caller-owned byte slice without secondary asset authority.
    #[allow(
        clippy::result_large_err,
        reason = "terminal import failures retain complete bounded session evidence without heap indirection"
    )]
    pub fn read_bytes(
        &self,
        source: AssetPath,
        bytes: &[u8],
        control: ImportOperationControl,
    ) -> ImportResult<ImportReport> {
        crate::session::import_bytes(self, source, bytes, control)
    }

    /// Imports an asset using the primary authority for dependencies.
    #[allow(
        clippy::result_large_err,
        reason = "terminal import failures retain complete bounded session evidence without heap indirection"
    )]
    pub fn read_asset(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        control: ImportOperationControl,
    ) -> ImportResult<ImportReport> {
        crate::session::import_asset(self, io, source, control)
    }

    /// Imports an asset using an explicit dependency authority policy.
    #[allow(
        clippy::result_large_err,
        reason = "terminal import failures retain complete bounded session evidence without heap indirection"
    )]
    pub fn read_asset_with_resolver(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        resolver: &dyn DependencyResolver,
        control: ImportOperationControl,
    ) -> ImportResult<ImportReport> {
        crate::session::import_asset_with_resolver(self, io, source, resolver, control)
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

    pub(crate) fn invocation_for(&self, entry: &RegistryEntry) -> Option<PreparedInvocation> {
        if !entry.belongs_to(&self.snapshot) {
            return None;
        }
        Some(
            self.overrides
                .get(&entry.index())
                .cloned()
                .unwrap_or_else(|| entry.default_invocation()),
        )
    }

    pub(crate) fn try_create_storage_account(
        &self,
    ) -> Result<PhysicalStorageAccount, PhysicalStorageAccountCreateError> {
        let max_retained_bytes = self.limits().max_live_bytes.max(1);
        let limits = PhysicalStorageAccountLimits::new(max_retained_bytes, max_retained_bytes)
            .expect("the import storage account limits are nonzero by construction");
        self.account_factory.try_create_account(limits)
    }
}

impl fmt::Debug for ImportPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportPlan")
            .field("snapshot", &self.snapshot)
            .field("config", &self.config)
            .field("overrides", &OverriddenFormatsDebug(self))
            .finish()
    }
}

struct OverriddenFormatsDebug<'a>(&'a ImportPlan);

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
    pub fn plan(
        &self,
        request: &ImportRequest,
        limits: OptionCompileLimits,
    ) -> Result<ImportPlan, RequestError> {
        self.plan_layers(
            request.resolved_config(),
            request.preset().format_options(),
            request.caller().format_options(),
            limits,
        )
    }

    fn plan_layers(
        &self,
        config: ImportConfig,
        preset: &[FormatOptionEnvelope],
        caller: &[FormatOptionEnvelope],
        limits: OptionCompileLimits,
    ) -> Result<ImportPlan, RequestError> {
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
            let compiled = match target.entry.compile_overrides(&mut scope, target.layers) {
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
            let index = target.entry.index();
            let replaced = overrides.insert(index, compiled);
            debug_assert!(replaced.is_none());
            override_order.push(index);
        }

        let account_factory = PhysicalStorageAccountFactory::try_new_for_live_accounts(
            NonZeroU64::new(MAX_LIVE_STORAGE_ACCOUNTS_PER_PLAN)
                .expect("the import-plan account limit is nonzero"),
        )
        .map_err(|_| RequestError::AllocationFailed)?;

        Ok(ImportPlan {
            snapshot: self.clone(),
            config,
            overrides,
            override_order,
            account_factory,
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
}

const _: () = assert!(std::mem::size_of::<RequestError>() < 128);

impl RequestError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AllocationFailed => "allocation-failed",
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
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => {
                formatter.write_str("format option request allocation failed")
            }
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
    use crate::config::{ImportConfigOverlay, ImportRequestLayer};
    use crate::descriptor::{FormatDescriptor, FormatEncoding};
    use crate::format::Format;
    use crate::options::{
        CompiledOptions, FormatOptionSchema, FormatOptionSchemaKey, RegistryBuildLimits,
        RegistryBuildScope,
    };
    use crate::registry::RegistryBuilder;
    use crate::scope::{ParseContext, ParseResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CompilationLog {
        defaults: AtomicUsize,
        overrides: Mutex<Vec<(FormatId, u8)>>,
    }

    struct CountingFormat {
        schema: FormatOptionSchema,
        log: Arc<CompilationLog>,
    }

    impl Format for CountingFormat {
        type Options = u8;

        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(self.schema.key().owner().clone(), "Counting format")
                .encoding(FormatEncoding::Binary)
                .build()
                .unwrap()
        }

        fn option_schema(&self) -> Option<FormatOptionSchema> {
            Some(self.schema.clone())
        }

        fn compile_defaults(
            &self,
            scope: &mut RegistryBuildScope<'_>,
        ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
            self.log.defaults.fetch_add(1, Ordering::SeqCst);
            scope.charge_work(1)?;
            scope.finish(7, 1)
        }

        fn compile_options(
            &self,
            scope: &mut OptionCompileScope<'_>,
            defaults: &Self::Options,
            layers: FormatOptionLayers<'_>,
        ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
            assert_eq!(*defaults, 7);
            let mut value = *defaults;
            for payload in [layers.preset(), layers.caller()].into_iter().flatten() {
                let bytes = scope.reader(payload).read_remaining()?;
                let [submitted] = bytes else {
                    return Err(OptionCompileError::invalid_payload(
                        "one-byte",
                        "expected exactly one byte",
                    ));
                };
                value = *submitted;
            }
            let compiled = scope.finish(value, 1)?;
            self.log
                .overrides
                .lock()
                .unwrap()
                .push((scope.format().clone(), *compiled.value()));
            Ok(compiled)
        }

        fn parse(
            &self,
            _options: &Self::Options,
            _context: &mut ParseContext<'_>,
        ) -> ParseResult<()> {
            unreachable!("request compilation tests do not parse")
        }
    }

    fn counting_schema(format: &str) -> FormatOptionSchema {
        FormatOptionSchema::new(
            FormatOptionSchemaKey::new(
                FormatId::new(format).unwrap(),
                FormatOptionSchemaId::new("import-options").unwrap(),
            ),
            NonZeroU32::new(1).unwrap(),
        )
    }

    fn counting_snapshot(
        schemas: &[FormatOptionSchema],
        log: &Arc<CompilationLog>,
    ) -> RegistrySnapshot {
        let mut builder = RegistryBuilder::new();
        for schema in schemas {
            builder
                .register(CountingFormat {
                    schema: schema.clone(),
                    log: Arc::clone(log),
                })
                .unwrap();
        }
        builder.freeze(RegistryBuildLimits::default()).unwrap()
    }

    #[test]
    fn later_invalid_metadata_prevents_every_target_compiler() {
        let first = counting_schema("org.example.first");
        let last = counting_schema("org.example.last");
        let log = Arc::new(CompilationLog::default());
        let snapshot = counting_snapshot(&[last.clone(), first.clone()], &log);
        let caller = ImportRequestLayer::default()
            .try_with_format_option(first.envelope_from_canonical_payload([11]).unwrap())
            .unwrap()
            .try_with_format_option(
                FormatOptionEnvelope::new(
                    last.key().owner().clone(),
                    last.key().clone(),
                    NonZeroU32::new(2).unwrap(),
                    [29],
                )
                .unwrap(),
            )
            .unwrap();
        let request = ImportRequest::new().with_caller_layer(caller);

        assert!(matches!(
            snapshot.plan(&request, OptionCompileLimits::default()),
            Err(RequestError::UnsupportedVersion { format, .. })
                if &format == last.key().owner()
        ));
        assert!(log.overrides.lock().unwrap().is_empty());
    }

    #[test]
    fn plans_reuse_snapshot_defaults_and_clones_reuse_compiled_overrides() {
        let schema = counting_schema("org.example.cached");
        let log = Arc::new(CompilationLog::default());
        let snapshot = counting_snapshot(std::slice::from_ref(&schema), &log);
        for _ in 0..2 {
            let plan = snapshot
                .plan(&ImportRequest::new(), OptionCompileLimits::default())
                .unwrap();
            let cloned = plan.clone();
            assert_eq!(cloned.overridden_formats().len(), 0);
        }
        let caller = ImportRequestLayer::default()
            .try_with_format_option(schema.envelope_from_canonical_payload([11]).unwrap())
            .unwrap();
        let plan = snapshot
            .plan(
                &ImportRequest::new().with_caller_layer(caller),
                OptionCompileLimits::default(),
            )
            .unwrap();
        let cloned = plan.clone();

        assert_eq!(cloned.overridden_formats().len(), 1);
        assert_eq!(log.defaults.load(Ordering::SeqCst), 1);
        assert_eq!(
            log.overrides.lock().unwrap().as_slice(),
            &[(schema.key().owner().clone(), 11)]
        );
    }

    #[test]
    fn cloned_plans_share_the_live_account_limit_and_released_capacity() {
        let schema = counting_schema("org.example.account-limit");
        let log = Arc::new(CompilationLog::default());
        let snapshot = counting_snapshot(&[schema], &log);
        let plan = snapshot
            .plan(&ImportRequest::new(), OptionCompileLimits::default())
            .unwrap();
        let cloned = plan.clone();
        let mut accounts = Vec::new();
        for index in 0..MAX_LIVE_STORAGE_ACCOUNTS_PER_PLAN {
            let selected = if index % 2 == 0 { &plan } else { &cloned };
            accounts.push(selected.try_create_storage_account().unwrap());
        }
        let full = plan.account_factory.snapshot();
        assert_eq!(full.live_accounts(), MAX_LIVE_STORAGE_ACCOUNTS_PER_PLAN);
        for selected in [&plan, &cloned, &cloned.clone()] {
            assert!(matches!(
                selected.try_create_storage_account(),
                Err(PhysicalStorageAccountCreateError::LiveAccountLimitExceeded { requested, limit }
                    | PhysicalStorageAccountCreateError::ControlByteLimitExceeded { requested, limit })
                    if requested > limit
            ));
            assert_eq!(selected.account_factory.snapshot(), full);
        }
        drop(accounts.pop().unwrap());
        let replacement = cloned.try_create_storage_account().unwrap();
        assert!(plan.try_create_storage_account().is_err());
        drop(replacement);
        drop(accounts);
        assert!(plan.try_create_storage_account().is_ok());
    }

    #[test]
    fn plan_compilation_preserves_layer_presence_and_stable_target_order() {
        let first = counting_schema("org.example.first");
        let last = counting_schema("org.example.last");
        let log = Arc::new(CompilationLog::default());
        let snapshot = counting_snapshot(&[last.clone(), first.clone()], &log);
        let preset = ImportRequestLayer::new(ImportConfigOverlay::default())
            .try_with_format_option(last.envelope_from_canonical_payload([9]).unwrap())
            .unwrap()
            .try_with_format_option(first.envelope_from_canonical_payload([11]).unwrap())
            .unwrap();
        let caller = ImportRequestLayer::default()
            .try_with_format_option(first.envelope_from_canonical_payload([7]).unwrap())
            .unwrap();
        let plan = snapshot
            .plan(
                &ImportRequest::from_layers(preset, caller),
                OptionCompileLimits::default(),
            )
            .unwrap();

        assert_eq!(
            plan.overridden_formats().collect::<Vec<_>>(),
            [first.key().owner(), last.key().owner()]
        );
        assert_eq!(
            log.overrides.lock().unwrap().as_slice(),
            &[
                (first.key().owner().clone(), 7),
                (last.key().owner().clone(), 9)
            ]
        );
    }

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
