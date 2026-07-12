use crate::format::BoundedFaultDetail;
use baozi_core::FormatId;
use std::any::{Any, TypeId};
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::panic::RefUnwindSafe;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

const MAX_SCHEMA_ID_BYTES: usize = 64;
const MAX_ENVELOPE_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_ERROR_CODE_BYTES: usize = 64;

/// A format-local stable option-schema identifier.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FormatOptionSchemaId {
    bytes: [u8; MAX_SCHEMA_ID_BYTES],
    length: u8,
}

impl FormatOptionSchemaId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, FormatOptionSchemaIdError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(FormatOptionSchemaIdError::Empty);
        }
        if value.len() > MAX_SCHEMA_ID_BYTES {
            return Err(FormatOptionSchemaIdError::TooLong {
                actual: value.len(),
                maximum: MAX_SCHEMA_ID_BYTES,
            });
        }
        if let Some(offset) = invalid_local_id_offset(value) {
            return Err(FormatOptionSchemaIdError::InvalidByte { offset });
        }
        let length = u8::try_from(value.len()).map_err(|_| FormatOptionSchemaIdError::TooLong {
            actual: value.len(),
            maximum: MAX_SCHEMA_ID_BYTES,
        })?;
        let mut bytes = [0; MAX_SCHEMA_ID_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self { bytes, length })
    }

    pub fn as_str(&self) -> &str {
        let length = usize::from(self.length);
        std::str::from_utf8(&self.bytes[..length])
            .expect("FormatOptionSchemaId construction accepts only ASCII bytes")
    }
}

impl fmt::Debug for FormatOptionSchemaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FormatOptionSchemaId")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for FormatOptionSchemaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a local option-schema identity is not canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatOptionSchemaIdError {
    Empty,
    TooLong { actual: usize, maximum: usize },
    InvalidByte { offset: usize },
}

impl fmt::Display for FormatOptionSchemaIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("format option schema id must not be empty"),
            Self::TooLong { actual, maximum } => write!(
                formatter,
                "format option schema id is {actual} bytes; the maximum is {maximum}"
            ),
            Self::InvalidByte { offset } => write!(
                formatter,
                "format option schema id has a noncanonical byte at offset {offset}"
            ),
        }
    }
}

impl std::error::Error for FormatOptionSchemaIdError {}

const _: () = assert!(MAX_SCHEMA_ID_BYTES <= 255);

/// Stable serialized identity of one format-owned option schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FormatOptionSchemaKey {
    owner: FormatId,
    local: FormatOptionSchemaId,
}

impl FormatOptionSchemaKey {
    pub const fn new(owner: FormatId, local: FormatOptionSchemaId) -> Self {
        Self { owner, local }
    }

    pub const fn owner(&self) -> &FormatId {
        &self.owner
    }

    pub const fn local(&self) -> &FormatOptionSchemaId {
        &self.local
    }
}

/// Immutable schema metadata exposed by an option codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptionSchema {
    key: FormatOptionSchemaKey,
    version: NonZeroU32,
}

impl FormatOptionSchema {
    pub const fn new(key: FormatOptionSchemaKey, version: NonZeroU32) -> Self {
        Self { key, version }
    }

    pub const fn key(&self) -> &FormatOptionSchemaKey {
        &self.key
    }

    pub const fn version(&self) -> NonZeroU32 {
        self.version
    }
}

/// Opaque process-local identity for a concrete compiled option type.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OptionRuntimeTypeKey(TypeId);

impl OptionRuntimeTypeKey {
    pub fn of<T: Any + Send + Sync + RefUnwindSafe>() -> Self {
        Self(TypeId::of::<T>())
    }
}

impl fmt::Debug for OptionRuntimeTypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OptionRuntimeTypeKey(..)")
    }
}

/// Typed key shared by a format's codec, parser, and envelope factory.
#[derive(Debug, Clone)]
pub struct FormatOptionKey<T: Any + Send + Sync + RefUnwindSafe> {
    schema: FormatOptionSchema,
    runtime: OptionRuntimeTypeKey,
    option_type: PhantomData<fn() -> T>,
}

impl<T: Any + Send + Sync + RefUnwindSafe> FormatOptionKey<T> {
    pub fn new(schema: FormatOptionSchema) -> Self {
        Self {
            schema,
            runtime: OptionRuntimeTypeKey::of::<T>(),
            option_type: PhantomData,
        }
    }

    pub const fn schema(&self) -> &FormatOptionSchema {
        &self.schema
    }

    pub const fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        self.runtime
    }

    /// Wraps payload emitted by the owning format's canonical encoder.
    ///
    /// The target is always copied from the schema owner and cannot be relabelled.
    pub fn envelope_from_canonical_payload(
        &self,
        payload: impl AsRef<[u8]>,
    ) -> Result<FormatOptionEnvelope, FormatOptionEnvelopeError> {
        FormatOptionEnvelope::new(
            self.schema.key.owner.clone(),
            self.schema.key.clone(),
            self.schema.version,
            payload,
        )
    }
}

/// Versioned serialized format-option input.
#[derive(Clone, PartialEq, Eq)]
pub struct FormatOptionEnvelope {
    target: FormatId,
    schema: FormatOptionSchemaKey,
    version: NonZeroU32,
    payload: Vec<u8>,
}

impl fmt::Debug for FormatOptionEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FormatOptionEnvelope")
            .field("target", &self.target)
            .field("schema", &self.schema)
            .field("version", &self.version)
            .field("payload_len", &self.payload.len())
            .finish_non_exhaustive()
    }
}

impl FormatOptionEnvelope {
    /// Creates a dynamic envelope. Cross-format owner mismatches are intentionally
    /// admitted here and rejected against a concrete registry snapshot.
    pub fn new(
        target: FormatId,
        schema: FormatOptionSchemaKey,
        version: NonZeroU32,
        payload: impl AsRef<[u8]>,
    ) -> Result<Self, FormatOptionEnvelopeError> {
        let payload = payload.as_ref();
        if payload.len() > MAX_ENVELOPE_PAYLOAD_BYTES {
            return Err(FormatOptionEnvelopeError::PayloadTooLarge {
                actual: payload.len(),
                maximum: MAX_ENVELOPE_PAYLOAD_BYTES,
            });
        }
        let mut owned = Vec::new();
        reserve_envelope_payload(&mut owned, payload.len())?;
        owned.extend_from_slice(payload);
        Ok(Self {
            target,
            schema,
            version,
            payload: owned,
        })
    }

    pub const fn target(&self) -> &FormatId {
        &self.target
    }

    pub const fn schema(&self) -> &FormatOptionSchemaKey {
        &self.schema
    }

    pub const fn version(&self) -> NonZeroU32 {
        self.version
    }

    pub const fn payload_len(&self) -> usize {
        self.payload.len()
    }

    /// Crate-owned transport access. Format codecs receive only the opaque,
    /// metered `FormatOptionPayload` view below.
    pub(crate) fn canonical_payload_bytes(&self) -> &[u8] {
        &self.payload
    }

    pub(crate) fn payload(&self) -> FormatOptionPayload<'_> {
        FormatOptionPayload {
            bytes: &self.payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatOptionEnvelopeError {
    PayloadTooLarge { actual: usize, maximum: usize },
    AllocationFailed,
}

impl fmt::Display for FormatOptionEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { actual, maximum } => write!(
                formatter,
                "format option payload is {actual} bytes; the maximum is {maximum}"
            ),
            Self::AllocationFailed => {
                formatter.write_str("format option payload allocation failed")
            }
        }
    }
}

impl std::error::Error for FormatOptionEnvelopeError {}

fn reserve_envelope_payload(
    payload: &mut Vec<u8>,
    additional: usize,
) -> Result<(), FormatOptionEnvelopeError> {
    payload
        .try_reserve_exact(additional)
        .map_err(|_| FormatOptionEnvelopeError::AllocationFailed)
}

/// Per-codec bounds used by registry and request compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionScopeLimits {
    max_work: u64,
    max_payload_bytes: u64,
    max_decoded_bytes: u64,
    max_decoded_cells: u64,
    max_recursion_depth: u32,
    max_live_bytes: u64,
    max_result_bytes: u64,
}

impl OptionScopeLimits {
    pub const fn new(
        max_work: u64,
        max_payload_bytes: u64,
        max_decoded_bytes: u64,
        max_decoded_cells: u64,
        max_recursion_depth: u32,
        max_live_bytes: u64,
        max_result_bytes: u64,
    ) -> Self {
        Self {
            max_work,
            max_payload_bytes,
            max_decoded_bytes,
            max_decoded_cells,
            max_recursion_depth,
            max_live_bytes,
            max_result_bytes,
        }
    }
}

impl Default for OptionScopeLimits {
    fn default() -> Self {
        Self::new(
            1_000_000,
            1024 * 1024,
            1024 * 1024,
            100_000,
            64,
            1024 * 1024,
            1024 * 1024,
        )
    }
}

/// Aggregate and per-codec bounds for atomic registry freeze.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryBuildLimits {
    per_codec: OptionScopeLimits,
    max_total_work: u64,
    max_total_decoded_bytes: u64,
    max_total_decoded_cells: u64,
    max_total_live_bytes: u64,
    max_total_result_bytes: u64,
}

impl RegistryBuildLimits {
    pub const fn new(
        per_codec: OptionScopeLimits,
        max_total_work: u64,
        max_total_live_bytes: u64,
        max_total_result_bytes: u64,
    ) -> Self {
        Self {
            per_codec,
            max_total_work,
            max_total_decoded_bytes: per_codec.max_decoded_bytes,
            max_total_decoded_cells: per_codec.max_decoded_cells,
            max_total_live_bytes,
            max_total_result_bytes,
        }
    }

    /// Overrides the aggregate decoded-output bounds across all registered codecs.
    pub const fn with_aggregate_decoded_limits(
        mut self,
        max_total_decoded_bytes: u64,
        max_total_decoded_cells: u64,
    ) -> Self {
        self.max_total_decoded_bytes = max_total_decoded_bytes;
        self.max_total_decoded_cells = max_total_decoded_cells;
        self
    }
}

impl Default for RegistryBuildLimits {
    fn default() -> Self {
        Self::new(
            OptionScopeLimits::default(),
            10_000_000,
            16 * 1024 * 1024,
            16 * 1024 * 1024,
        )
        .with_aggregate_decoded_limits(16 * 1024 * 1024, 1_600_000)
    }
}

/// Aggregate and per-target bounds for submitted request options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionCompileLimits {
    per_target: OptionScopeLimits,
    max_envelopes: usize,
    max_total_work: u64,
    max_total_payload_bytes: u64,
    max_total_decoded_bytes: u64,
    max_total_decoded_cells: u64,
    max_total_live_bytes: u64,
    max_total_result_bytes: u64,
}

impl OptionCompileLimits {
    pub const fn new(
        per_target: OptionScopeLimits,
        max_envelopes: usize,
        max_total_work: u64,
        max_total_payload_bytes: u64,
        max_total_live_bytes: u64,
        max_total_result_bytes: u64,
    ) -> Self {
        Self {
            per_target,
            max_envelopes,
            max_total_work,
            max_total_payload_bytes,
            max_total_decoded_bytes: per_target.max_decoded_bytes,
            max_total_decoded_cells: per_target.max_decoded_cells,
            max_total_live_bytes,
            max_total_result_bytes,
        }
    }

    /// Overrides the aggregate decoded-output bounds across all submitted targets.
    pub const fn with_aggregate_decoded_limits(
        mut self,
        max_total_decoded_bytes: u64,
        max_total_decoded_cells: u64,
    ) -> Self {
        self.max_total_decoded_bytes = max_total_decoded_bytes;
        self.max_total_decoded_cells = max_total_decoded_cells;
        self
    }

    pub(crate) const fn max_envelopes(self) -> usize {
        self.max_envelopes
    }

    pub(crate) const fn max_total_payload_bytes(self) -> u64 {
        self.max_total_payload_bytes
    }
}

impl Default for OptionCompileLimits {
    fn default() -> Self {
        Self::new(
            OptionScopeLimits::default(),
            64,
            10_000_000,
            4 * 1024 * 1024,
            4 * 1024 * 1024,
            4 * 1024 * 1024,
        )
        .with_aggregate_decoded_limits(4 * 1024 * 1024, 400_000)
    }
}

#[derive(Debug, Default)]
pub(crate) struct CompileAggregate {
    work: u64,
    payload_bytes: u64,
    decoded_bytes: u64,
    decoded_cells: u64,
    live_bytes: Rc<Cell<u64>>,
    result_bytes: u64,
}

/// Capability supplied to a codec while a registry snapshot is frozen.
pub struct RegistryBuildScope<'a> {
    format: &'a FormatId,
    limits: RegistryBuildLimits,
    aggregate: &'a mut CompileAggregate,
    counters: ScopeCounters,
    live: Rc<OptionLiveState>,
}

impl<'a> RegistryBuildScope<'a> {
    pub(crate) fn new(
        format: &'a FormatId,
        limits: RegistryBuildLimits,
        aggregate: &'a mut CompileAggregate,
    ) -> Self {
        Self {
            format,
            limits,
            aggregate,
            counters: ScopeCounters::default(),
            live: Rc::new(OptionLiveState::default()),
        }
    }

    pub const fn format(&self) -> &FormatId {
        self.format
    }

    pub fn charge_work(&mut self, units: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.work,
            &mut self.aggregate.work,
            units,
            self.limits.per_codec.max_work,
            self.limits.max_total_work,
            OptionBudget::Work,
        )
    }

    pub fn charge_decoded_bytes(&mut self, bytes: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.decoded_bytes,
            &mut self.aggregate.decoded_bytes,
            bytes,
            self.limits.per_codec.max_decoded_bytes,
            self.limits.max_total_decoded_bytes,
            OptionBudget::DecodedBytes,
        )
    }

    pub fn charge_decoded_cells(&mut self, cells: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.decoded_cells,
            &mut self.aggregate.decoded_cells,
            cells,
            self.limits.per_codec.max_decoded_cells,
            self.limits.max_total_decoded_cells,
            OptionBudget::DecodedCells,
        )
    }

    pub fn check_recursion_depth(&self, depth: u32) -> Result<(), OptionCompileError> {
        check_depth(depth, self.limits.per_codec.max_recursion_depth)
    }

    /// Reserves decoder-owned live capacity until the token is dropped or this scope closes.
    pub fn reserve_live(
        &mut self,
        bytes: u64,
    ) -> Result<OptionLiveReservation, OptionCompileError> {
        reserve_live(
            &self.live,
            &self.aggregate.live_bytes,
            bytes,
            self.limits.per_codec.max_live_bytes,
            self.limits.max_total_live_bytes,
        )
    }

    /// Closes this one-shot scope and returns a value only after all reservations are released.
    pub fn finish<T: Any + Send + Sync + RefUnwindSafe>(
        &mut self,
        value: T,
        retained_bytes: u64,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        self.live.finish(&self.aggregate.live_bytes)?;
        charge(
            &mut self.counters.result_bytes,
            &mut self.aggregate.result_bytes,
            retained_bytes,
            self.limits.per_codec.max_result_bytes,
            self.limits.max_total_result_bytes,
            OptionBudget::ResultBytes,
        )?;
        Ok(CompiledFormatOptions::new(value, retained_bytes))
    }
}

impl Drop for RegistryBuildScope<'_> {
    fn drop(&mut self) {
        self.live.close(&self.aggregate.live_bytes);
    }
}

/// Capability supplied to a codec for one submitted request target.
pub struct OptionCompileScope<'a> {
    format: &'a FormatId,
    limits: OptionCompileLimits,
    aggregate: &'a mut CompileAggregate,
    counters: ScopeCounters,
    live: Rc<OptionLiveState>,
}

impl<'a> OptionCompileScope<'a> {
    pub(crate) fn new(
        format: &'a FormatId,
        limits: OptionCompileLimits,
        aggregate: &'a mut CompileAggregate,
    ) -> Self {
        Self {
            format,
            limits,
            aggregate,
            counters: ScopeCounters::default(),
            live: Rc::new(OptionLiveState::default()),
        }
    }

    pub const fn format(&self) -> &FormatId {
        self.format
    }

    pub fn charge_work(&mut self, units: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.work,
            &mut self.aggregate.work,
            units,
            self.limits.per_target.max_work,
            self.limits.max_total_work,
            OptionBudget::Work,
        )
    }

    pub fn charge_decoded_bytes(&mut self, bytes: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.decoded_bytes,
            &mut self.aggregate.decoded_bytes,
            bytes,
            self.limits.per_target.max_decoded_bytes,
            self.limits.max_total_decoded_bytes,
            OptionBudget::DecodedBytes,
        )
    }

    pub fn charge_decoded_cells(&mut self, cells: u64) -> Result<(), OptionCompileError> {
        charge(
            &mut self.counters.decoded_cells,
            &mut self.aggregate.decoded_cells,
            cells,
            self.limits.per_target.max_decoded_cells,
            self.limits.max_total_decoded_cells,
            OptionBudget::DecodedCells,
        )
    }

    pub fn check_recursion_depth(&self, depth: u32) -> Result<(), OptionCompileError> {
        check_depth(depth, self.limits.per_target.max_recursion_depth)
    }

    /// Reserves decoder-owned live capacity until the token is dropped or this scope closes.
    pub fn reserve_live(
        &mut self,
        bytes: u64,
    ) -> Result<OptionLiveReservation, OptionCompileError> {
        reserve_live(
            &self.live,
            &self.aggregate.live_bytes,
            bytes,
            self.limits.per_target.max_live_bytes,
            self.limits.max_total_live_bytes,
        )
    }

    pub fn reader<'scope, 'payload>(
        &'scope mut self,
        payload: FormatOptionPayload<'payload>,
    ) -> FormatOptionReader<'scope, 'payload, 'a> {
        FormatOptionReader {
            scope: self,
            payload: payload.bytes,
            cursor: 0,
        }
    }

    /// Closes this one-shot scope and returns a value only after all reservations are released.
    pub fn finish<T: Any + Send + Sync + RefUnwindSafe>(
        &mut self,
        value: T,
        retained_bytes: u64,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        self.live.finish(&self.aggregate.live_bytes)?;
        charge(
            &mut self.counters.result_bytes,
            &mut self.aggregate.result_bytes,
            retained_bytes,
            self.limits.per_target.max_result_bytes,
            self.limits.max_total_result_bytes,
            OptionBudget::ResultBytes,
        )?;
        Ok(CompiledFormatOptions::new(value, retained_bytes))
    }
}

impl Drop for OptionCompileScope<'_> {
    fn drop(&mut self) {
        self.live.close(&self.aggregate.live_bytes);
    }
}

/// Opaque handle to one precedence layer's serialized payload.
#[derive(Clone, Copy)]
pub struct FormatOptionPayload<'a> {
    bytes: &'a [u8],
}

impl fmt::Debug for FormatOptionPayload<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FormatOptionPayload")
            .field("length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Metered range reader for a submitted option payload.
pub struct FormatOptionReader<'scope, 'payload, 'budget> {
    scope: &'scope mut OptionCompileScope<'budget>,
    payload: &'payload [u8],
    cursor: usize,
}

/// Linear reservation for temporary option-decoder capacity.
///
/// The token is intentionally neither cloneable nor transferable across threads.
/// Closing its issuing compile scope invalidates it, so a later drop has no accounting effect.
///
/// ```compile_fail
/// use baozi_import::OptionLiveReservation;
///
/// fn clone_reservation(reservation: OptionLiveReservation) {
///     let _duplicate = reservation.clone();
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::OptionLiveReservation;
///
/// fn require_send<T: Send>() {}
/// require_send::<OptionLiveReservation>();
/// ```
#[must_use = "dropping the reservation immediately releases the admitted capacity"]
pub struct OptionLiveReservation {
    scope: Rc<OptionLiveState>,
    aggregate: Rc<Cell<u64>>,
    bytes: u64,
}

impl OptionLiveReservation {
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl fmt::Debug for OptionLiveReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OptionLiveReservation")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl Drop for OptionLiveReservation {
    fn drop(&mut self) {
        self.scope.release(&self.aggregate, self.bytes);
    }
}

impl<'payload> FormatOptionReader<'_, 'payload, '_> {
    pub const fn remaining(&self) -> usize {
        self.payload.len() - self.cursor
    }

    pub fn read_exact(&mut self, length: usize) -> Result<&'payload [u8], OptionCompileError> {
        let end = self
            .cursor
            .checked_add(length)
            .filter(|end| *end <= self.payload.len())
            .ok_or_else(|| {
                OptionCompileError::invalid_payload("truncated", "payload is truncated")
            })?;
        let length_u64 = u64::try_from(length).map_err(|_| OptionCompileError::BudgetExceeded {
            budget: OptionBudget::PayloadBytes,
        })?;
        charge(
            &mut self.scope.counters.payload_bytes,
            &mut self.scope.aggregate.payload_bytes,
            length_u64,
            self.scope.limits.per_target.max_payload_bytes,
            self.scope.limits.max_total_payload_bytes,
            OptionBudget::PayloadBytes,
        )?;
        self.scope.charge_work(length_u64.max(1))?;
        let bytes = &self.payload[self.cursor..end];
        self.cursor = end;
        Ok(bytes)
    }

    pub fn read_remaining(&mut self) -> Result<&'payload [u8], OptionCompileError> {
        self.read_exact(self.remaining())
    }
}

/// Opaque, precedence-ordered layers supplied to one format codec.
#[derive(Debug, Clone, Copy)]
pub struct FormatOptionLayers<'a> {
    preset: Option<FormatOptionPayload<'a>>,
    caller: Option<FormatOptionPayload<'a>>,
}

impl<'a> FormatOptionLayers<'a> {
    pub(crate) const fn new(
        preset: Option<FormatOptionPayload<'a>>,
        caller: Option<FormatOptionPayload<'a>>,
    ) -> Self {
        Self { preset, caller }
    }

    pub const fn preset(&self) -> Option<FormatOptionPayload<'a>> {
        self.preset
    }

    pub const fn caller(&self) -> Option<FormatOptionPayload<'a>> {
        self.caller
    }
}

/// Type-erased immutable options whose concrete identity was derived by Baozi.
pub struct CompiledFormatOptions {
    schema: Option<FormatOptionSchema>,
    runtime: OptionRuntimeTypeKey,
    value: Arc<dyn Any + Send + Sync + RefUnwindSafe>,
    retained_bytes: u64,
}

impl CompiledFormatOptions {
    fn new<T: Any + Send + Sync + RefUnwindSafe>(value: T, retained_bytes: u64) -> Self {
        Self {
            schema: None,
            runtime: OptionRuntimeTypeKey::of::<T>(),
            value: Arc::new(value),
            retained_bytes,
        }
    }

    /// Seals the registry-owned schema after a codec returns its erased value.
    ///
    /// Codecs cannot choose or relabel this binding: the registry supplies the
    /// descriptor captured for the registration that invoked the codec.
    pub(crate) fn seal_schema(mut self, schema: Option<FormatOptionSchema>) -> Self {
        self.schema = schema;
        self
    }

    pub const fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        self.runtime
    }

    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    pub fn downcast_ref<T: Any + Send + Sync + RefUnwindSafe>(
        &self,
        key: &FormatOptionKey<T>,
    ) -> Option<&T> {
        (self.runtime == key.runtime && self.schema.as_ref() == Some(key.schema()))
            .then(|| (self.value.as_ref() as &dyn Any).downcast_ref::<T>())
            .flatten()
    }
}

impl fmt::Debug for CompiledFormatOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledFormatOptions")
            .field("schema", &self.schema)
            .field("runtime", &self.runtime)
            .field("retained_bytes", &self.retained_bytes)
            .finish_non_exhaustive()
    }
}

/// Object-safe format-owned compilation capability.
pub trait FormatOptionCodec: Send + Sync + RefUnwindSafe + 'static {
    fn schema(&self) -> Option<FormatOptionSchema>;

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError>;

    fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &CompiledFormatOptions,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError>;
}

/// Stable resource dimension exceeded during bounded option compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionBudget {
    Work,
    PayloadBytes,
    DecodedBytes,
    DecodedCells,
    RecursionDepth,
    LiveBytes,
    ResultBytes,
}

/// Bounded codec failure shared by registry-build and request phases.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptionCompileError {
    /// Host-owned budget evidence emitted only by a compile scope.
    ///
    /// ```compile_fail
    /// use baozi_import::{OptionBudget, OptionCompileError};
    ///
    /// let _forged = OptionCompileError::BudgetExceeded {
    ///     budget: OptionBudget::Work,
    /// };
    /// ```
    #[non_exhaustive]
    BudgetExceeded {
        budget: OptionBudget,
    },
    /// The codec returned while temporary decoder capacity was still owned.
    #[non_exhaustive]
    LiveReservationsRemain {
        bytes: u64,
    },
    /// The codec retained zero-byte linear reservation capabilities.
    #[non_exhaustive]
    ActiveReservationsRemain {
        count: u64,
    },
    /// The codec attempted to reuse its one-shot compilation scope.
    #[non_exhaustive]
    ScopeClosed {},
    OptionsUnsupported,
    /// Codec-owned invalid-input evidence created through the bounded factory.
    ///
    /// ```compile_fail
    /// use baozi_import::OptionCompileError;
    ///
    /// let _unbounded = OptionCompileError::InvalidPayload {
    ///     code: "forged".into(),
    ///     detail: "unbounded".repeat(1_000_000).into(),
    /// };
    /// ```
    #[non_exhaustive]
    InvalidPayload {
        code: OptionErrorCode,
        detail: BoundedFaultDetail,
    },
}

impl OptionCompileError {
    pub fn invalid_payload(code: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::InvalidPayload {
            code: bounded_code(code.as_ref()),
            detail: BoundedFaultDetail::new(detail.as_ref()),
        }
    }
}

impl fmt::Display for OptionCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExceeded { budget } => {
                write!(formatter, "format option compilation exceeded {budget:?}")
            }
            Self::LiveReservationsRemain { bytes } => {
                write!(
                    formatter,
                    "{bytes} format option live bytes remain reserved"
                )
            }
            Self::ActiveReservationsRemain { count } => {
                write!(
                    formatter,
                    "{count} format option live reservations remain active"
                )
            }
            Self::ScopeClosed {} => {
                formatter.write_str("format option compilation scope is closed")
            }
            Self::OptionsUnsupported => formatter.write_str("format does not accept option layers"),
            Self::InvalidPayload { code, detail } => {
                write!(formatter, "invalid format options ({code}): {detail}")
            }
        }
    }
}

impl std::error::Error for OptionCompileError {}

/// A validated, bounded format-option error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionErrorCode(Option<String>);

impl OptionErrorCode {
    pub fn as_str(&self) -> &str {
        self.0.as_deref().unwrap_or("invalid-code")
    }
}

impl AsRef<str> for OptionErrorCode {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for OptionErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Concrete default used by every importer without configurable options.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct NoFormatOptions;

#[derive(Debug)]
struct NoOptionsCodec;

impl FormatOptionCodec for NoOptionsCodec {
    fn schema(&self) -> Option<FormatOptionSchema> {
        None
    }

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        no_options_runtime_type_key()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        scope.charge_work(1)?;
        scope.finish(NoFormatOptions, 0)
    }

    fn compile_layers(
        &self,
        _scope: &mut OptionCompileScope<'_>,
        _defaults: &CompiledFormatOptions,
        _layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        Err(OptionCompileError::OptionsUnsupported)
    }
}

pub(crate) fn no_options_codec() -> Arc<dyn FormatOptionCodec> {
    static CODEC: OnceLock<Arc<dyn FormatOptionCodec>> = OnceLock::new();
    Arc::clone(CODEC.get_or_init(|| Arc::new(NoOptionsCodec)))
}

pub(crate) fn no_options_runtime_type_key() -> OptionRuntimeTypeKey {
    OptionRuntimeTypeKey::of::<NoFormatOptions>()
}

#[derive(Debug, Default)]
struct ScopeCounters {
    work: u64,
    payload_bytes: u64,
    decoded_bytes: u64,
    decoded_cells: u64,
    result_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum OptionLiveStatus {
    #[default]
    Open,
    Closed,
    Rejected {
        bytes: u64,
        reservations: u64,
    },
}

#[derive(Debug, Default)]
struct OptionLiveState {
    bytes: Cell<u64>,
    reservations: Cell<u64>,
    status: Cell<OptionLiveStatus>,
}

impl OptionLiveState {
    fn finish(&self, aggregate: &Cell<u64>) -> Result<(), OptionCompileError> {
        match self.status.get() {
            OptionLiveStatus::Open => {
                let (bytes, reservations) = self.release_all(aggregate);
                if bytes == 0 && reservations == 0 {
                    self.status.set(OptionLiveStatus::Closed);
                    Ok(())
                } else {
                    self.status.set(OptionLiveStatus::Rejected {
                        bytes,
                        reservations,
                    });
                    Err(live_reservation_error(bytes, reservations))
                }
            }
            OptionLiveStatus::Closed => Err(OptionCompileError::ScopeClosed {}),
            OptionLiveStatus::Rejected {
                bytes,
                reservations,
            } => Err(live_reservation_error(bytes, reservations)),
        }
    }

    fn close(&self, aggregate: &Cell<u64>) {
        if self.status.get() != OptionLiveStatus::Open {
            return;
        }
        let (bytes, reservations) = self.release_all(aggregate);
        let status = if bytes == 0 && reservations == 0 {
            OptionLiveStatus::Closed
        } else {
            OptionLiveStatus::Rejected {
                bytes,
                reservations,
            }
        };
        self.status.set(status);
    }

    fn release(&self, aggregate: &Cell<u64>, bytes: u64) {
        if self.status.get() != OptionLiveStatus::Open {
            return;
        }
        let Some(local) = self.bytes.get().checked_sub(bytes) else {
            return;
        };
        let Some(reservations) = self.reservations.get().checked_sub(1) else {
            return;
        };
        self.bytes.set(local);
        self.reservations.set(reservations);
        release_aggregate_live(aggregate, bytes);
    }

    fn release_all(&self, aggregate: &Cell<u64>) -> (u64, u64) {
        let bytes = self.bytes.replace(0);
        let reservations = self.reservations.replace(0);
        release_aggregate_live(aggregate, bytes);
        (bytes, reservations)
    }
}

fn release_aggregate_live(aggregate: &Cell<u64>, bytes: u64) {
    let remaining = aggregate.get().checked_sub(bytes).unwrap_or(u64::MAX);
    aggregate.set(remaining);
}

fn live_reservation_error(bytes: u64, reservations: u64) -> OptionCompileError {
    if bytes == 0 {
        OptionCompileError::ActiveReservationsRemain {
            count: reservations,
        }
    } else {
        OptionCompileError::LiveReservationsRemain { bytes }
    }
}

fn charge(
    local: &mut u64,
    aggregate: &mut u64,
    amount: u64,
    local_limit: u64,
    aggregate_limit: u64,
    budget: OptionBudget,
) -> Result<(), OptionCompileError> {
    let next_local = local
        .checked_add(amount)
        .filter(|value| *value <= local_limit)
        .ok_or(OptionCompileError::BudgetExceeded { budget })?;
    let next_aggregate = aggregate
        .checked_add(amount)
        .filter(|value| *value <= aggregate_limit)
        .ok_or(OptionCompileError::BudgetExceeded { budget })?;
    *local = next_local;
    *aggregate = next_aggregate;
    Ok(())
}

fn reserve_live(
    scope: &Rc<OptionLiveState>,
    aggregate: &Rc<Cell<u64>>,
    bytes: u64,
    local_limit: u64,
    aggregate_limit: u64,
) -> Result<OptionLiveReservation, OptionCompileError> {
    if scope.status.get() != OptionLiveStatus::Open {
        return Err(OptionCompileError::ScopeClosed {});
    }
    let next_local = scope
        .bytes
        .get()
        .checked_add(bytes)
        .filter(|value| *value <= local_limit)
        .ok_or(OptionCompileError::BudgetExceeded {
            budget: OptionBudget::LiveBytes,
        })?;
    let next_aggregate = aggregate
        .get()
        .checked_add(bytes)
        .filter(|value| *value <= aggregate_limit)
        .ok_or(OptionCompileError::BudgetExceeded {
            budget: OptionBudget::LiveBytes,
        })?;
    let next_reservations =
        scope
            .reservations
            .get()
            .checked_add(1)
            .ok_or(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::LiveBytes,
            })?;
    scope.bytes.set(next_local);
    scope.reservations.set(next_reservations);
    aggregate.set(next_aggregate);
    Ok(OptionLiveReservation {
        scope: Rc::clone(scope),
        aggregate: Rc::clone(aggregate),
        bytes,
    })
}

fn check_depth(depth: u32, maximum: u32) -> Result<(), OptionCompileError> {
    if depth > maximum {
        Err(OptionCompileError::BudgetExceeded {
            budget: OptionBudget::RecursionDepth,
        })
    } else {
        Ok(())
    }
}

fn invalid_local_id_offset(value: &str) -> Option<usize> {
    let mut previous_separator = true;
    for (offset, byte) in value.bytes().enumerate() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => previous_separator = false,
            b'-' if !previous_separator => previous_separator = true,
            _ => return Some(offset),
        }
    }
    previous_separator.then_some(value.len().saturating_sub(1))
}

fn bounded_code(value: &str) -> OptionErrorCode {
    if value.is_empty()
        || value.len() > MAX_ERROR_CODE_BYTES
        || invalid_local_id_offset(value).is_some()
    {
        return OptionErrorCode(None);
    }
    let mut code = String::new();
    if code.try_reserve_exact(value.len()).is_err() {
        return OptionErrorCode(None);
    }
    code.push_str(value);
    OptionErrorCode(Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct TestOptions(u8);

    fn key() -> FormatOptionKey<TestOptions> {
        FormatOptionKey::new(FormatOptionSchema::new(
            FormatOptionSchemaKey::new(
                FormatId::new("org.example.test").unwrap(),
                FormatOptionSchemaId::new("import-options").unwrap(),
            ),
            NonZeroU32::new(1).unwrap(),
        ))
    }

    #[test]
    fn schema_id_storage_and_cloning_are_inline() {
        fn assert_inline(value: &FormatOptionSchemaId) {
            let object_start = std::ptr::from_ref(value).addr();
            let object_end = object_start + std::mem::size_of_val(value);
            let string_start = value.as_str().as_ptr().addr();
            let string_end = string_start + value.as_str().len();
            assert!(string_start >= object_start);
            assert!(string_end <= object_end);
        }

        let value = FormatOptionSchemaId::new("import-options").unwrap();
        let cloned = value.clone();

        assert_eq!(std::mem::size_of::<FormatOptionSchemaId>(), 65);
        assert!(!std::mem::needs_drop::<FormatOptionSchemaId>());
        assert_eq!(cloned, value);
        assert_inline(&value);
        assert_inline(&cloned);
    }

    #[test]
    fn schema_id_enforces_boundary_length_and_canonical_grammar() {
        let maximum = "a".repeat(MAX_SCHEMA_ID_BYTES);
        assert_eq!(
            FormatOptionSchemaId::new(&maximum).unwrap().as_str(),
            maximum
        );
        let oversized = "a".repeat(MAX_SCHEMA_ID_BYTES + 1);
        assert_eq!(
            FormatOptionSchemaId::new(&oversized),
            Err(FormatOptionSchemaIdError::TooLong {
                actual: MAX_SCHEMA_ID_BYTES + 1,
                maximum: MAX_SCHEMA_ID_BYTES,
            })
        );

        for (value, offset) in [
            ("", 0),
            ("-options", 0),
            ("import--options", 7),
            ("import-options-", 14),
            ("Import-options", 0),
        ] {
            let expected = if value.is_empty() {
                FormatOptionSchemaIdError::Empty
            } else {
                FormatOptionSchemaIdError::InvalidByte { offset }
            };
            assert_eq!(FormatOptionSchemaId::new(value), Err(expected));
        }
    }

    #[test]
    fn typed_factory_seals_the_target_owner() {
        let key = key();
        let envelope = key.envelope_from_canonical_payload([7]).unwrap();
        assert_eq!(envelope.target(), key.schema().key().owner());
        assert_eq!(envelope.schema(), key.schema().key());
    }

    #[test]
    fn payload_capacity_overflow_is_a_typed_envelope_error() {
        let mut payload = Vec::new();
        assert!(matches!(
            reserve_envelope_payload(&mut payload, usize::MAX),
            Err(FormatOptionEnvelopeError::AllocationFailed)
        ));
    }

    #[test]
    fn option_debug_views_do_not_bypass_metered_payload_reads() {
        let key = key();
        let envelope = key
            .envelope_from_canonical_payload([222, 173, 190, 239])
            .unwrap();
        let envelope_debug = format!("{envelope:?}");
        let payload_debug = format!("{:?}", envelope.payload());

        assert!(envelope_debug.contains("payload_len: 4"));
        assert!(payload_debug.contains("length: 4"));
        for byte in ["222", "173", "190", "239"] {
            assert!(!envelope_debug.contains(byte));
            assert!(!payload_debug.contains(byte));
        }
    }

    #[test]
    fn compiled_option_storage_is_ref_unwind_safe() {
        fn require_ref_unwind_safe<T: RefUnwindSafe>() {}

        require_ref_unwind_safe::<CompiledFormatOptions>();
        require_ref_unwind_safe::<FormatOptionKey<TestOptions>>();
    }

    #[test]
    fn compiled_value_uses_the_actual_concrete_type() {
        let format = FormatId::new("org.example.test").unwrap();
        let mut aggregate = CompileAggregate::default();
        let mut scope =
            RegistryBuildScope::new(&format, RegistryBuildLimits::default(), &mut aggregate);
        let compiled = scope
            .finish(TestOptions(7), 1)
            .unwrap()
            .seal_schema(Some(key().schema().clone()));
        assert_eq!(compiled.downcast_ref(&key()), Some(&TestOptions(7)));
        assert_eq!(
            compiled.runtime_type_key(),
            OptionRuntimeTypeKey::of::<TestOptions>()
        );
    }

    #[test]
    fn compiled_value_requires_the_exact_frozen_schema() {
        let format = FormatId::new("org.example.test").unwrap();
        let mut aggregate = CompileAggregate::default();
        let mut scope =
            RegistryBuildScope::new(&format, RegistryBuildLimits::default(), &mut aggregate);
        let compiled = scope
            .finish(TestOptions(7), 1)
            .unwrap()
            .seal_schema(Some(key().schema().clone()));
        drop(scope);
        let wrong_key = FormatOptionKey::<TestOptions>::new(FormatOptionSchema::new(
            FormatOptionSchemaKey::new(format, FormatOptionSchemaId::new("other-options").unwrap()),
            NonZeroU32::new(1).unwrap(),
        ));

        assert_eq!(compiled.downcast_ref(&wrong_key), None);
    }

    #[test]
    fn scopes_enforce_per_codec_and_aggregate_budgets() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits = RegistryBuildLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 1, 1), 1, 1, 1);
        let mut aggregate = CompileAggregate::default();
        let mut scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
        scope.charge_work(1).unwrap();
        assert_eq!(
            scope.charge_work(1),
            Err(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::Work
            })
        );
        assert!(scope.finish(TestOptions(1), 1).is_ok());
    }

    #[test]
    fn decoded_budgets_aggregate_across_registry_scopes() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits =
            RegistryBuildLimits::new(OptionScopeLimits::new(8, 0, 1, 1, 0, 0, 0), 16, 0, 0)
                .with_aggregate_decoded_limits(1, 1);
        let mut aggregate = CompileAggregate::default();
        {
            let mut first = RegistryBuildScope::new(&format, limits, &mut aggregate);
            first.charge_decoded_bytes(1).unwrap();
            first.charge_decoded_cells(1).unwrap();
        }
        let mut second = RegistryBuildScope::new(&format, limits, &mut aggregate);
        assert!(matches!(
            second.charge_decoded_bytes(1),
            Err(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::DecodedBytes
            })
        ));
        assert!(matches!(
            second.charge_decoded_cells(1),
            Err(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::DecodedCells
            })
        ));
    }

    #[test]
    fn live_capacity_is_linear_and_late_drop_is_inert_after_scope_close() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits = RegistryBuildLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 1, 1), 1, 1, 1);
        let mut aggregate = CompileAggregate::default();
        let late_reservation = {
            let mut scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
            let reservation = scope.reserve_live(1).unwrap();
            assert_eq!(reservation.bytes(), 1);
            assert!(matches!(
                scope.reserve_live(1),
                Err(OptionCompileError::BudgetExceeded {
                    budget: OptionBudget::LiveBytes
                })
            ));
            drop(reservation);
            scope.reserve_live(1).unwrap()
        };

        let mut next_scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
        let current_reservation = next_scope.reserve_live(1).unwrap();
        drop(late_reservation);
        assert!(matches!(
            next_scope.reserve_live(1),
            Err(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::LiveBytes
            })
        ));
        drop(current_reservation);
        let released_reservation = next_scope.reserve_live(1).unwrap();
        drop(released_reservation);
        assert!(next_scope.finish(TestOptions(1), 1).is_ok());
    }

    #[test]
    fn forgotten_registry_live_capacity_rejects_result_and_restores_aggregate() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits = RegistryBuildLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 1, 1), 1, 1, 1);
        let mut aggregate = CompileAggregate::default();
        let (result, retry) = {
            let mut scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
            let reservation = scope.reserve_live(1).unwrap();
            std::mem::forget(reservation);
            let result = scope.finish(TestOptions(1), 1);
            let retry = scope.finish(TestOptions(1), 1);
            (result, retry)
        };

        assert!(matches!(
            result,
            Err(OptionCompileError::LiveReservationsRemain { bytes: 1 })
        ));
        assert!(matches!(
            retry,
            Err(OptionCompileError::LiveReservationsRemain { bytes: 1 })
        ));
        let mut next_scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
        assert!(next_scope.reserve_live(1).is_ok());
    }

    #[test]
    fn forgotten_request_live_capacity_rejects_result_and_restores_aggregate() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits =
            OptionCompileLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 1, 1), 1, 1, 0, 1, 1);
        let mut aggregate = CompileAggregate::default();
        let result = {
            let mut scope = OptionCompileScope::new(&format, limits, &mut aggregate);
            let reservation = scope.reserve_live(1).unwrap();
            std::mem::forget(reservation);
            scope.finish(TestOptions(1), 1)
        };

        assert!(matches!(
            result,
            Err(OptionCompileError::LiveReservationsRemain { bytes: 1 })
        ));
        let mut next_scope = OptionCompileScope::new(&format, limits, &mut aggregate);
        assert!(next_scope.reserve_live(1).is_ok());
    }

    #[test]
    fn forgotten_zero_byte_reservation_rejects_result() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits = RegistryBuildLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 1, 1), 1, 1, 1);
        let mut aggregate = CompileAggregate::default();
        {
            let mut scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
            let reservation = scope.reserve_live(0).unwrap();
            drop(reservation);
            assert!(scope.finish(TestOptions(1), 0).is_ok());
        }
        let (result, retry) = {
            let mut scope = RegistryBuildScope::new(&format, limits, &mut aggregate);
            let reservation = scope.reserve_live(0).unwrap();
            std::mem::forget(reservation);
            let result = scope.finish(TestOptions(1), 1);
            let retry = scope.finish(TestOptions(1), 1);
            (result, retry)
        };

        assert!(matches!(
            result,
            Err(OptionCompileError::ActiveReservationsRemain { count: 1 })
        ));
        assert!(matches!(
            retry,
            Err(OptionCompileError::ActiveReservationsRemain { count: 1 })
        ));
    }

    #[test]
    fn request_live_capacity_is_released_during_unwind() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits =
            OptionCompileLimits::new(OptionScopeLimits::new(1, 0, 0, 0, 0, 2, 1), 1, 1, 0, 2, 1);
        let mut aggregate = CompileAggregate::default();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut scope = OptionCompileScope::new(&format, limits, &mut aggregate);
            let _reservation = scope.reserve_live(2).unwrap();
            panic!("release reservation");
        }));
        assert!(unwind.is_err());

        let mut next_scope = OptionCompileScope::new(&format, limits, &mut aggregate);
        assert!(next_scope.reserve_live(2).is_ok());
    }

    #[test]
    fn payload_reader_charges_every_observed_byte() {
        let format = FormatId::new("org.example.test").unwrap();
        let limits =
            OptionCompileLimits::new(OptionScopeLimits::new(8, 2, 8, 8, 8, 8, 8), 1, 8, 2, 8, 8);
        let mut aggregate = CompileAggregate::default();
        let mut scope = OptionCompileScope::new(&format, limits, &mut aggregate);
        let payload = FormatOptionPayload { bytes: &[1, 2, 3] };
        let mut reader = scope.reader(payload);
        assert_eq!(reader.read_exact(2).unwrap(), [1, 2]);
        assert_eq!(
            reader.read_exact(1),
            Err(OptionCompileError::BudgetExceeded {
                budget: OptionBudget::PayloadBytes
            })
        );
    }
}
