use crate::descriptor::FormatDescriptor;
use crate::detection::{DetectionResult, DetectionScope};
use crate::format::{
    BoundedFaultDetail, DetectionOutcome, FormatImporter, Probe, bounded_panic_detail,
    detection_fault_from_panic, discard_untrusted_panic_payload,
};
use crate::options::{
    CompileAggregate, CompiledFormatOptions, FormatOptionCodec, FormatOptionLayers,
    FormatOptionSchema, OptionCompileError, OptionCompileScope, OptionRuntimeTypeKey,
    RegistryBuildLimits, RegistryBuildScope,
};
use crate::scope::{ParseResult, ParseScope};
use crate::session::ReadHint;
use baozi_core::{FormatId, SceneDraft};
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Why a known format is unavailable in this frozen registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatUnavailability {
    FeatureDisabled,
    UnsupportedTarget,
}

/// Mutable registration proposal. Imports consume only frozen snapshots.
#[derive(Default)]
pub struct RegistryBuilder {
    registrations: BTreeMap<FormatId, PendingRegistration>,
    unavailable: BTreeMap<Arc<FormatId>, FormatUnavailability>,
}

impl RegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a trusted importer with a qualified reverse-DNS ID.
    pub fn register<I>(&mut self, importer: I) -> Result<(), RegistryRegistrationError>
    where
        I: FormatImporter,
    {
        self.register_inner(Arc::new(importer))
    }

    /// Records a known but unavailable format without publishing a capability.
    pub fn declare_unavailable(
        &mut self,
        id: FormatId,
        reason: FormatUnavailability,
    ) -> Result<(), RegistryRegistrationError> {
        validate_qualified_format_id(&id)?;
        if self.registrations.contains_key(&id) || self.unavailable.contains_key(&id) {
            return Err(RegistryRegistrationError::DuplicateFormatId { id });
        }
        let replaced = self.unavailable.insert(Arc::new(id), reason);
        debug_assert!(replaced.is_none());
        Ok(())
    }

    /// Removes a registration or unavailable declaration from future snapshots.
    pub fn remove(&mut self, id: &FormatId) -> bool {
        self.registrations.remove(id).is_some() || self.unavailable.remove(id).is_some()
    }

    /// Atomically validates and freezes all registrations in stable ID order.
    pub fn freeze(
        &self,
        limits: RegistryBuildLimits,
    ) -> Result<RegistrySnapshot, RegistryBuildError> {
        let mut aggregate = CompileAggregate::default();
        let mut entries = Vec::new();
        reserve_registry_vec(&mut entries, self.registrations.len())?;
        let mut by_id = HashMap::new();
        reserve_registry_map(&mut by_id, self.registrations.len())?;
        let mut unavailable = HashMap::new();
        reserve_registry_map(&mut unavailable, self.unavailable.len())?;
        let mut unavailable_order = Vec::new();
        reserve_registry_vec(&mut unavailable_order, self.unavailable.len())?;
        for (id, reason) in &self.unavailable {
            let replaced = unavailable.insert(Arc::clone(id), *reason);
            debug_assert!(replaced.is_none());
            unavailable_order.push(Arc::clone(id));
        }

        for pending in self.registrations.values() {
            validate_pending(pending).map_err(RegistryBuildError::Registration)?;
            let format = pending.descriptor.id();
            if pending.health.is_poisoned() {
                return Err(RegistryBuildError::ExtensionPoisoned {
                    format: format.clone(),
                });
            }
            let mut scope = RegistryBuildScope::new(format, limits, &mut aggregate);
            let compiled = match invoke_extension(&pending.health, || {
                pending.codec.compile_defaults(&mut scope)
            }) {
                Ok(result) => result,
                Err(RegistrationInvocationError::ExtensionPanicked { detail }) => {
                    return Err(RegistryBuildError::CodecPanicked {
                        format: format.clone(),
                        detail,
                    });
                }
                Err(RegistrationInvocationError::ExtensionPoisoned) => {
                    return Err(RegistryBuildError::ExtensionPoisoned {
                        format: format.clone(),
                    });
                }
            }
            .map_err(|cause| RegistryBuildError::CodecRejected {
                format: format.clone(),
                cause,
            })?;
            if pending.health.is_poisoned() {
                return Err(RegistryBuildError::ExtensionPoisoned {
                    format: format.clone(),
                });
            }

            if compiled.runtime_type_key() != pending.expected_runtime
                || compiled.runtime_type_key() != pending.codec_runtime
            {
                return Err(RegistryBuildError::RuntimeTypeMismatch {
                    format: format.clone(),
                });
            }

            let compiled = compiled.seal_schema(pending.schema.clone());
            let index = entries.len();
            let replaced =
                by_id.insert(RegisteredFormatKey(Arc::clone(&pending.descriptor)), index);
            debug_assert!(replaced.is_none());
            entries.push(FrozenRegistration {
                importer: Arc::clone(&pending.importer),
                descriptor: Arc::clone(&pending.descriptor),
                codec: Arc::clone(&pending.codec),
                schema: pending.schema.clone(),
                expected_runtime: pending.expected_runtime,
                default_options: Arc::new(compiled),
                health: Arc::clone(&pending.health),
            });
        }

        Ok(RegistrySnapshot {
            data: Arc::new(RegistryData {
                entries,
                by_id,
                unavailable,
                unavailable_order,
            }),
        })
    }

    fn register_inner(
        &mut self,
        importer: Arc<dyn FormatImporter>,
    ) -> Result<(), RegistryRegistrationError> {
        let descriptor = catch_unwind(|| importer.descriptor()).map_err(|payload| {
            RegistryRegistrationError::ImporterPanicked {
                detail: bounded_panic_detail(payload),
            }
        })?;
        let id = descriptor.id().clone();

        validate_qualified_format_id(&id)?;
        if self.registrations.contains_key(&id) || self.unavailable.contains_key(&id) {
            return Err(RegistryRegistrationError::DuplicateFormatId { id });
        }

        let captured = catch_unwind(|| {
            let codec = importer.option_codec();
            let expected_runtime = importer.option_runtime_type_key();
            let codec_runtime = codec.runtime_type_key();
            let schema = codec.schema();
            (codec, expected_runtime, codec_runtime, schema)
        })
        .map_err(|payload| RegistryRegistrationError::ImporterPanicked {
            detail: bounded_panic_detail(payload),
        })?;
        let (codec, expected_runtime, codec_runtime, schema) = captured;

        let pending = PendingRegistration {
            importer,
            descriptor: Arc::new(descriptor),
            codec,
            schema,
            expected_runtime,
            codec_runtime,
            health: Arc::new(RegistrationHealth::new()),
        };
        validate_pending(&pending)?;
        let replaced = self.registrations.insert(id, pending);
        debug_assert!(replaced.is_none());
        Ok(())
    }
}

fn validate_qualified_format_id(id: &FormatId) -> Result<(), RegistryRegistrationError> {
    if id.as_str().contains('.') {
        Ok(())
    } else {
        Err(RegistryRegistrationError::FormatIdMustBeQualified { id: id.clone() })
    }
}

impl fmt::Debug for RegistryBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryBuilder")
            .field("registrations", &self.registrations.keys())
            .field("unavailable", &self.unavailable)
            .finish()
    }
}

struct PendingRegistration {
    importer: Arc<dyn FormatImporter>,
    descriptor: Arc<FormatDescriptor>,
    codec: Arc<dyn FormatOptionCodec>,
    schema: Option<FormatOptionSchema>,
    expected_runtime: OptionRuntimeTypeKey,
    codec_runtime: OptionRuntimeTypeKey,
    health: Arc<RegistrationHealth>,
}

fn validate_pending(pending: &PendingRegistration) -> Result<(), RegistryRegistrationError> {
    let format = pending.descriptor.id();
    if let Some(schema) = pending.schema.as_ref()
        && schema.key().owner() != format
    {
        return Err(RegistryRegistrationError::OptionSchemaOwnerMismatch {
            format: format.clone(),
        });
    }
    if pending.expected_runtime != pending.codec_runtime {
        return Err(RegistryRegistrationError::RuntimeTypeMismatch {
            format: format.clone(),
        });
    }
    Ok(())
}

/// Immutable registry view shared by requests and sessions.
#[derive(Clone)]
pub struct RegistrySnapshot {
    data: Arc<RegistryData>,
}

impl RegistrySnapshot {
    pub fn len(&self) -> usize {
        self.data.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.entries.is_empty()
    }

    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = &FormatDescriptor> {
        self.data
            .entries
            .iter()
            .map(|entry| entry.descriptor.as_ref())
    }

    pub fn descriptor(&self, id: &FormatId) -> Option<&FormatDescriptor> {
        self.data
            .by_id
            .get(id)
            .map(|index| self.data.entries[*index].descriptor.as_ref())
    }

    pub fn unavailable_reason(&self, id: &FormatId) -> Option<FormatUnavailability> {
        self.data.unavailable.get(id).copied()
    }

    /// Returns whether both handles refer to the same frozen registry publication.
    pub fn has_same_identity(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = RegistryEntry> + '_ {
        (0..self.data.entries.len()).map(|index| RegistryEntry {
            snapshot: self.clone(),
            index,
        })
    }

    pub(crate) fn entry(&self, id: &FormatId) -> Option<RegistryEntry> {
        self.data.by_id.get(id).map(|index| RegistryEntry {
            snapshot: self.clone(),
            index: *index,
        })
    }

    pub(crate) fn format_id_at(&self, index: usize) -> &FormatId {
        self.data.entries[index].descriptor.id()
    }
}

impl fmt::Debug for RegistrySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistrySnapshot")
            .field("formats", &RegisteredFormatIdsDebug(&self.data))
            .field("unavailable", &UnavailableFormatIdsDebug(&self.data))
            .finish()
    }
}

struct RegistryData {
    entries: Vec<FrozenRegistration>,
    by_id: HashMap<RegisteredFormatKey, usize>,
    unavailable: HashMap<Arc<FormatId>, FormatUnavailability>,
    unavailable_order: Vec<Arc<FormatId>>,
}

struct RegisteredFormatKey(Arc<FormatDescriptor>);

impl Borrow<FormatId> for RegisteredFormatKey {
    fn borrow(&self) -> &FormatId {
        self.0.id()
    }
}

impl PartialEq for RegisteredFormatKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl Eq for RegisteredFormatKey {}

impl Hash for RegisteredFormatKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id().hash(state);
    }
}

struct RegisteredFormatIdsDebug<'a>(&'a RegistryData);

impl fmt::Debug for RegisteredFormatIdsDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.entries.iter().map(|entry| entry.descriptor.id()))
            .finish()
    }
}

struct UnavailableFormatIdsDebug<'a>(&'a RegistryData);

impl fmt::Debug for UnavailableFormatIdsDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut unavailable = formatter.debug_map();
        for id in &self.0.unavailable_order {
            let reason = self
                .0
                .unavailable
                .get(id.as_ref())
                .expect("unavailable order is populated with the lookup map");
            unavailable.entry(id, reason);
        }
        unavailable.finish()
    }
}

struct FrozenRegistration {
    importer: Arc<dyn FormatImporter>,
    descriptor: Arc<FormatDescriptor>,
    codec: Arc<dyn FormatOptionCodec>,
    schema: Option<FormatOptionSchema>,
    expected_runtime: OptionRuntimeTypeKey,
    default_options: Arc<CompiledFormatOptions>,
    health: Arc<RegistrationHealth>,
}

/// Snapshot-qualified handle used only by import-session implementation.
#[derive(Clone)]
pub(crate) struct RegistryEntry {
    snapshot: RegistrySnapshot,
    index: usize,
}

impl RegistryEntry {
    fn registration(&self) -> &FrozenRegistration {
        &self.snapshot.data.entries[self.index]
    }

    pub(crate) fn descriptor(&self) -> &FormatDescriptor {
        &self.registration().descriptor
    }

    pub(crate) fn descriptor_handle(&self) -> Arc<FormatDescriptor> {
        Arc::clone(&self.registration().descriptor)
    }

    pub(crate) fn format_id(&self) -> &FormatId {
        self.descriptor().id()
    }

    pub(crate) fn option_schema(&self) -> Option<&FormatOptionSchema> {
        self.registration().schema.as_ref()
    }

    pub(crate) fn expected_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        self.registration().expected_runtime
    }

    pub(crate) fn default_options(&self) -> Arc<CompiledFormatOptions> {
        Arc::clone(&self.registration().default_options)
    }

    pub(crate) const fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.registration().health.is_poisoned()
    }

    pub(crate) fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        let registration = self.registration();
        let Some(permit) = registration.health.begin_invocation() else {
            return Ok(DetectionOutcome::Fault(
                crate::format::DetectionFault::extension_poisoned(),
            ));
        };
        match catch_unwind(AssertUnwindSafe(|| {
            registration.importer.detect(probe, hint, scope)
        })) {
            Ok(Err(abort)) => Err(abort),
            Ok(Ok(_)) if registration.health.is_poisoned() => Ok(DetectionOutcome::Fault(
                crate::format::DetectionFault::extension_poisoned(),
            )),
            Ok(Ok(outcome)) => Ok(outcome),
            Err(payload) => {
                permit.poison();
                Ok(DetectionOutcome::Fault(detection_fault_from_panic(payload)))
            }
        }
    }

    pub(crate) fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        layers: FormatOptionLayers<'_>,
    ) -> Result<Result<CompiledFormatOptions, OptionCompileError>, RegistrationInvocationError>
    {
        let registration = self.registration();
        let default_options = registration.default_options.as_ref();
        let compiled = invoke_extension(&registration.health, || {
            registration
                .codec
                .compile_layers(scope, default_options, layers)
        })?;
        if registration.health.is_poisoned() {
            return Err(RegistrationInvocationError::ExtensionPoisoned);
        }
        Ok(compiled.map(|compiled| compiled.seal_schema(registration.schema.clone())))
    }

    pub(crate) fn parse(
        &self,
        scope: &mut ParseScope<'_>,
    ) -> Result<ParseResult<SceneDraft>, ParseInvocationError> {
        let registration = self.registration();
        let Some(permit) = registration.health.begin_invocation() else {
            return Err(ParseInvocationError::ExtensionPoisoned);
        };
        let result = match catch_unwind(AssertUnwindSafe(|| registration.importer.parse(scope))) {
            Ok(result) => result,
            Err(payload) => {
                permit.poison();
                discard_untrusted_panic_payload(payload);
                return Err(ParseInvocationError::ExtensionPanicked);
            }
        };
        if registration.health.is_poisoned() && result.is_ok() {
            Err(ParseInvocationError::ExtensionPoisoned)
        } else {
            Ok(result)
        }
    }

    pub(crate) fn belongs_to(&self, snapshot: &RegistrySnapshot) -> bool {
        Arc::ptr_eq(&self.snapshot.data, &snapshot.data)
    }
}

fn reserve_registry_map<K: Eq + Hash, V>(
    map: &mut HashMap<K, V>,
    additional: usize,
) -> Result<(), RegistryBuildError> {
    map.try_reserve(additional)
        .map_err(|_| RegistryBuildError::AllocationFailed)
}

fn reserve_registry_vec<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), RegistryBuildError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| RegistryBuildError::AllocationFailed)
}

impl fmt::Debug for RegistryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryEntry")
            .field("format", &self.format_id())
            .field("poisoned", &self.is_poisoned())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrationInvocationError {
    ExtensionPanicked { detail: BoundedFaultDetail },
    ExtensionPoisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseInvocationError {
    ExtensionPanicked,
    ExtensionPoisoned,
}

const _: () = assert!(std::mem::size_of::<ParseInvocationError>() == 1);

/// Eager registration failure. The builder is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistryRegistrationError {
    DuplicateFormatId { id: FormatId },
    FormatIdMustBeQualified { id: FormatId },
    OptionSchemaOwnerMismatch { format: FormatId },
    RuntimeTypeMismatch { format: FormatId },
    ImporterPanicked { detail: BoundedFaultDetail },
}

const _: () = assert!(std::mem::size_of::<RegistryRegistrationError>() < 128);

impl RegistryRegistrationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DuplicateFormatId { .. } => "duplicate-format-id",
            Self::FormatIdMustBeQualified { .. } => "format-id-must-be-qualified",
            Self::OptionSchemaOwnerMismatch { .. } => "option-schema-owner-mismatch",
            Self::RuntimeTypeMismatch { .. } => "option-runtime-type-mismatch",
            Self::ImporterPanicked { .. } => "importer-panicked",
        }
    }
}

impl fmt::Display for RegistryRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateFormatId { id } => write!(formatter, "duplicate format id {id}"),
            Self::FormatIdMustBeQualified { id } => {
                write!(formatter, "format id {id} must use a reverse-DNS prefix")
            }
            Self::OptionSchemaOwnerMismatch { format } => {
                write!(
                    formatter,
                    "option schema owner does not match format {format}"
                )
            }
            Self::RuntimeTypeMismatch { format } => {
                write!(
                    formatter,
                    "option runtime type does not match importer {format}"
                )
            }
            Self::ImporterPanicked { detail } => {
                write!(
                    formatter,
                    "format importer panicked during registration: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for RegistryRegistrationError {}

/// Atomic snapshot-build failure. No partial snapshot is published.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistryBuildError {
    Registration(RegistryRegistrationError),
    CodecPanicked {
        format: FormatId,
        detail: BoundedFaultDetail,
    },
    CodecRejected {
        format: FormatId,
        cause: OptionCompileError,
    },
    RuntimeTypeMismatch {
        format: FormatId,
    },
    ExtensionPoisoned {
        format: FormatId,
    },
    AllocationFailed,
}

const _: () = assert!(std::mem::size_of::<RegistryBuildError>() < 128);

impl RegistryBuildError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Registration(_) => "invalid-registration",
            Self::CodecPanicked { .. } => "default-codec-panicked",
            Self::CodecRejected { .. } => "default-codec-rejected",
            Self::RuntimeTypeMismatch { .. } => "option-runtime-type-mismatch",
            Self::ExtensionPoisoned { .. } => "extension-poisoned",
            Self::AllocationFailed => "allocation-failed",
        }
    }
}

impl fmt::Display for RegistryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registration(error) => write!(formatter, "invalid registry proposal: {error}"),
            Self::CodecPanicked { format, detail } => {
                write!(
                    formatter,
                    "default option codec for {format} panicked: {detail}"
                )
            }
            Self::CodecRejected { format, cause } => {
                write!(
                    formatter,
                    "default options for {format} were rejected: {cause}"
                )
            }
            Self::RuntimeTypeMismatch { format } => write!(
                formatter,
                "compiled default option type does not match importer {format}"
            ),
            Self::ExtensionPoisoned { format } => {
                write!(formatter, "format registration {format} is poisoned")
            }
            Self::AllocationFailed => formatter.write_str("registry snapshot allocation failed"),
        }
    }
}

// The high bit is a one-way poison latch. The remaining bits count concurrent
// invocation permits, so admission is linearizable without serializing a
// semantically reentrant registration behind a lock.
const POISONED_BIT: usize = 1_usize << (usize::BITS - 1);
const ACTIVE_INVOCATIONS_MASK: usize = !POISONED_BIT;

#[derive(Debug)]
struct RegistrationHealth {
    state: AtomicUsize,
}

impl RegistrationHealth {
    const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
        }
    }

    fn is_poisoned(&self) -> bool {
        self.state.load(Ordering::Acquire) & POISONED_BIT != 0
    }

    fn poison(&self) {
        self.state.fetch_or(POISONED_BIT, Ordering::AcqRel);
    }

    fn begin_invocation(self: &Arc<Self>) -> Option<RegistrationInvocationPermit> {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current & POISONED_BIT != 0 {
                return None;
            }
            if (current & ACTIVE_INVOCATIONS_MASK) == ACTIVE_INVOCATIONS_MASK {
                self.poison();
                return None;
            }
            match self.state.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(RegistrationInvocationPermit {
                        health: Arc::clone(self),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

struct RegistrationInvocationPermit {
    health: Arc<RegistrationHealth>,
}

impl RegistrationInvocationPermit {
    fn poison(&self) {
        self.health.poison();
    }
}

impl Drop for RegistrationInvocationPermit {
    fn drop(&mut self) {
        self.health.state.fetch_sub(1, Ordering::Release);
    }
}

fn invoke_extension<T>(
    health: &Arc<RegistrationHealth>,
    invocation: impl FnOnce() -> T,
) -> Result<T, RegistrationInvocationError> {
    let Some(permit) = health.begin_invocation() else {
        return Err(RegistrationInvocationError::ExtensionPoisoned);
    };
    match catch_unwind(AssertUnwindSafe(invocation)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            permit.poison();
            Err(RegistrationInvocationError::ExtensionPanicked {
                detail: bounded_panic_detail(payload),
            })
        }
    }
}

impl std::error::Error for RegistryBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration(error) => Some(error),
            Self::CodecRejected { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{CapabilityStatus, FormatCapability, FormatEncoding};
    use crate::error::ScopeAbortCause;
    use crate::format::{DetectionFaultCode, ReadConfidence};
    use crate::ledger::{
        ImportDimension, ImportOperationControl, import_ledger_limits, try_import_coordinator,
    };
    use crate::limits::ResourceLimits;
    use crate::options::{
        FormatOptionEnvelope, FormatOptionKey, FormatOptionSchemaId, FormatOptionSchemaKey,
        OptionCompileLimits,
    };
    use baozi_io::AssetPath;
    use baozi_operation::LedgerCause;
    use std::cell::RefCell;
    use std::num::NonZeroU32;
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[derive(Debug, PartialEq, Eq)]
    struct ReentrantOptions(u8);

    struct ReentrantCodec {
        schema: FormatOptionSchema,
        concurrent_gate: Arc<Barrier>,
        layer_calls: AtomicUsize,
    }

    impl FormatOptionCodec for ReentrantCodec {
        fn schema(&self) -> Option<FormatOptionSchema> {
            Some(self.schema.clone())
        }

        fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
            OptionRuntimeTypeKey::of::<ReentrantOptions>()
        }

        fn compile_defaults(
            &self,
            scope: &mut RegistryBuildScope<'_>,
        ) -> Result<CompiledFormatOptions, OptionCompileError> {
            scope.charge_work(1)?;
            scope.finish(ReentrantOptions(0), 1)
        }

        fn compile_layers(
            &self,
            scope: &mut OptionCompileScope<'_>,
            defaults: &CompiledFormatOptions,
            layers: FormatOptionLayers<'_>,
        ) -> Result<CompiledFormatOptions, OptionCompileError> {
            let key = FormatOptionKey::<ReentrantOptions>::new(self.schema.clone());
            let _ = defaults.downcast_ref(&key).ok_or_else(|| {
                OptionCompileError::invalid_payload("default-type", "wrong default type")
            })?;
            let payload = layers.caller().ok_or_else(|| {
                OptionCompileError::invalid_payload("missing-layer", "missing caller layer")
            })?;
            let bytes = scope.reader(payload).read_remaining()?;
            let [value] = bytes else {
                return Err(OptionCompileError::invalid_payload(
                    "one-byte",
                    "expected exactly one byte",
                ));
            };
            if self.layer_calls.fetch_add(1, Ordering::AcqRel) < 2 {
                self.concurrent_gate.wait();
            }
            scope.finish(ReentrantOptions(*value), 1)
        }
    }

    struct ReentrantImporter {
        codec: Arc<dyn FormatOptionCodec>,
        detector_gate: Arc<Barrier>,
        detector_calls: AtomicUsize,
    }

    impl FormatImporter for ReentrantImporter {
        fn descriptor(&self) -> FormatDescriptor {
            descriptor_for("org.example.reentrant", "reentrant")
        }

        fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
            Arc::clone(&self.codec)
        }

        fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
            OptionRuntimeTypeKey::of::<ReentrantOptions>()
        }

        fn detect(
            &self,
            _probe: &Probe<'_>,
            hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            if self.detector_calls.fetch_add(1, Ordering::AcqRel) < 2 {
                self.detector_gate.wait();
            }
            Ok(match hint.extension() {
                Some("left") => DetectionOutcome::Match(ReadConfidence::Maybe),
                Some("right") => DetectionOutcome::Match(ReadConfidence::Certain),
                _ => DetectionOutcome::NoMatch,
            })
        }

        fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            unreachable!("registry reentrancy test does not parse")
        }
    }

    struct RacingDetector {
        entered: Arc<Barrier>,
        release_success: Arc<Barrier>,
        calls: Arc<AtomicUsize>,
    }

    impl FormatImporter for RacingDetector {
        fn descriptor(&self) -> FormatDescriptor {
            descriptor_for("org.example.racing-detector", "race")
        }

        fn detect(
            &self,
            _probe: &Probe<'_>,
            hint: &ReadHint,
            scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.entered.wait();
            if hint.extension() == Some("panic") {
                panic!("racing detector panic");
            }
            self.release_success.wait();
            if hint.extension() == Some("abort") {
                return match scope.with_work(u64::MAX, || Ok(DetectionOutcome::NoMatch)) {
                    Ok(_) => panic!("oversized detector work must be rejected"),
                    Err(abort) => Err(abort),
                };
            }
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            unreachable!("detector poison race test does not parse")
        }
    }

    struct DelayedDefaultPanicCodec {
        schema: FormatOptionSchema,
        calls: Arc<AtomicUsize>,
    }

    impl FormatOptionCodec for DelayedDefaultPanicCodec {
        fn schema(&self) -> Option<FormatOptionSchema> {
            Some(self.schema.clone())
        }

        fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
            OptionRuntimeTypeKey::of::<ReentrantOptions>()
        }

        fn compile_defaults(
            &self,
            scope: &mut RegistryBuildScope<'_>,
        ) -> Result<CompiledFormatOptions, OptionCompileError> {
            if self.calls.fetch_add(1, Ordering::AcqRel) >= 2 {
                panic!("late default panic");
            }
            scope.charge_work(1)?;
            scope.finish(ReentrantOptions(0), 1)
        }

        fn compile_layers(
            &self,
            scope: &mut OptionCompileScope<'_>,
            _defaults: &CompiledFormatOptions,
            _layers: FormatOptionLayers<'_>,
        ) -> Result<CompiledFormatOptions, OptionCompileError> {
            scope.finish(ReentrantOptions(0), 1)
        }
    }

    struct DelayedDefaultPanicImporter {
        codec: Arc<dyn FormatOptionCodec>,
        detector_calls: Arc<AtomicUsize>,
    }

    impl FormatImporter for DelayedDefaultPanicImporter {
        fn descriptor(&self) -> FormatDescriptor {
            descriptor_for("org.example.late-default-panic", "late")
        }

        fn option_codec(&self) -> Arc<dyn FormatOptionCodec> {
            Arc::clone(&self.codec)
        }

        fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
            OptionRuntimeTypeKey::of::<ReentrantOptions>()
        }

        fn detect(
            &self,
            _probe: &Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            self.detector_calls.fetch_add(1, Ordering::AcqRel);
            Ok(DetectionOutcome::NoMatch)
        }

        fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            unreachable!("default codec poison test does not parse")
        }
    }

    fn descriptor_for(format: &str, extension: &str) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(format).unwrap(), "Test format")
            .extensions([extension])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
            .build()
            .unwrap()
    }

    fn reentrant_schema(format: &FormatId) -> FormatOptionSchema {
        FormatOptionSchema::new(
            FormatOptionSchemaKey::new(
                format.clone(),
                FormatOptionSchemaId::new("import-options").unwrap(),
            ),
            NonZeroU32::new(1).unwrap(),
        )
    }

    fn compile_reentrant_value(
        snapshot: RegistrySnapshot,
        format: FormatId,
        schema: FormatOptionSchema,
        value: u8,
    ) -> u8 {
        let envelope = FormatOptionEnvelope::new(
            format.clone(),
            schema.key().clone(),
            schema.version(),
            [value],
        )
        .unwrap();
        let layers = FormatOptionLayers::new(None, Some(envelope.payload()));
        let mut aggregate = CompileAggregate::default();
        let mut scope =
            OptionCompileScope::new(&format, OptionCompileLimits::default(), &mut aggregate);
        let compiled = snapshot
            .entry(&format)
            .unwrap()
            .compile_layers(&mut scope, layers)
            .unwrap()
            .unwrap();
        compiled
            .downcast_ref(&FormatOptionKey::<ReentrantOptions>::new(schema))
            .unwrap()
            .0
    }

    struct PanickingDetector {
        calls: Arc<AtomicUsize>,
    }

    impl FormatImporter for PanickingDetector {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("org.example.panicking-detector").unwrap(),
                "Panicking detector",
            )
            .extensions(["panic"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
            .build()
            .unwrap()
        }

        fn detect(
            &self,
            _probe: &Probe<'_>,
            _hint: &ReadHint,
            scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            scope.with_work(1, || panic!("detector panic"))
        }

        fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            unreachable!("detector poison test does not parse")
        }
    }

    fn detect_entry(
        entry: &RegistryEntry,
        probe: &Probe<'_>,
        hint: &ReadHint,
    ) -> Result<DetectionOutcome, LedgerCause<ImportDimension>> {
        let coordinator = RefCell::new(try_import_coordinator(
            ImportOperationControl::default(),
            import_ledger_limits(&ResourceLimits::default()).unwrap(),
        )?);
        let terminal = RefCell::new(None);
        let scope = DetectionScope::new(&coordinator, &terminal);
        let outcome = entry
            .detect(probe, hint, &scope)
            .expect("uncontrolled registry detector must not abort");
        assert!(scope.into_terminal_abort().is_none());
        Ok(outcome)
    }

    fn detect_entry_abort_code(
        entry: &RegistryEntry,
        probe: &Probe<'_>,
        hint: &ReadHint,
    ) -> Result<&'static str, LedgerCause<ImportDimension>> {
        let coordinator = RefCell::new(try_import_coordinator(
            ImportOperationControl::default(),
            import_ledger_limits(&ResourceLimits::default()).unwrap(),
        )?);
        let terminal = RefCell::new(None);
        let scope = DetectionScope::new(&coordinator, &terminal);
        assert!(entry.detect(probe, hint, &scope).is_err());
        let abort = scope
            .into_terminal_abort()
            .expect("detector abort must latch a terminal cause");
        let code = match abort.cause() {
            ScopeAbortCause::DescriptorContract { code } => code,
            cause => panic!("expected descriptor contract abort, got {cause:?}"),
        };
        Ok(code)
    }

    #[test]
    fn detector_panic_poison_is_shared_by_all_snapshots_of_one_registration()
    -> Result<(), LedgerCause<ImportDimension>> {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RegistryBuilder::new();
        builder
            .register(PanickingDetector {
                calls: Arc::clone(&calls),
            })
            .unwrap();
        let first = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let second = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let id = FormatId::new("org.example.panicking-detector").unwrap();
        let probe = Probe::new(&[], 0);
        let hint = ReadHint::from_source(AssetPath::new("model.panic").unwrap());

        let DetectionOutcome::Fault(first_fault) =
            detect_entry(&first.entry(&id).unwrap(), &probe, &hint)?
        else {
            panic!("panicking detector must produce fault evidence");
        };
        assert_eq!(first_fault.code(), DetectionFaultCode::ExtensionPanicked);
        assert_eq!(first_fault.detail(), "detector panic");
        let DetectionOutcome::Fault(second_fault) =
            detect_entry(&second.entry(&id).unwrap(), &probe, &hint)?
        else {
            panic!("poisoned detector must produce fault evidence");
        };
        assert_eq!(second_fault.code(), DetectionFaultCode::ExtensionPoisoned);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            builder.freeze(RegistryBuildLimits::default()),
            Err(RegistryBuildError::ExtensionPoisoned { format }) if format == id
        ));
        Ok(())
    }

    #[test]
    fn one_snapshot_is_semantically_reentrant_for_detection_and_option_compilation()
    -> Result<(), LedgerCause<ImportDimension>> {
        let format = FormatId::new("org.example.reentrant").unwrap();
        let schema = reentrant_schema(&format);
        let codec: Arc<dyn FormatOptionCodec> = Arc::new(ReentrantCodec {
            schema: schema.clone(),
            concurrent_gate: Arc::new(Barrier::new(2)),
            layer_calls: AtomicUsize::new(0),
        });
        let mut builder = RegistryBuilder::new();
        builder
            .register(ReentrantImporter {
                codec,
                detector_gate: Arc::new(Barrier::new(2)),
                detector_calls: AtomicUsize::new(0),
            })
            .unwrap();
        let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();

        let left_snapshot = snapshot.clone();
        let left_format = format.clone();
        let left_schema = schema.clone();
        let left = thread::spawn(move || {
            compile_reentrant_value(left_snapshot, left_format, left_schema, 11)
        });
        let right_snapshot = snapshot.clone();
        let right_format = format.clone();
        let right_schema = schema.clone();
        let right = thread::spawn(move || {
            compile_reentrant_value(right_snapshot, right_format, right_schema, 29)
        });
        let concurrent_options = (left.join().unwrap(), right.join().unwrap());
        let serial_options = (
            compile_reentrant_value(snapshot.clone(), format.clone(), schema.clone(), 11),
            compile_reentrant_value(snapshot.clone(), format.clone(), schema, 29),
        );
        assert_eq!(concurrent_options, serial_options);

        let left_snapshot = snapshot.clone();
        let left_format = format.clone();
        let left = thread::spawn(move || {
            let probe = Probe::new(&[], 0);
            let hint = ReadHint::from_source(AssetPath::new("model.left").unwrap());
            detect_entry(&left_snapshot.entry(&left_format).unwrap(), &probe, &hint)
        });
        let right_snapshot = snapshot.clone();
        let right_format = format.clone();
        let right = thread::spawn(move || {
            let probe = Probe::new(&[], 0);
            let hint = ReadHint::from_source(AssetPath::new("model.right").unwrap());
            detect_entry(&right_snapshot.entry(&right_format).unwrap(), &probe, &hint)
        });
        let concurrent_detection = (left.join().unwrap()?, right.join().unwrap()?);
        let probe = Probe::new(&[], 0);
        let serial_detection = (
            detect_entry(
                &snapshot.entry(&format).unwrap(),
                &probe,
                &ReadHint::from_source(AssetPath::new("model.left").unwrap()),
            )?,
            detect_entry(
                &snapshot.entry(&format).unwrap(),
                &probe,
                &ReadHint::from_source(AssetPath::new("model.right").unwrap()),
            )?,
        );
        assert_eq!(concurrent_detection, serial_detection);
        Ok(())
    }

    #[test]
    fn poison_linearization_rejects_in_flight_success_and_later_reinvocation()
    -> Result<(), LedgerCause<ImportDimension>> {
        let entered = Arc::new(Barrier::new(3));
        let release_success = Arc::new(Barrier::new(2));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RegistryBuilder::new();
        builder
            .register(RacingDetector {
                entered: Arc::clone(&entered),
                release_success: Arc::clone(&release_success),
                calls: Arc::clone(&calls),
            })
            .unwrap();
        let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let format = FormatId::new("org.example.racing-detector").unwrap();

        let panic_snapshot = snapshot.clone();
        let panic_format = format.clone();
        let panicking = thread::spawn(move || {
            detect_entry(
                &panic_snapshot.entry(&panic_format).unwrap(),
                &Probe::new(&[], 0),
                &ReadHint::from_source(AssetPath::new("model.panic").unwrap()),
            )
        });
        let success_snapshot = snapshot.clone();
        let success_format = format.clone();
        let succeeding = thread::spawn(move || {
            detect_entry(
                &success_snapshot.entry(&success_format).unwrap(),
                &Probe::new(&[], 0),
                &ReadHint::from_source(AssetPath::new("model.ok").unwrap()),
            )
        });
        entered.wait();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !snapshot.entry(&format).unwrap().is_poisoned()
            && std::time::Instant::now() < deadline
        {
            thread::yield_now();
        }
        let poisoned = snapshot.entry(&format).unwrap().is_poisoned();
        release_success.wait();
        assert!(poisoned, "panicking invocation did not poison registration");

        let DetectionOutcome::Fault(panic_fault) = panicking.join().unwrap()? else {
            panic!("panicking invocation must return fault evidence");
        };
        assert_eq!(panic_fault.code(), DetectionFaultCode::ExtensionPanicked);
        let DetectionOutcome::Fault(success_fault) = succeeding.join().unwrap()? else {
            panic!("in-flight success must be discarded after poison");
        };
        assert_eq!(success_fault.code(), DetectionFaultCode::ExtensionPoisoned);

        let DetectionOutcome::Fault(later_fault) = detect_entry(
            &snapshot.entry(&format).unwrap(),
            &Probe::new(&[], 0),
            &ReadHint::from_source(AssetPath::new("model.ok").unwrap()),
        )?
        else {
            panic!("later invocation must be rejected before extension entry");
        };
        assert_eq!(later_fault.code(), DetectionFaultCode::ExtensionPoisoned);
        assert_eq!(calls.load(Ordering::Acquire), 2);
        Ok(())
    }

    #[test]
    fn poison_linearization_preserves_in_flight_scope_abort()
    -> Result<(), LedgerCause<ImportDimension>> {
        let entered = Arc::new(Barrier::new(3));
        let release_abort = Arc::new(Barrier::new(2));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RegistryBuilder::new();
        builder
            .register(RacingDetector {
                entered: Arc::clone(&entered),
                release_success: Arc::clone(&release_abort),
                calls: Arc::clone(&calls),
            })
            .unwrap();
        let snapshot = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let format = FormatId::new("org.example.racing-detector").unwrap();

        let panic_snapshot = snapshot.clone();
        let panic_format = format.clone();
        let panicking = thread::spawn(move || {
            detect_entry(
                &panic_snapshot.entry(&panic_format).unwrap(),
                &Probe::new(&[], 0),
                &ReadHint::from_source(AssetPath::new("model.panic").unwrap()),
            )
        });
        let abort_snapshot = snapshot.clone();
        let abort_format = format.clone();
        let aborting = thread::spawn(move || {
            detect_entry_abort_code(
                &abort_snapshot.entry(&abort_format).unwrap(),
                &Probe::new(&[], 0),
                &ReadHint::from_source(AssetPath::new("model.abort").unwrap()),
            )
        });
        entered.wait();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !snapshot.entry(&format).unwrap().is_poisoned()
            && std::time::Instant::now() < deadline
        {
            thread::yield_now();
        }
        let poisoned = snapshot.entry(&format).unwrap().is_poisoned();
        release_abort.wait();
        assert!(poisoned, "panicking invocation did not poison registration");

        let DetectionOutcome::Fault(panic_fault) = panicking.join().unwrap()? else {
            panic!("panicking invocation must return fault evidence");
        };
        assert_eq!(panic_fault.code(), DetectionFaultCode::ExtensionPanicked);
        assert_eq!(aborting.join().unwrap()?, "detector-work-quantum-exceeded");
        assert_eq!(calls.load(Ordering::Acquire), 2);
        Ok(())
    }

    #[test]
    fn default_codec_panic_poisons_existing_and_future_snapshots()
    -> Result<(), LedgerCause<ImportDimension>> {
        let format = FormatId::new("org.example.late-default-panic").unwrap();
        let default_calls = Arc::new(AtomicUsize::new(0));
        let detector_calls = Arc::new(AtomicUsize::new(0));
        let codec: Arc<dyn FormatOptionCodec> = Arc::new(DelayedDefaultPanicCodec {
            schema: reentrant_schema(&format),
            calls: Arc::clone(&default_calls),
        });
        let mut builder = RegistryBuilder::new();
        builder
            .register(DelayedDefaultPanicImporter {
                codec,
                detector_calls: Arc::clone(&detector_calls),
            })
            .unwrap();
        let first = builder.freeze(RegistryBuildLimits::default()).unwrap();
        let second = builder.freeze(RegistryBuildLimits::default()).unwrap();

        assert!(matches!(
            builder.freeze(RegistryBuildLimits::default()),
            Err(RegistryBuildError::CodecPanicked {
                format: failed,
                detail,
            }) if failed == format && detail.as_ref() == "late default panic"
        ));
        for snapshot in [&first, &second] {
            let DetectionOutcome::Fault(fault) = detect_entry(
                &snapshot.entry(&format).unwrap(),
                &Probe::new(&[], 0),
                &ReadHint::from_source(AssetPath::new("model.late").unwrap()),
            )?
            else {
                panic!("existing snapshot must observe permanent poison");
            };
            assert_eq!(fault.code(), DetectionFaultCode::ExtensionPoisoned);
        }
        assert!(matches!(
            builder.freeze(RegistryBuildLimits::default()),
            Err(RegistryBuildError::ExtensionPoisoned { format: failed }) if failed == format
        ));
        assert_eq!(default_calls.load(Ordering::Acquire), 3);
        assert_eq!(detector_calls.load(Ordering::Acquire), 0);
        Ok(())
    }

    #[test]
    fn collection_capacity_overflow_is_a_typed_registry_error() {
        let mut map = HashMap::<u8, u8>::new();
        assert!(matches!(
            reserve_registry_map(&mut map, usize::MAX),
            Err(RegistryBuildError::AllocationFailed)
        ));

        let mut values = Vec::<u8>::new();
        assert!(matches!(
            reserve_registry_vec(&mut values, usize::MAX),
            Err(RegistryBuildError::AllocationFailed)
        ));
    }
}
