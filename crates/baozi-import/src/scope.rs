use crate::config::{ConformancePolicy, ImportLossKind, LossPolicy};
use crate::dependency::{
    AuthorizedDependency, CachedDependencyFailure, DependencyCacheDisposition, DependencyIdentity,
    DependencyOrigin, DependencyOriginIssuer, DependencyOriginNode, DependencyOutcome,
    DependencyReference, DependencyRequirement, DependencyRoleId, DependencySnapshot,
    DependencySnapshotIdentity, OptionalDependencyEvidence, PendingNegativeObservation,
    ResolverBinding, SharedDependencySnapshot, shared_dependency_snapshot_allocation_bytes,
};
use crate::descriptor::TextDecodingMode;
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCause, DiagnosticCode, DiagnosticDetailSink,
    DiagnosticDropReason, DiagnosticLineColumn, DiagnosticPrepareCause, DiagnosticSeverity,
    DiagnosticSink, DiagnosticSource, PreparedDiagnostic, valid_diagnostic_code,
};
use crate::error::{
    BoundedFormatFailureDetailSink, FormatFailureDetailCause, FormatFailureEnvelope,
    FormatFailureEvidence, FormatFailureEvidenceProducer, ScopeAbort, ScopeAbortCause,
    TrustedExtensionBoundary,
};
use crate::format::discard_untrusted_panic_payload;
use crate::ledger::{ImportDimension, ImportLedgerPhase, ImportProgressPhase, ImportProgressUnit};
use crate::limits::ResourceLimits;
use crate::loss::{ImportLossDisposition, ImportLossEvidence, ImportLossSubject};
use crate::metered_storage::{
    AccountedVec, MeteredDecodedText, MeteredOpaqueAllowance, MeteredOrderedMap, MeteredString,
    MeteredVec, StorageMeter, metered_replacement_plans_with_element_size,
    metered_vec_replacement_plans,
};
use crate::options::{CompiledFormatOptions, FormatOptionKey};
use crate::registry::RegistryEntry;
use crate::resource_graph::{
    AvailableResourceIdentity, PendingResourceRelation, ResourceAcquisitionState, ResourceGraph,
    ResourceGraphError, ResourceUnavailableDisposition, cached_unavailable_disposition,
    registered_negative_proof_canonical_len, unavailable_disposition,
};
use crate::source_scope::{
    FinalizedSourceScopeRecord, LiveSourceScopeRecord, SourceScopeRecordBuilder,
};
use crate::store::{
    ImportObservedWorkFailure, ImportStoreHost, ImportStoreHostError, ImportStoreReservation,
    ImportStoreSessionOwner, scene_validation_limits,
};
use crate::text_input::{
    ConformedTextInput, TextConformanceResult, TextInputOptions, TextInputRecoveryTracker,
};
use crate::{SceneDraft as CanonicalSceneDraft, authoring::SceneAuthoringBatch};
use baozi_core::scene::integration::SceneConstruction;
use baozi_core::{OrderedMap, OrderedMapError};
use baozi_io::{
    __semantic_identity::{
        RegisteredNegativeProofError, RegisteredResolverProfile, RegisteredUnavailableCause,
        StableResourceIdentitySchema,
    },
    AssetIo, AssetLength, AssetOpenErrorKind, AssetPath, AssetReadErrorKind, AuthorityDomain,
    OpenedAsset, PathError,
};
use baozi_operation::{CapacityPlan, CapacityPlanError, CounterClass, LedgerCause};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;

pub type ParseResult<T> = Result<T, ParseFailure>;
type AssetSnapshotResult = Result<AccountedVec<u8>, ScopeAbortCause>;

enum NegativeProofActionError {
    Store(ImportStoreHostError),
    Proof(RegisteredNegativeProofError),
}

fn registered_negative_proof_error(cause: RegisteredNegativeProofError) -> ScopeAbortCause {
    let code = match cause {
        RegisteredNegativeProofError::AuthorityMismatch => {
            "registered-negative-proof-authority-mismatch"
        }
        RegisteredNegativeProofError::TargetMismatch => "registered-negative-proof-target-mismatch",
        RegisteredNegativeProofError::CauseMismatch => "registered-negative-proof-cause-mismatch",
        RegisteredNegativeProofError::IdentitySchemaMismatch => {
            "registered-negative-proof-identity-schema-mismatch"
        }
        RegisteredNegativeProofError::TargetAvailable => {
            "registered-negative-proof-target-available"
        }
        _ => "registered-negative-proof-unrecognized-failure",
    };
    ScopeAbortCause::DescriptorContract { code }
}

/// Opaque parser transport. Only a selected [`ParseScope`] can create it.
#[derive(Debug)]
pub struct ParseFailure {
    kind: ParseFailureKind,
}

#[derive(Debug)]
pub(crate) enum ParseFailureKind {
    Scope,
    Format(FormatFailureEnvelope),
}

impl ParseFailure {
    fn scope() -> Self {
        Self {
            kind: ParseFailureKind::Scope,
        }
    }

    fn format(envelope: FormatFailureEnvelope) -> Self {
        Self {
            kind: ParseFailureKind::Format(envelope),
        }
    }

    pub(crate) fn into_kind(self) -> ParseFailureKind {
        self.kind
    }
}

impl fmt::Display for ParseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseFailureKind::Scope => formatter.write_str("parse scope aborted"),
            ParseFailureKind::Format(envelope) => envelope.fmt(formatter),
        }
    }
}

impl std::error::Error for ParseFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ParseFailureKind::Scope => None,
            ParseFailureKind::Format(envelope) => Some(envelope),
        }
    }
}

/// A typed parser-owned work or canonical-resource budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportWorkClass {
    WorkUnits,
    DataUriBytes,
    LineBytes,
    TokenBytes,
    StringBytes,
    Tokens,
    Records,
    Meshes,
    Vertices,
    Faces,
    GraphNodes,
    GraphEdges,
    GraphDepth,
    GraphWork,
    Primitives,
    AttributeStreams,
    Solids,
    DecodeWork,
    AttributeCells,
    TopologyCells,
    InfluenceCells,
    MorphCells,
    AnimationCells,
    CanonicalStringBytes,
    CanonicalPayloadBytes,
    ArchiveBytes,
    CompressedBytes,
    DecompressedBytes,
}

impl ImportWorkClass {
    fn accounting(self, limits: &ResourceLimits) -> (ImportDimension, Option<(&'static str, u64)>) {
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        match self {
            Self::WorkUnits => (ImportDimension::WorkUnits, None),
            Self::DataUriBytes => (ImportDimension::DataUriBytes, None),
            Self::LineBytes => (
                ImportDimension::DecodeWork,
                Some(("line-bytes", count(limits.max_line_bytes))),
            ),
            Self::TokenBytes => (
                ImportDimension::DecodeWork,
                Some(("token-bytes", count(limits.max_token_bytes))),
            ),
            Self::StringBytes => (
                ImportDimension::CanonicalStringBytes,
                Some(("string-bytes", count(limits.max_string_bytes))),
            ),
            Self::Tokens => (ImportDimension::Tokens, None),
            Self::Records => (ImportDimension::Records, None),
            Self::Meshes => (ImportDimension::Meshes, None),
            Self::Vertices => (ImportDimension::Vertices, None),
            Self::Faces => (ImportDimension::Faces, None),
            Self::GraphNodes => (ImportDimension::GraphNodes, None),
            Self::GraphEdges => (ImportDimension::GraphEdges, None),
            Self::GraphDepth => (ImportDimension::GraphDepth, None),
            Self::GraphWork => (ImportDimension::GraphWork, None),
            Self::Primitives => (ImportDimension::Primitives, None),
            Self::AttributeStreams => (ImportDimension::AttributeStreams, None),
            Self::Solids => (ImportDimension::Solids, None),
            Self::DecodeWork => (ImportDimension::DecodeWork, None),
            Self::AttributeCells => (ImportDimension::AttributeCells, None),
            Self::TopologyCells => (ImportDimension::TopologyCells, None),
            Self::InfluenceCells => (ImportDimension::InfluenceCells, None),
            Self::MorphCells => (ImportDimension::MorphCells, None),
            Self::AnimationCells => (ImportDimension::AnimationCells, None),
            Self::CanonicalStringBytes => (ImportDimension::CanonicalStringBytes, None),
            Self::CanonicalPayloadBytes => (ImportDimension::CanonicalPayloadBytes, None),
            Self::ArchiveBytes => (ImportDimension::ArchiveBytes, None),
            Self::CompressedBytes => (ImportDimension::CompressedBytes, None),
            Self::DecompressedBytes => (ImportDimension::DecompressedBytes, None),
        }
    }
}

/// Callback-scoped completion capability for one admitted parser work group.
///
/// The scope has no public constructor and cannot outlive [`ParseScope::with_work`] or
/// [`ParseScope::with_work_group`]. Each completion records one exact physical prefix across the
/// complete dimension group.
///
/// ```compile_fail
/// use baozi_import::{ImportWorkClass, ParseResult, ParseScope};
///
/// fn escape(scope: &ParseScope<'_>) {
///     let _escaped = scope.with_work(ImportWorkClass::WorkUnits, 1, |work| {
///         Ok::<_, baozi_import::ParseFailure>(work)
///     });
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::{ImportWorkClass, ParseScope};
///
/// fn legacy_permit(scope: &ParseScope<'_>) {
///     let _permit = scope.begin_work(ImportWorkClass::WorkUnits, 1);
/// }
/// ```
pub struct ImportWorkScope<'work, 'session, const N: usize> {
    scope: &'work ParseScope<'session>,
    dimensions: [ImportDimension; N],
    item_limits: [Option<(&'static str, u64)>; N],
    admitted: [u64; N],
    completed: [u64; N],
    parent: bool,
    active: bool,
    finished: bool,
}

impl ImportWorkScope<'_, '_, 1> {
    /// Admits the next chunk after the current chunk has completed.
    ///
    /// This keeps runtime-sized parser loops inside one callback-scoped capability without
    /// exposing an escaping permit or acquiring one ledger lock per physical unit.
    pub fn replenish(&mut self, amount: u64) -> ParseResult<()> {
        self.scope.preflight_active()?;
        if self.item_limits[0].is_some() {
            return Err(self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-work-item-limit-replenish-unsupported",
            }));
        }
        if self.remaining()[0] != 0 {
            return Err(self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-work-replenish-before-completion",
            }));
        }
        self.scope.poll_active()?;

        let dimension = self.dimensions[0];
        self.scope
            .host
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.scope.abort(cause.into_scope_abort_cause()))?;
        if let Some((resource, limit)) = self.item_limits[0]
            && amount > limit
        {
            return Err(self
                .scope
                .abort(ScopeAbortCause::ResourceLimit { resource, limit }));
        }

        self.admitted[0] = amount;
        self.completed[0] = 0;
        Ok(())
    }
}

impl<const N: usize> ImportWorkScope<'_, '_, N> {
    /// Returns the unfinished admitted suffix for every work entry.
    pub fn remaining(&self) -> [u64; N] {
        std::array::from_fn(|index| self.admitted[index] - self.completed[index])
    }

    /// Records one physically completed prefix across the complete work group.
    pub fn complete(&mut self, amounts: [u64; N]) -> ParseResult<()> {
        self.scope.preflight_active()?;
        for (index, amount) in amounts.into_iter().enumerate() {
            if amount > self.admitted[index] - self.completed[index] {
                return Err(self.scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-work-completion-exceeds-admitted",
                }));
            }
        }
        if amounts.into_iter().all(|amount| amount == 0) {
            return Ok(());
        }

        let mut charges: [(ImportDimension, u64); N] =
            std::array::from_fn(|index| (self.dimensions[index], amounts[index]));
        charges.sort_unstable_by_key(|(dimension, _)| *dimension);
        self.scope
            .host
            .charge_consumed_batch(&charges)
            .map_err(|cause| self.scope.abort(cause.into_scope_abort_cause()))?;
        for (completed, amount) in self.completed.iter_mut().zip(amounts) {
            *completed = completed.checked_add(amount).ok_or_else(|| {
                self.scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-work-completion-overflow",
                })
            })?;
        }
        Ok(())
    }

    /// Records the complete unfinished suffix.
    pub fn complete_all(&mut self) -> ParseResult<()> {
        self.complete(self.remaining())
    }

    /// Runs one fallible physical action and records its prefix only after normal completion.
    pub fn run<T, E>(
        &mut self,
        amounts: [u64; N],
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ParseFailure>,
    {
        let value = action()?;
        self.complete(amounts).map_err(E::from)?;
        Ok(value)
    }

    /// Runs one fallible physical action and records the complete unfinished suffix.
    pub fn run_all<T, E>(&mut self, action: impl FnOnce() -> Result<T, E>) -> Result<T, E>
    where
        E: From<ParseFailure>,
    {
        let remaining = self.remaining();
        self.run(remaining, action)
    }

    /// Polls cancellation and deadline state while this work group remains active.
    pub fn poll(&mut self) -> ParseResult<()> {
        self.scope.poll_active()
    }

    fn finish_exact(&mut self) -> ParseResult<()> {
        self.release();
        self.scope.poll_active()
    }

    fn finish_partial(&mut self) {
        self.release();
    }

    fn release(&mut self) {
        if self.active {
            if self.parent {
                self.scope.parent_work_outstanding.set(false);
            } else {
                self.scope.work_outstanding.set(false);
            }
            self.active = false;
        }
        self.finished = true;
    }
}

impl<const N: usize> Drop for ImportWorkScope<'_, '_, N> {
    fn drop(&mut self) {
        if !self.finished && self.active {
            let was_outstanding = if self.parent {
                self.scope.parent_work_outstanding.replace(false)
            } else {
                self.scope.work_outstanding.replace(false)
            };
            debug_assert!(
                was_outstanding && std::thread::panicking(),
                "the private parser work scope may only be abandoned by unwinding"
            );
        }
    }
}

pub(crate) struct DiagnosticLineIndex {
    authority: baozi_io::AuthorityDomain,
    source: AssetPath,
    _source_reservation: Option<ImportStoreReservation>,
    source_len: u64,
    line_starts: AccountedVec<u64>,
}

impl DiagnosticLineIndex {
    fn matches(&self, source: &DiagnosticSource<'_>, source_len: u64) -> bool {
        &self.authority == source.authority()
            && &self.source == source.path()
            && self.source_len == source_len
    }

    fn derive(&self, range: DiagnosticByteRange) -> Result<DiagnosticLineColumn, DiagnosticCause> {
        if range.end() > self.source_len {
            return Err(DiagnosticCause::ByteRangeOutOfBounds {
                end: range.end(),
                source_len: self.source_len,
            });
        }
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= range.start())
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        DiagnosticLineColumn::new(
            (line_index as u64).saturating_add(1),
            range.start().saturating_sub(line_start).saturating_add(1),
        )
    }
}

/// A capability-limited parser view valid only for one selected importer call.
pub struct ParseScope<'session> {
    entry: RegistryEntry,
    options: Arc<CompiledFormatOptions>,
    primary_io: &'session dyn AssetIo,
    primary_authority: &'session AuthorityDomain,
    primary_source: &'session AssetPath,
    primary_bytes: &'session [u8],
    resolver: ResolverBinding<'session>,
    dependency_origin_issuer: &'session DependencyOriginIssuer,
    conformance_policy: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: &'session ResourceLimits,
    store_owner: &'session ImportStoreSessionOwner,
    host: ImportStoreHost<'session>,
    diagnostics: &'session RefCell<DiagnosticSink>,
    diagnostic_line_indexes: &'session RefCell<AccountedVec<DiagnosticLineIndex>>,
    resource_graph: &'session RefCell<ResourceGraph>,
    optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    format_failure_storage: &'session RefCell<Option<ImportStoreReservation>>,
    dependency_failure: RefCell<Option<DependencyResolutionContext>>,
    callback_active: Cell<bool>,
    work_outstanding: Cell<bool>,
    parent_work_outstanding: Cell<bool>,
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

pub(crate) struct ParseScopeParts<'session> {
    pub entry: RegistryEntry,
    pub options: Arc<CompiledFormatOptions>,
    pub primary_io: &'session dyn AssetIo,
    pub primary_authority: &'session AuthorityDomain,
    pub primary_source: &'session AssetPath,
    pub primary_bytes: &'session [u8],
    pub resolver: ResolverBinding<'session>,
    pub dependency_origin_issuer: &'session DependencyOriginIssuer,
    pub conformance_policy: ConformancePolicy,
    pub loss_policy: LossPolicy,
    pub limits: &'session ResourceLimits,
    pub store_owner: &'session ImportStoreSessionOwner,
    pub host: ImportStoreHost<'session>,
    pub diagnostics: &'session RefCell<DiagnosticSink>,
    pub diagnostic_line_indexes: &'session RefCell<AccountedVec<DiagnosticLineIndex>>,
    pub resource_graph: &'session RefCell<ResourceGraph>,
    pub optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    pub loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    pub source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    pub format_failure_storage: &'session RefCell<Option<ImportStoreReservation>>,
    pub terminal: &'session RefCell<Option<ScopeAbortCause>>,
}

struct ParseCallbackGuard<'scope> {
    active: &'scope Cell<bool>,
}

impl Drop for ParseCallbackGuard<'_> {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

struct PreparedDiagnosticEvent {
    prepared: PreparedDiagnostic,
    payload_reservation: Option<ImportStoreReservation>,
    structural_reservation: Option<ImportStoreReservation>,
}

impl PreparedDiagnosticEvent {
    const fn is_stored(&self) -> bool {
        self.prepared.is_stored()
    }

    const fn stored_bytes(&self) -> u64 {
        self.prepared.stored_bytes()
    }

    const fn drop_reason(&self) -> Option<DiagnosticDropReason> {
        self.prepared.drop_reason()
    }

    fn can_commit_to(&self, diagnostics: &DiagnosticSink) -> bool {
        diagnostics.can_commit(
            &self.prepared,
            &self.payload_reservation,
            &self.structural_reservation,
        )
    }

    fn commit_infallible(self, diagnostics: &mut DiagnosticSink) {
        diagnostics.commit_reserved_infallible(
            self.prepared,
            self.payload_reservation,
            self.structural_reservation,
        );
    }

    fn discard(self, diagnostics: &mut DiagnosticSink) {
        diagnostics.discard_prepared(self.prepared);
        drop(self.payload_reservation);
        drop(self.structural_reservation);
    }
}

struct PreparedLossEvidence {
    evidence: ImportLossEvidence,
    stored_bytes: u64,
}

struct DiagnosticEventMetadata<'event> {
    severity: DiagnosticSeverity,
    code: DiagnosticCode,
    phase: ImportProgressPhase,
    source: &'event AssetPath,
    byte_range: Option<DiagnosticByteRange>,
    line_column: Option<DiagnosticLineColumn>,
    object_path: Option<&'event str>,
}

impl<'session> ParseScope<'session> {
    pub(crate) fn new(parts: ParseScopeParts<'session>) -> Self {
        Self {
            entry: parts.entry,
            options: parts.options,
            primary_io: parts.primary_io,
            primary_authority: parts.primary_authority,
            primary_source: parts.primary_source,
            primary_bytes: parts.primary_bytes,
            resolver: parts.resolver,
            dependency_origin_issuer: parts.dependency_origin_issuer,
            conformance_policy: parts.conformance_policy,
            loss_policy: parts.loss_policy,
            limits: parts.limits,
            store_owner: parts.store_owner,
            host: parts.host,
            diagnostics: parts.diagnostics,
            diagnostic_line_indexes: parts.diagnostic_line_indexes,
            resource_graph: parts.resource_graph,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            source_scope_record: parts.source_scope_record,
            format_failure_storage: parts.format_failure_storage,
            dependency_failure: RefCell::new(None),
            callback_active: Cell::new(false),
            work_outstanding: Cell::new(false),
            parent_work_outstanding: Cell::new(false),
            terminal: parts.terminal,
            not_send_or_sync: PhantomData,
        }
    }

    pub fn source(&self) -> &AssetPath {
        self.primary_source
    }

    pub const fn conformance_policy(&self) -> ConformancePolicy {
        self.conformance_policy
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.loss_policy
    }

    pub fn primary_bytes(&self) -> &[u8] {
        self.primary_bytes
    }

    /// Conforms the source suffix selected by `options` and runs a format-owned grammar action.
    ///
    /// The source capability is the sole byte authority. Recovery diagnostics cannot be rebound to
    /// a separately supplied slice or a different dependency.
    pub fn with_conformed_text_input<'scope, 'source, T>(
        &'scope self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
        action: impl for<'input> FnOnce(
            &'input ConformedTextInput<'scope>,
            &mut TextInputRecoveryTracker<'input, 'session>,
        ) -> ParseResult<T>,
    ) -> ParseResult<TextConformanceResult<T>> {
        crate::text_input::with_conformed_text_input(
            self,
            self.entry.descriptor().text_profile(),
            source,
            options,
            action,
        )
    }

    /// Creates a source-bound cursor that conforms one physical text record at a time.
    pub fn text_input_record_cursor<'scope, 'source>(
        &'scope self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
    ) -> ParseResult<crate::text_input::TextInputRecordCursor<'scope, 'session, 'source>> {
        crate::text_input::TextInputRecordCursor::new(
            self,
            self.entry.descriptor().text_profile(),
            source,
            options,
        )
    }

    pub fn diagnostic_source(&self) -> DiagnosticSource<'_> {
        DiagnosticSource::new(
            self.primary_authority.clone(),
            self.primary_source,
            self.primary_bytes,
        )
    }

    pub fn limits(&self) -> &ResourceLimits {
        self.limits
    }

    /// Submits one complete canonical Scene batch to this session's sole retained-store owner.
    pub fn submit_scene<'input>(
        &self,
        input: SceneAuthoringBatch<'input>,
    ) -> ParseResult<CanonicalSceneDraft> {
        self.preflight_active()?;
        self.store_owner
            .poll_parser_boundary()
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
        self.resource_graph
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .seal(self.host)
            .map_err(|cause| self.abort(cause.into_scope_cause()))?;
        let construction = SceneConstruction::try_new(input, scene_validation_limits(self.limits))
            .map_err(|_| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "canonical-scene-request-rejected",
                })
            })?;
        self.store_owner
            .submit_scene(construction)
            .map_err(|cause| {
                let code = match cause {
                    crate::store::ImportStoreActivationError::StoreUnavailable(_) => {
                        "canonical-scene-store-unavailable"
                    }
                    crate::store::ImportStoreActivationError::Control(cause) => {
                        return self.abort(ScopeAbortCause::Control(cause));
                    }
                    crate::store::ImportStoreActivationError::AuthoringRejected => {
                        "canonical-scene-authoring-rejected"
                    }
                    crate::store::ImportStoreActivationError::MaterializationFailed => {
                        "canonical-scene-materialization-failed"
                    }
                    crate::store::ImportStoreActivationError::Containment(_)
                    | crate::store::ImportStoreActivationError::Preparation(_) => {
                        "canonical-scene-activation-contained"
                    }
                };
                self.abort(ScopeAbortCause::DescriptorContract { code })
            })
    }

    /// Starts one incremental source-selection and closure evidence record.
    pub fn source_scope_builder(&self) -> ParseResult<SourceScopeRecordBuilder<'_, 'session>> {
        self.preflight_active()?;
        Ok(SourceScopeRecordBuilder::new(self))
    }

    /// Creates vector storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_vec<T>(
        &self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredVec<'_, T> {
        MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter: self.storage_meter(),
            class,
            resource,
        }
    }

    /// Creates string storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_string(
        &self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredString<'_> {
        MeteredString {
            value: String::new(),
            capacity: None,
            meter: self.storage_meter(),
            class,
            resource,
        }
    }

    /// Creates ordered-map storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_ordered_map<K, V>(
        &self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredOrderedMap<'_, K, V> {
        MeteredOrderedMap {
            value: OrderedMap::new(),
            capacity: None,
            meter: self.storage_meter(),
            class,
            resource,
        }
    }

    /// Reserves an opaque backend allowance that cannot be transferred into a Scene value.
    pub fn metered_opaque_allowance(
        &self,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<MeteredOpaqueAllowance<'_>> {
        self.preflight_active()?;
        let reservation = if bytes == 0 {
            None
        } else {
            Some(
                self.reserve_storage_cause(class, bytes)
                    .map_err(|cause| self.abort(cause))?,
            )
        };
        Ok(MeteredOpaqueAllowance {
            reservation,
            scope: PhantomData,
        })
    }

    /// Creates a decoded text owner with separate logical-length and allocator-slack accounting.
    pub fn metered_decoded_text(
        &self,
        decoded_len: usize,
        resource: &'static str,
    ) -> ParseResult<MeteredDecodedText<'_>> {
        self.preflight_active()?;
        let plan = CapacityPlan::with_element_size(0, 0, decoded_len, 1)
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        let envelope = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        let decoded_bytes =
            u64::try_from(decoded_len).map_err(|_| self.allocation_failure(resource))?;
        let slack_bytes = envelope.checked_sub(decoded_bytes).ok_or_else(|| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-envelope-below-length",
            })
        })?;
        let decoded = if decoded_bytes == 0 {
            None
        } else {
            Some(
                self.reserve_storage_cause(ImportStorageClass::DecodedText, decoded_bytes)
                    .map_err(|cause| self.abort(cause))?,
            )
        };
        let mut allocator_slack = if slack_bytes == 0 {
            None
        } else {
            Some(
                self.reserve_storage_cause(ImportStorageClass::BackendDecode, slack_bytes)
                    .map_err(|cause| self.abort(cause))?,
            )
        };

        let mut value = String::new();
        plan.verify_preflight(value.len(), value.capacity())
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        value
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| self.allocation_failure(resource))?;
        let actual = plan
            .finish(value.len(), value.capacity())
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        let actual_slack = actual.checked_sub(decoded_bytes).ok_or_else(|| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-capacity-below-length",
            })
        })?;
        if actual_slack == 0 {
            drop(allocator_slack.take());
        } else if let Some(reservation) = allocator_slack.as_mut() {
            self.resize_storage_reservation(reservation, actual_slack)
                .map_err(|cause| self.abort(cause))?;
        } else {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-slack-unreserved",
            }));
        }

        Ok(MeteredDecodedText {
            value,
            decoded_len,
            decoded,
            allocator_slack,
            meter: self.storage_meter(),
            resource,
        })
    }

    fn storage_meter(&self) -> StorageMeter<'_> {
        StorageMeter {
            host: self.host,
            callback_active: &self.callback_active,
            terminal: self.terminal,
        }
    }

    pub fn options<T: std::any::Any + Send + Sync + std::panic::RefUnwindSafe>(
        &self,
        key: &FormatOptionKey<T>,
    ) -> ParseResult<&T> {
        self.preflight_active()?;
        self.options
            .downcast_ref(key)
            .ok_or_else(|| self.abort(ScopeAbortCause::OptionTypeMismatch))
    }

    pub fn poll(&self) -> ParseResult<()> {
        self.poll_active()
    }

    fn poll_active(&self) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .poll()
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    pub fn charge_work(&self, amount: u64) -> ParseResult<()> {
        self.with_work(ImportWorkClass::WorkUnits, amount, |work| {
            work.complete_all()
        })
    }

    /// Runs one callback-scoped parser work action.
    pub fn with_work<T, E>(
        &self,
        class: ImportWorkClass,
        amount: u64,
        action: impl FnOnce(&mut ImportWorkScope<'_, 'session, 1>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ParseFailure>,
    {
        self.with_work_group([(class, amount)], action)
    }

    /// Runs a semantic parent action whose callback may admit sequential child work.
    ///
    /// The parent amount is attempted before the callback starts and remains incomplete when any
    /// child decode, token, allocation, cancellation, or format action fails. Ordinary work scopes
    /// remain non-nestable. Parent completion is sealed inside this method so callers cannot publish
    /// it before every child action succeeds.
    pub fn with_parent_work<T, E>(
        &self,
        class: ImportWorkClass,
        amount: u64,
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ParseFailure>,
    {
        let mut work = self
            .begin_work_group([(class, amount)], true)
            .map_err(E::from)?;
        match action() {
            Ok(value) => {
                if let Err(failure) = self.poll_active() {
                    work.finish_partial();
                    return Err(E::from(failure));
                }
                if let Err(failure) = work.complete_all() {
                    work.finish_partial();
                    return Err(E::from(failure));
                }
                work.finish_exact().map_err(E::from)?;
                Ok(value)
            }
            Err(cause) => {
                work.finish_partial();
                let _ = self.poll_active();
                Err(cause)
            }
        }
    }

    /// Runs one callback-scoped parser action across an exact fixed work group.
    pub fn with_work_group<const N: usize, T, E>(
        &self,
        charges: [(ImportWorkClass, u64); N],
        action: impl FnOnce(&mut ImportWorkScope<'_, 'session, N>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ParseFailure>,
    {
        let mut work = self.begin_work_group(charges, false).map_err(E::from)?;
        match action(&mut work) {
            Ok(value) => {
                if work.remaining().into_iter().any(|amount| amount != 0) {
                    work.finish_partial();
                    self.poll_active().map_err(E::from)?;
                    return Err(E::from(self.abort(ScopeAbortCause::DescriptorContract {
                        code: "parse-work-callback-incomplete",
                    })));
                }
                work.finish_exact().map_err(E::from)?;
                Ok(value)
            }
            Err(cause) => {
                work.finish_partial();
                let _ = self.poll_active();
                Err(cause)
            }
        }
    }

    fn begin_work_group<const N: usize>(
        &self,
        charges: [(ImportWorkClass, u64); N],
        parent: bool,
    ) -> ParseResult<ImportWorkScope<'_, 'session, N>> {
        self.preflight_active()?;
        if self.work_outstanding.get() || (parent && self.parent_work_outstanding.get()) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-work-already-in-progress",
            }));
        }
        self.poll_active()?;

        let accounted = charges.map(|(class, amount)| {
            let (dimension, item_limit) = class.accounting(self.limits);
            (dimension, item_limit, amount)
        });
        let mut item_limit_failure = None;
        for (dimension, item_limit, amount) in accounted {
            self.host
                .charge_attempted(dimension, amount)
                .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
            if let Some((resource, limit)) = item_limit
                && amount > limit
            {
                item_limit_failure = Some((resource, limit));
                break;
            }
        }
        if let Some((resource, limit)) = item_limit_failure {
            return Err(self.abort(ScopeAbortCause::ResourceLimit { resource, limit }));
        }

        let admitted = accounted.map(|(_, _, amount)| amount);
        if parent {
            self.parent_work_outstanding.set(true);
        } else {
            self.work_outstanding.set(true);
        }
        Ok(ImportWorkScope {
            scope: self,
            dimensions: accounted.map(|(dimension, _, _)| dimension),
            item_limits: accounted.map(|(_, item_limit, _)| item_limit),
            admitted,
            completed: [0; N],
            parent,
            active: true,
            finished: false,
        })
    }

    /// Converts a fallible allocation failure into the scope's first terminal import cause.
    pub fn allocation_failure(&self, resource: &'static str) -> ParseFailure {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        self.abort(ScopeAbortCause::Allocation { resource })
    }

    /// Enforces one format-local bound that has no aggregate ledger dimension.
    pub fn enforce_resource_limit(
        &self,
        resource: &'static str,
        attempted: u64,
        limit: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        if attempted <= limit {
            return Ok(());
        }
        Err(self.abort(ScopeAbortCause::ResourceLimit { resource, limit }))
    }

    /// Reports a checked-arithmetic overflow as an already-proven resource-limit failure.
    pub fn resource_limit_exceeded(&self, resource: &'static str, limit: u64) -> ParseFailure {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        self.abort(ScopeAbortCause::ResourceLimit { resource, limit })
    }

    pub(crate) fn descriptor_contract_failure(&self, code: &'static str) -> ParseFailure {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        self.abort(ScopeAbortCause::DescriptorContract { code })
    }

    pub fn report_progress(
        &self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn reserve_live(
        &self,
        storage: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<ImportLiveReservation<'_>> {
        self.preflight_active()?;
        let reservation = self
            .reserve_storage_cause(storage, bytes)
            .map_err(|cause| self.abort(cause))?;
        Ok(ImportLiveReservation {
            reservation,
            scope: PhantomData,
        })
    }

    pub(crate) fn reserve_storage_cause(
        &self,
        storage: ImportStorageClass,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        let (phase, dimension) = storage.accounting();
        self.host
            .reserve(phase, dimension, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resize_storage_reservation(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ScopeAbortCause> {
        self.host
            .resize(reservation, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resize_live(
        &self,
        reservation: &mut ImportLiveReservation<'_>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .resize(&mut reservation.reservation, bytes)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn capacity_plan_error(
        &self,
        resource: &'static str,
        cause: CapacityPlanError,
    ) -> ParseFailure {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        match cause {
            CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
                self.resource_limit_exceeded(resource, u64::MAX)
            }
            CapacityPlanError::InvalidState { .. } | CapacityPlanError::StalePlan { .. } => self
                .abort(ScopeAbortCause::DescriptorContract {
                    code: "capacity-plan-contract-failed",
                }),
            cause @ CapacityPlanError::CapacityEnvelopeViolated { .. } => {
                self.abort(ScopeAbortCause::CapacityEnvelope { resource, cause })
            }
            _ => self.abort(ScopeAbortCause::DescriptorContract {
                code: "capacity-plan-error-unknown",
            }),
        }
    }

    /// Records and authorizes one named format recovery under frozen common policy.
    ///
    /// The loss kind, disposition, diagnostic severity, and retained source identity are
    /// session-owned so a format extension cannot forge arbitrary loss evidence.
    pub fn record_format_recovery(
        &self,
        code: DiagnosticCode,
        source: DiagnosticSource<'_>,
        raw_range: DiagnosticByteRange,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let source_len = u64::try_from(source.bytes().len())
            .map_err(|_| self.resource_limit_exceeded("format-recovery-source", u64::MAX))?;
        if raw_range.end() > source_len {
            return Err(self.abort(ScopeAbortCause::Diagnostic(
                DiagnosticCause::ByteRangeOutOfBounds {
                    end: raw_range.end(),
                    source_len,
                },
            )));
        }
        let raw_bytes = raw_range.end().saturating_sub(raw_range.start());
        let charge = self
            .host
            .charge_attempted(ImportDimension::RecoveryCount, 1)
            .and_then(|()| self.host.charge_consumed(ImportDimension::RecoveryCount, 1))
            .and_then(|()| {
                self.host
                    .charge_attempted(ImportDimension::RecoveryRawBytes, raw_bytes)
            })
            .and_then(|()| {
                self.host
                    .charge_consumed(ImportDimension::RecoveryRawBytes, raw_bytes)
            });
        if let Err(cause) = charge {
            return Err(self.abort(cause.into_scope_abort_cause()));
        }

        let descriptor_allows_recovery =
            self.entry
                .descriptor()
                .text_profile()
                .is_some_and(|profile| {
                    profile.allows_decoding_mode(TextDecodingMode::ReplaceInvalidUtf8)
                });
        let disposition = if !descriptor_allows_recovery
            || self.conformance_policy != ConformancePolicy::AllowDeclaredRecovery
        {
            ImportLossDisposition::RejectedByConformance
        } else if !self.loss_policy.allows(ImportLossKind::FormatRecovery) {
            ImportLossDisposition::RejectedByLossPolicy
        } else {
            ImportLossDisposition::Applied
        };
        self.retain_loss_evidence(
            ImportLossKind::FormatRecovery,
            ImportLossSubject::FormatRecovery,
            code,
            &source,
            raw_range,
            disposition,
        )?;
        self.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Normalize,
            source,
            Some(raw_range),
            None,
            |_| {},
            message,
        )?;

        match disposition {
            ImportLossDisposition::Applied => Ok(()),
            ImportLossDisposition::RejectedByConformance => {
                Err(self.abort(ScopeAbortCause::FormatRecoveryRejectedByConformance { code }))
            }
            ImportLossDisposition::RejectedByLossPolicy => {
                Err(self.abort(ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { code }))
            }
        }
    }

    /// Submits one finalized source-scope record to this parse session's sealed slot.
    pub fn record_source_scope(
        &self,
        finalized: FinalizedSourceScopeRecord<'_, 'session>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        if !finalized.belongs_to(self) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-record-owner-mismatch",
            }));
        }
        let mut source_scope_slot = self
            .source_scope_record
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        if source_scope_slot.is_some() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-record-already-submitted",
            }));
        }

        self.source_scope_charge_attempted(ImportDimension::SourceScopeRecordCount, 1)?;
        let record = finalized.into_record();
        if !record.is_collapsed() {
            self.source_scope_charge_consumed(ImportDimension::SourceScopeRecordCount, 1)?;
            *source_scope_slot = Some(record);
            return Ok(());
        }

        let code = DiagnosticCode::new("baozi.source_scope_details_collapsed").map_err(|_| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-collapse-code-invalid",
            })
        })?;
        let diagnostic_source = self.diagnostic_source();
        let loss = self.prepare_loss_evidence(
            ImportLossKind::ProvenanceEvidenceLoss,
            ImportLossSubject::SourceScopeDetails,
            code,
            &diagnostic_source,
            None,
            ImportLossDisposition::Applied,
        )?;
        let diagnostic = self.prepare_diagnostic_event(
            DiagnosticEventMetadata {
                severity: DiagnosticSeverity::Warning,
                code,
                phase: ImportProgressPhase::Normalize,
                source: self.primary_source,
                byte_range: None,
                line_column: None,
                object_path: None,
            },
            |details| {
                details.unsigned("detail_count", record.collapsed_detail_count());
                details.unsigned("detail_bytes", record.collapsed_detail_bytes());
            },
            format_args!("source-scope details were collapsed after explicit caller authority"),
        )?;
        if let Some(reason) = diagnostic.drop_reason() {
            self.discard_diagnostic_event(diagnostic)?;
            drop(loss);
            let (resource, limit) = match reason {
                DiagnosticDropReason::CountLimit => (
                    "source-scope-collapse-diagnostic-count",
                    self.limits.max_diagnostics,
                ),
                DiagnosticDropReason::StoredByteLimit => (
                    "source-scope-collapse-diagnostic-bytes",
                    self.limits.max_diagnostic_bytes,
                ),
            };
            return Err(self.abort(ScopeAbortCause::ResourceLimit { resource, limit }));
        }

        let mut loss_evidence = match self.loss_evidence.try_borrow_mut() {
            Ok(evidence) => evidence,
            Err(_) => {
                self.discard_diagnostic_event(diagnostic)?;
                drop(loss);
                return Err(self.reentrant_capability_failure());
            }
        };
        let mut diagnostics = match self.diagnostics.try_borrow_mut() {
            Ok(diagnostics) => diagnostics,
            Err(_) => {
                drop(loss_evidence);
                self.discard_diagnostic_event(diagnostic)?;
                drop(loss);
                return Err(self.reentrant_capability_failure());
            }
        };
        if !loss_evidence.has_slots(1) || !diagnostic.can_commit_to(&diagnostics) {
            diagnostic.discard(&mut diagnostics);
            drop(loss);
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-publication-capacity-not-admitted",
            }));
        }
        if let Err(failure) =
            self.charge_source_scope_bundle_consumed(loss.stored_bytes, diagnostic.stored_bytes())
        {
            diagnostic.discard(&mut diagnostics);
            drop(loss);
            return Err(failure);
        }
        loss_evidence.push_reserved_infallible(loss.evidence);
        diagnostic.commit_infallible(&mut diagnostics);
        *source_scope_slot = Some(record);
        Ok(())
    }

    fn retain_loss_evidence(
        &self,
        kind: ImportLossKind,
        subject: ImportLossSubject,
        code: DiagnosticCode,
        source: &DiagnosticSource<'_>,
        raw_range: DiagnosticByteRange,
        disposition: ImportLossDisposition,
    ) -> ParseResult<()> {
        let evidence =
            self.prepare_loss_evidence(kind, subject, code, source, Some(raw_range), disposition)?;
        self.commit_loss_evidence(evidence)
    }

    fn prepare_loss_evidence(
        &self,
        kind: ImportLossKind,
        subject: ImportLossSubject,
        code: DiagnosticCode,
        source: &DiagnosticSource<'_>,
        raw_range: Option<DiagnosticByteRange>,
        disposition: ImportLossDisposition,
    ) -> ParseResult<PreparedLossEvidence> {
        self.host
            .charge_attempted(ImportDimension::LossEvidenceCount, 1)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
        let source_envelope = u64::try_from(source.path().as_str().len())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        let mut reservation = self
            .host
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::LossEvidenceBytes,
                source_envelope,
            )
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;

        let retained_source = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("loss-evidence-source"))?;
        let source_bytes = u64::try_from(retained_source.capacity_bytes())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        if source_bytes != source_envelope {
            self.host
                .resize(&mut reservation, source_bytes)
                .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
        }

        let (prior_capacity_bytes, capacity_result) = {
            let mut evidence = self
                .loss_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let prior_capacity_bytes = evidence.capacity_bytes();
            let result = evidence.try_reserve(1, "loss-evidence-capacity", |capacity_bytes| {
                self.host
                    .reserve(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::LossEvidenceBytes,
                        capacity_bytes,
                    )
                    .map_err(|cause| cause.into_scope_abort_cause())
            });
            (prior_capacity_bytes, result)
        };
        capacity_result.map_err(|cause| self.abort(cause))?;
        let capacity_bytes = self
            .loss_evidence
            .try_borrow()
            .map_err(|_| self.reentrant_capability_failure())?
            .capacity_bytes()
            .saturating_sub(prior_capacity_bytes);
        let actual_bytes = source_bytes
            .checked_add(capacity_bytes)
            .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-bytes", u64::MAX))?;
        let charge = self
            .host
            .charge_attempted(ImportDimension::LossEvidenceBytes, actual_bytes);
        if let Err(cause) = charge {
            return Err(self.abort(cause.into_scope_abort_cause()));
        }

        let evidence = ImportLossEvidence::new(
            kind,
            subject,
            code,
            retained_source,
            raw_range,
            disposition,
            reservation,
        );
        Ok(PreparedLossEvidence {
            evidence,
            stored_bytes: actual_bytes,
        })
    }

    fn commit_loss_evidence(&self, prepared: PreparedLossEvidence) -> ParseResult<()> {
        let mut loss_evidence = self
            .loss_evidence
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        if !loss_evidence.has_slots(1) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "loss-evidence-slots-not-admitted",
            }));
        }
        self.charge_loss_evidence_consumed(prepared.stored_bytes)?;
        loss_evidence.push_reserved_infallible(prepared.evidence);
        Ok(())
    }

    fn charge_loss_evidence_consumed(&self, stored_bytes: u64) -> ParseResult<()> {
        self.host
            .charge_consumed_batch(&[
                (ImportDimension::LossEvidenceCount, 1),
                (ImportDimension::LossEvidenceBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    pub fn emit_diagnostic(
        &self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.emit_diagnostic_event(
            DiagnosticEventMetadata {
                severity,
                code,
                phase: ImportProgressPhase::Decode,
                source: self.primary_source,
                byte_range: None,
                line_column: None,
                object_path: None,
            },
            |_| {},
            message,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_diagnostic_with(
        &self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        source: DiagnosticSource<'_>,
        byte_range: Option<DiagnosticByteRange>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let line_column = self.derive_line_column(&source, byte_range)?;
        self.emit_diagnostic_event(
            DiagnosticEventMetadata {
                severity,
                code,
                phase,
                source: source.path(),
                byte_range,
                line_column,
                object_path,
            },
            details,
            message,
        )
    }

    fn emit_diagnostic_event(
        &self,
        metadata: DiagnosticEventMetadata<'_>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        let prepared = self.prepare_diagnostic_event(metadata, details, message)?;
        self.commit_diagnostic_event(prepared)
    }

    fn prepare_diagnostic_event(
        &self,
        metadata: DiagnosticEventMetadata<'_>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<PreparedDiagnosticEvent> {
        let DiagnosticEventMetadata {
            severity,
            code,
            phase,
            source,
            byte_range,
            line_column,
            object_path,
        } = metadata;
        self.preflight_active()?;
        let admission = self
            .diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .begin(code, source, byte_range, line_column, object_path);
        let initial_failure = admission.failure().cloned();
        let initial_charge = self
            .host
            .charge_attempted(ImportDimension::DiagnosticCount, 1)
            .and_then(|()| {
                self.host.charge_attempted(
                    ImportDimension::DiagnosticBytes,
                    admission.fixed_attempted_bytes(),
                )
            });
        if let Err(cause) = initial_charge {
            self.diagnostics.borrow_mut().discard_admission();
            return Err(self.abort(cause.into_scope_abort_cause()));
        }
        if let Some(cause) = initial_failure {
            self.diagnostics.borrow_mut().discard_admission();
            return Err(self.abort(ScopeAbortCause::Diagnostic(cause)));
        }

        let stores_diagnostic = admission.storage_reservation_bytes() != 0;
        let mut reservation = if !stores_diagnostic {
            None
        } else {
            Some(
                self.host
                    .reserve(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::DiagnosticBytes,
                        admission.payload_capacity_envelope(),
                    )
                    .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?,
            )
        };
        let mut structural_reservation = if stores_diagnostic {
            Some(
                self.reserve_live(
                    ImportStorageClass::FormatNormalization,
                    admission.structural_capacity_envelope(),
                )?
                .into_reservation(),
            )
        } else {
            None
        };
        if reservation.is_some() {
            let mut diagnostics = self
                .diagnostics
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            diagnostics
                .try_reserve_storage_slot(|capacity_bytes| {
                    self.reserve_diagnostic_scratch_cause(capacity_bytes)
                })
                .map_err(|cause| self.abort(cause))?;
            diagnostics
                .try_reserve_reservation_slots(2, |capacity_bytes| {
                    self.reserve_diagnostic_scratch_cause(capacity_bytes)
                })
                .map_err(|cause| self.abort(cause))?;
        }

        let prepared = self
            .diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .prepare(
                admission,
                severity,
                code,
                phase,
                source,
                byte_range,
                line_column,
                object_path,
                details,
                message,
            );
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(failure) => {
                if let Err(cause) = self.host.charge_attempted(
                    ImportDimension::DiagnosticBytes,
                    failure.additional_attempted_bytes,
                ) {
                    return Err(self.abort(cause.into_scope_abort_cause()));
                }
                return Err(match failure.cause {
                    DiagnosticPrepareCause::Diagnostic(cause) => {
                        self.abort(ScopeAbortCause::Diagnostic(cause))
                    }
                    DiagnosticPrepareCause::Panicked => {
                        self.abort(ScopeAbortCause::TrustedExtension {
                            boundary: TrustedExtensionBoundary::DiagnosticCallback,
                            code: "diagnostic-callback-panicked",
                        })
                    }
                });
            }
        };

        if let Err(cause) = self.host.charge_attempted(
            ImportDimension::DiagnosticBytes,
            prepared.additional_attempted_bytes(),
        ) {
            self.diagnostics.borrow_mut().discard_prepared(prepared);
            return Err(self.abort(cause.into_scope_abort_cause()));
        }

        let (payload_reservation, structural_reservation) = if prepared.is_stored() {
            let Some(upper_bound) = reservation.take() else {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-reservation",
                }));
            };
            let payload_capacity_bytes = prepared.payload_capacity_bytes();
            let structural_bytes = prepared.structural_bytes();
            let mut exact = upper_bound;
            let payload_resize = self
                .host
                .resize(&mut exact, payload_capacity_bytes)
                .map_err(|cause| cause.into_scope_abort_cause());
            if let Err(cause) = payload_resize {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                drop(exact);
                return Err(self.abort(cause));
            }
            let Some(structural_upper_bound) = structural_reservation.take() else {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                drop(exact);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-structural-reservation",
                }));
            };
            let mut structural_exact = structural_upper_bound;
            let structural_resize = self
                .host
                .resize(&mut structural_exact, structural_bytes)
                .map_err(|cause| cause.into_scope_abort_cause());
            if let Err(cause) = structural_resize {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                drop(exact);
                drop(structural_exact);
                return Err(self.abort(cause));
            }
            (Some(exact), Some(structural_exact))
        } else {
            (None, None)
        };
        Ok(PreparedDiagnosticEvent {
            prepared,
            payload_reservation,
            structural_reservation,
        })
    }

    fn commit_diagnostic_event(&self, event: PreparedDiagnosticEvent) -> ParseResult<()> {
        if !event.is_stored() {
            return Ok(());
        }
        let mut diagnostics = match self.diagnostics.try_borrow_mut() {
            Ok(diagnostics) => diagnostics,
            Err(_) => {
                drop(event);
                return Err(self.reentrant_capability_failure());
            }
        };
        if !event.can_commit_to(&diagnostics) {
            event.discard(&mut diagnostics);
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "diagnostic-storage-slots-not-admitted",
            }));
        }
        if let Err(failure) = self.charge_diagnostic_consumed(event.stored_bytes()) {
            event.discard(&mut diagnostics);
            return Err(failure);
        }
        event.commit_infallible(&mut diagnostics);
        Ok(())
    }

    fn charge_diagnostic_consumed(&self, stored_bytes: u64) -> ParseResult<()> {
        self.host
            .charge_consumed_batch(&[
                (ImportDimension::DiagnosticCount, 1),
                (ImportDimension::DiagnosticBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn discard_diagnostic_event(&self, event: PreparedDiagnosticEvent) -> ParseResult<()> {
        let mut diagnostics = self
            .diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        event.discard(&mut diagnostics);
        Ok(())
    }

    fn reserve_diagnostic_scratch_cause(
        &self,
        capacity_bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        self.host
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::ScratchBytes,
                capacity_bytes,
            )
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn derive_line_column(
        &self,
        source: &DiagnosticSource<'_>,
        byte_range: Option<DiagnosticByteRange>,
    ) -> ParseResult<Option<DiagnosticLineColumn>> {
        let Some(byte_range) = byte_range else {
            return Ok(None);
        };
        let bytes = source.bytes();
        let source_len = u64::try_from(bytes.len())
            .map_err(|_| self.resource_limit_exceeded("diagnostic-source-bytes", u64::MAX))?;
        if byte_range.end() > source_len {
            return Err(self.abort(ScopeAbortCause::Diagnostic(
                DiagnosticCause::ByteRangeOutOfBounds {
                    end: byte_range.end(),
                    source_len,
                },
            )));
        }
        if let Some(location) = self
            .diagnostic_line_indexes
            .try_borrow()
            .map_err(|_| self.reentrant_capability_failure())?
            .iter()
            .find(|index| index.matches(source, source_len))
            .map(|index| index.derive(byte_range))
        {
            return location
                .map(Some)
                .map_err(|cause| self.abort(ScopeAbortCause::Diagnostic(cause)));
        }

        let mut line_count = 1_usize;
        for chunk in bytes.chunks(4096) {
            line_count = self.with_work(
                ImportWorkClass::DecodeWork,
                u64::try_from(chunk.len()).map_err(|_| {
                    self.resource_limit_exceeded("diagnostic-line-index-work", u64::MAX)
                })?,
                |work| {
                    let next = chunk
                        .iter()
                        .try_fold(line_count, |count, byte| {
                            if *byte == b'\n' {
                                count.checked_add(1)
                            } else {
                                Some(count)
                            }
                        })
                        .ok_or_else(|| {
                            self.resource_limit_exceeded("diagnostic-line-index-entries", u64::MAX)
                        })?;
                    work.complete_all()?;
                    Ok(next)
                },
            )?;
        }
        let source_plan = CapacityPlan::for_type::<u8>(0, 0, source.path().as_str().len())
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-source", cause))?;
        let source_admitted_bytes = source_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-source", cause))?;
        let mut source_reservation = if source_admitted_bytes == 0 {
            None
        } else {
            Some(self.reserve_live(ImportStorageClass::LineIndex, source_admitted_bytes)?)
        };
        let mut line_starts = AccountedVec::new();
        line_starts
            .try_reserve_capacity(line_count, "diagnostic-line-index", |capacity_bytes| {
                self.reserve_storage_cause(ImportStorageClass::LineIndex, capacity_bytes)
            })
            .map_err(|cause| self.abort(cause))?;
        line_starts.try_push_reserved(0).map_err(|_| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "diagnostic-line-index-slots-not-admitted",
            })
        })?;
        let mut offset = 0_usize;
        for chunk in bytes.chunks(4096) {
            offset = self.with_work(
                ImportWorkClass::DecodeWork,
                u64::try_from(chunk.len()).map_err(|_| {
                    self.resource_limit_exceeded("diagnostic-line-index-work", u64::MAX)
                })?,
                |work| {
                    for (chunk_offset, byte) in chunk.iter().enumerate() {
                        if *byte == b'\n' {
                            let start = offset
                                .checked_add(chunk_offset)
                                .and_then(|value| value.checked_add(1))
                                .and_then(|value| u64::try_from(value).ok())
                                .ok_or_else(|| {
                                    self.resource_limit_exceeded(
                                        "diagnostic-line-index-offset",
                                        u64::MAX,
                                    )
                                })?;
                            line_starts.try_push_reserved(start).map_err(|_| {
                                self.abort(ScopeAbortCause::DescriptorContract {
                                    code: "diagnostic-line-index-slots-not-admitted",
                                })
                            })?;
                        }
                    }
                    let next = offset.checked_add(chunk.len()).ok_or_else(|| {
                        self.resource_limit_exceeded("diagnostic-line-index-offset", u64::MAX)
                    })?;
                    work.complete_all()?;
                    Ok(next)
                },
            )?;
        }

        let source_path = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("diagnostic-line-index-source"))?;
        let source_capacity_bytes = u64::try_from(source_path.capacity_bytes())
            .map_err(|_| self.resource_limit_exceeded("diagnostic-line-index-source", u64::MAX))?;
        if source_capacity_bytes > source_admitted_bytes {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "diagnostic-line-index-reservation-underestimated",
            }));
        }
        let source_reservation = match source_reservation.as_mut() {
            Some(reservation) if source_capacity_bytes != 0 => {
                self.resize_live(reservation, source_capacity_bytes)?;
                source_reservation.map(ImportLiveReservation::into_reservation)
            }
            Some(_) => {
                drop(source_reservation.take());
                None
            }
            None if source_capacity_bytes == 0 => None,
            None => {
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "diagnostic-line-index-source-without-reservation",
                }));
            }
        };
        let index = DiagnosticLineIndex {
            authority: source.authority().clone(),
            source: source_path,
            _source_reservation: source_reservation,
            source_len,
            line_starts,
        };
        let mut indexes = self
            .diagnostic_line_indexes
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        indexes
            .try_reserve(1, "diagnostic-line-index-cache", |capacity_bytes| {
                self.reserve_storage_cause(ImportStorageClass::LineIndex, capacity_bytes)
            })
            .map_err(|cause| self.abort(cause))?;
        if let Err(index) = indexes.try_push_reserved(index) {
            drop(index);
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "diagnostic-line-index-cache-slots-not-admitted",
            }));
        }
        indexes
            .last()
            .ok_or_else(|| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "diagnostic-line-index-missing-after-insert",
                })
            })?
            .derive(byte_range)
            .map(Some)
            .map_err(|cause| self.abort(ScopeAbortCause::Diagnostic(cause)))
    }

    /// Returns the opaque origin capability for references authored by the primary resource.
    pub fn root_origin(&self) -> DependencyOrigin<'_> {
        DependencyOrigin::primary(self.dependency_origin_issuer, self.primary_io)
    }

    /// Acquires one dependency relative to the resource that authored its hostile reference.
    ///
    /// Requiredness is derived exclusively from the selected format's frozen role declaration.
    pub fn dependency<'scope>(
        &'scope self,
        origin: &DependencyOrigin<'scope>,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> ParseResult<DependencyOutcome<'scope>> {
        self.poll_active()?;
        if !origin.is_issued_by(self.dependency_origin_issuer) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-origin-foreign-session",
            }));
        }
        let requirement = self.frozen_dependency_requirement(role)?;

        let source = match origin.node() {
            DependencyOriginNode::Primary => {
                if !std::ptr::eq(origin.io(), self.primary_io) {
                    return Err(self.abort(ScopeAbortCause::DescriptorContract {
                        code: "dependency-origin-primary-backend-mismatch",
                    }));
                }
                self.resource_graph
                    .try_borrow()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .source_for_primary()
                    .map_err(|cause| self.abort(cause.into_scope_cause()))?
            }
            DependencyOriginNode::Dependency(index) => self
                .resource_graph
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?
                .source_for_cache(index)
                .map_err(|_| {
                    self.abort(ScopeAbortCause::DescriptorContract {
                        code: "dependency-origin-stale",
                    })
                })?,
        };
        self.charge_logical_reference()
            .map_err(|failure| self.abort_dependency_failure(failure))?;
        let pending = self
            .resource_graph
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .begin_relation(self.host, source, role, requirement, raw_reference)
            .map_err(|cause| self.abort(cause.into_scope_cause()))?;

        let authorized = match origin.node() {
            DependencyOriginNode::Primary => self.authorize_dependency(
                pending,
                origin.io(),
                self.primary_authority,
                self.primary_source,
                role,
                raw_reference,
            ),
            DependencyOriginNode::Dependency(index) => {
                let cache = self
                    .resource_graph
                    .try_borrow()
                    .map_err(|_| self.reentrant_capability_failure())?;
                let Some(identity) = cache.origin_identity(index) else {
                    drop(cache);
                    return Err(self.abort(ScopeAbortCause::DescriptorContract {
                        code: "dependency-origin-stale",
                    }));
                };
                self.authorize_dependency(
                    pending,
                    origin.io(),
                    identity.authority(),
                    identity.path(),
                    role,
                    raw_reference,
                )
            }
        };
        let acquired = authorized
            .and_then(|(pending, normalized, authorized)| {
                self.prepare_authorized_target(pending, normalized, authorized)
            })
            .and_then(|(pending, normalized, authorized)| {
                self.acquire_authorized(pending, normalized, authorized)
            });
        match (requirement, acquired) {
            (_, Ok((snapshot, pending, normalized, cache_index))) => {
                self.resource_graph
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .complete_available(
                        self.host,
                        pending,
                        cache_index,
                        snapshot.bytes(),
                        AvailableResourceIdentity::new(
                            snapshot.identity(),
                            normalized.path(),
                            snapshot.stable_identity_schema(),
                        ),
                    )
                    .map_err(|cause| self.abort(cause.into_scope_cause()))?;
                Ok(DependencyOutcome::available(snapshot))
            }
            (DependencyRequirement::Required, Err(failure)) => {
                Err(self.abort_dependency_failure(failure))
            }
            (DependencyRequirement::Optional, Err(failure)) => {
                self.resolve_optional_failure(role, failure)
            }
        }
    }

    fn frozen_dependency_requirement(
        &self,
        role: &DependencyRoleId,
    ) -> ParseResult<DependencyRequirement> {
        self.entry
            .descriptor()
            .dependency_requirement(role)
            .ok_or_else(|| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "dependency-role-not-declared",
                })
            })
    }

    /// Extracts an admitted dependency, failing closed when the caller expected availability but
    /// the frozen descriptor allowed an unavailable outcome.
    pub fn into_available_dependency<'scope>(
        &'scope self,
        outcome: DependencyOutcome<'scope>,
    ) -> ParseResult<DependencySnapshot<'scope>> {
        match outcome.into_result() {
            Ok(snapshot) => Ok(snapshot),
            Err(evidence) => {
                drop(evidence);
                Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "required-dependency-became-unavailable",
                }))
            }
        }
    }

    pub fn format_failure<P>(&self, producer: P) -> ParseFailure
    where
        P: FormatFailureEvidenceProducer,
    {
        let mut producer = Some(producer);
        let captured = (|| -> ParseResult<FormatFailureEvidence> {
            self.preflight_active()?;
            let failure_storage_is_active = match self.format_failure_storage.try_borrow() {
                Ok(storage) => storage.is_some(),
                Err(_) => return Err(self.reentrant_capability_failure()),
            };
            if failure_storage_is_active {
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "format-failure-storage-already-active",
                }));
            }
            let capacity_plan =
                BoundedFormatFailureDetailSink::capacity_plan().map_err(|cause| {
                    self.capacity_plan_error("format-failure-evidence-storage", cause)
                })?;
            let envelope_bytes = capacity_plan.envelope_bytes().map_err(|cause| {
                self.capacity_plan_error("format-failure-evidence-storage", cause)
            })?;
            let mut storage =
                self.reserve_live(ImportStorageClass::FormatNormalization, envelope_bytes)?;
            let (mut evidence, actual_storage_bytes) =
                BoundedFormatFailureDetailSink::try_allocate(capacity_plan).map_err(|cause| {
                    match cause {
                        FormatFailureDetailCause::CapacityPlan(cause) => {
                            self.capacity_plan_error("format-failure-evidence-storage", cause)
                        }
                        cause => self.abort(ScopeAbortCause::FormatFailureDetail(cause)),
                    }
                })?;
            self.resize_live(&mut storage, actual_storage_bytes)?;
            {
                let mut failure_storage = match self.format_failure_storage.try_borrow_mut() {
                    Ok(storage) => storage,
                    Err(_) => return Err(self.reentrant_capability_failure()),
                };
                if failure_storage.is_some() {
                    return Err(self.abort(ScopeAbortCause::DescriptorContract {
                        code: "format-failure-storage-already-active",
                    }));
                }
                *failure_storage = Some(storage.into_reservation());
            }

            let Some(producer) = producer.as_ref() else {
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "format-failure-producer-was-lost",
                }));
            };
            let code_result = {
                let _callback = self.enter_parse_callback()?;
                catch_unwind(AssertUnwindSafe(|| producer.code()))
            };
            let code = match code_result {
                Ok(code) => code,
                Err(payload) => {
                    discard_untrusted_panic_payload(payload);
                    return Err(self.abort(ScopeAbortCause::TrustedExtension {
                        boundary: TrustedExtensionBoundary::FormatFailureDetail,
                        code: "format-failure-code-panicked",
                    }));
                }
            };
            self.preflight_active()?;
            if !valid_diagnostic_code(code) {
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "invalid-format-failure-code",
                }));
            }
            evidence
                .store_code(code)
                .map_err(|cause| self.abort(ScopeAbortCause::FormatFailureDetail(cause)))?;
            let details_result = {
                let _callback = self.enter_parse_callback()?;
                catch_unwind(AssertUnwindSafe(|| {
                    producer.visit_details(&mut evidence);
                }))
            };
            if let Err(payload) = details_result {
                discard_untrusted_panic_payload(payload);
                return Err(self.abort(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::FormatFailureDetail,
                    code: "format-failure-detail-panicked",
                }));
            }
            self.preflight_active()?;
            evidence
                .finish()
                .map_err(|cause| self.abort(ScopeAbortCause::FormatFailureDetail(cause)))
        })();

        let producer_drop = match self.enter_parse_callback() {
            Ok(_callback) => catch_unwind(AssertUnwindSafe(|| drop(producer.take()))),
            Err(failure) => {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(producer.take()))) {
                    discard_untrusted_panic_payload(payload);
                }
                return failure;
            }
        };
        if let Err(payload) = producer_drop {
            discard_untrusted_panic_payload(payload);
            return self.abort(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::FormatFailureDetail,
                code: "format-failure-producer-drop-panicked",
            });
        }

        match captured {
            Ok(evidence) => match self.preflight_active() {
                Ok(()) => ParseFailure::format(FormatFailureEnvelope::seal(
                    self.entry.descriptor_handle(),
                    evidence,
                )),
                Err(failure) => failure,
            },
            Err(failure) => failure,
        }
    }

    pub(crate) fn take_terminal_abort(&self) -> Option<ScopeAbort> {
        self.terminal.borrow_mut().take().map(ScopeAbort::new)
    }

    fn dependency_failure(
        &self,
        relation: Option<PendingResourceRelation>,
        identity: Option<DependencyIdentity>,
        normalized_path: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            relation,
            identity,
            evidence: DependencyFailureEvidence::Normalized(normalized_path),
            cause: DependencyResolutionCause::Owned(cause),
            cache_disposition: DependencyCacheDisposition::Fresh,
        })
    }

    fn dependency_failure_with_rejected_reference(
        &self,
        relation: Option<PendingResourceRelation>,
        evidence_path: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            relation,
            identity: None,
            evidence: DependencyFailureEvidence::Rejected(Some(evidence_path)),
            cause: DependencyResolutionCause::Owned(cause),
            cache_disposition: DependencyCacheDisposition::Fresh,
        })
    }

    fn cached_dependency_failure(
        &self,
        relation: PendingResourceRelation,
        identity: DependencyIdentity,
        normalized_path: MeteredDependencyPath,
        index: usize,
        cache_disposition: DependencyCacheDisposition,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            relation: Some(relation),
            identity: Some(identity),
            evidence: DependencyFailureEvidence::Normalized(normalized_path),
            cause: DependencyResolutionCause::Cached { index },
            cache_disposition,
        })
    }

    fn dependency_failure_without_path(
        &self,
        relation: Option<PendingResourceRelation>,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            relation,
            identity: None,
            evidence: DependencyFailureEvidence::Rejected(None),
            cause: DependencyResolutionCause::Owned(cause),
            cache_disposition: DependencyCacheDisposition::Fresh,
        })
    }

    fn store_dependency_failure(
        &self,
        context: DependencyResolutionContext,
    ) -> DependencyResolutionFailure {
        let mut stored = self.dependency_failure.borrow_mut();
        if stored.is_none() {
            *stored = Some(context);
        }
        DependencyResolutionFailure
    }

    fn take_dependency_failure(
        &self,
        _failure: DependencyResolutionFailure,
    ) -> DependencyResolutionContext {
        self.dependency_failure
            .borrow_mut()
            .take()
            .unwrap_or(DependencyResolutionContext {
                relation: None,
                identity: None,
                evidence: DependencyFailureEvidence::Rejected(None),
                cause: DependencyResolutionCause::Owned(ScopeAbortCause::DescriptorContract {
                    code: "dependency-failure-marker-without-context",
                }),
                cache_disposition: DependencyCacheDisposition::Fresh,
            })
    }

    fn normalize_dependency_path_from(
        &self,
        origin: &AssetPath,
        raw_reference: &str,
    ) -> Result<MeteredDependencyPath, ScopeAbortCause> {
        let envelope = dependency_normalization_capacity_envelope(origin, raw_reference)?;
        let reservation = self.reserve_dependency_storage_cause(envelope)?;
        let path = origin
            .resolve_from_file(raw_reference)
            .map_err(dependency_path_cause)?;
        self.finish_metered_dependency_path(path, reservation, "dependency-path-normalization")
    }

    fn clone_metered_dependency_path(
        &self,
        source: &AssetPath,
        resource: &'static str,
    ) -> Result<MeteredDependencyPath, ScopeAbortCause> {
        let plan = CapacityPlan::for_type::<u8>(0, 0, source.as_str().len())
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        let envelope = plan
            .envelope_delta_bytes()
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        let reservation = self.reserve_dependency_storage_cause(envelope)?;
        let path = source
            .try_clone()
            .map_err(|_| ScopeAbortCause::Allocation { resource })?;
        self.finish_metered_dependency_path(path, reservation, resource)
    }

    fn finish_metered_dependency_path(
        &self,
        path: AssetPath,
        mut reservation: ImportStoreReservation,
        resource: &'static str,
    ) -> Result<MeteredDependencyPath, ScopeAbortCause> {
        let actual =
            u64::try_from(path.capacity_bytes()).map_err(|_| ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            })?;
        if actual > reservation.bytes() {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-path-reservation-underestimated",
            });
        }
        self.resize_dependency_storage_cause(&mut reservation, actual)?;
        Ok(MeteredDependencyPath::new(path, reservation))
    }

    fn authorize_dependency<'scope>(
        &'scope self,
        pending: PendingResourceRelation,
        origin_io: &'scope dyn AssetIo,
        origin_authority: &AuthorityDomain,
        origin_source: &AssetPath,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> Result<
        (
            PendingResourceRelation,
            MeteredDependencyPath,
            AuthorizedDependency<'scope>,
        ),
        DependencyResolutionFailure,
    > {
        if raw_reference.len() > self.limits.max_path_length {
            let origin_source = match self
                .clone_metered_dependency_path(origin_source, "dependency-failure-source")
            {
                Ok(origin_source) => origin_source,
                Err(cause) => {
                    return Err(self.dependency_failure_without_path(Some(pending), cause));
                }
            };
            return Err(self.dependency_failure_with_rejected_reference(
                Some(pending),
                origin_source,
                ScopeAbortCause::ResourceLimit {
                    resource: "dependency-reference-bytes",
                    limit: self.limits.max_path_length as u64,
                },
            ));
        }
        let (normalized, authorized) = match self.resolver {
            ResolverBinding::Registered(profile) => {
                let envelope = match dependency_normalization_capacity_envelope(
                    origin_source,
                    raw_reference,
                ) {
                    Ok(envelope) => envelope,
                    Err(cause) => {
                        return Err(self.dependency_failure_without_path(Some(pending), cause));
                    }
                };
                let reservation = match self.reserve_dependency_storage_cause(envelope) {
                    Ok(reservation) => reservation,
                    Err(cause) => {
                        return Err(self.dependency_failure_without_path(Some(pending), cause));
                    }
                };
                let reference = match profile
                    .semantic_subject()
                    .normalizer()
                    .normalize_from_file(origin_source, raw_reference)
                {
                    Ok(reference) => reference,
                    Err(PathError::AllocationFailed) => {
                        return Err(self.dependency_failure_without_path(
                            Some(pending),
                            ScopeAbortCause::Allocation {
                                resource: "dependency-path-normalization",
                            },
                        ));
                    }
                    Err(cause) => {
                        let origin_source = match self.clone_metered_dependency_path(
                            origin_source,
                            "dependency-failure-source",
                        ) {
                            Ok(origin_source) => origin_source,
                            Err(clone_cause) => {
                                return Err(self
                                    .dependency_failure_without_path(Some(pending), clone_cause));
                            }
                        };
                        return Err(self.dependency_failure_with_rejected_reference(
                            Some(pending),
                            origin_source,
                            dependency_path_cause(cause),
                        ));
                    }
                };
                let path = match reference.path().try_clone() {
                    Ok(path) => path,
                    Err(_) => {
                        return Err(self.dependency_failure_without_path(
                            Some(pending),
                            ScopeAbortCause::Allocation {
                                resource: "dependency-path-normalization",
                            },
                        ));
                    }
                };
                let normalized = match self.finish_metered_dependency_path(
                    path,
                    reservation,
                    "dependency-path-normalization",
                ) {
                    Ok(normalized) => normalized,
                    Err(cause) => {
                        return Err(self.dependency_failure_without_path(Some(pending), cause));
                    }
                };
                let resolution = match profile.resolve_same_authority(
                    origin_io,
                    origin_authority.clone(),
                    reference,
                ) {
                    Ok(resolution) => resolution,
                    Err(_) => {
                        return Err(self.dependency_failure(
                            Some(pending),
                            None,
                            normalized,
                            ScopeAbortCause::DescriptorContract {
                                code: "registered-resolver-normalizer-mismatch",
                            },
                        ));
                    }
                };
                let authorized = AuthorizedDependency::from_registered(resolution);
                (normalized, authorized)
            }
            ResolverBinding::Custom(resolver) => {
                let normalized = match self
                    .normalize_dependency_path_from(origin_source, raw_reference)
                {
                    Ok(normalized) => normalized,
                    Err(cause @ ScopeAbortCause::Path(_)) => {
                        let origin_source = match self.clone_metered_dependency_path(
                            origin_source,
                            "dependency-failure-source",
                        ) {
                            Ok(origin_source) => origin_source,
                            Err(clone_cause) => {
                                return Err(self
                                    .dependency_failure_without_path(Some(pending), clone_cause));
                            }
                        };
                        return Err(self.dependency_failure_with_rejected_reference(
                            Some(pending),
                            origin_source,
                            cause,
                        ));
                    }
                    Err(cause) => {
                        return Err(self.dependency_failure_without_path(Some(pending), cause));
                    }
                };
                let reference = DependencyReference::new(raw_reference, normalized.path());
                let authorized = match catch_unwind(AssertUnwindSafe(|| {
                    resolver.authorize(origin_io, origin_authority, origin_source, role, reference)
                })) {
                    Err(payload) => {
                        discard_untrusted_panic_payload(payload);
                        return Err(self.dependency_failure(
                            Some(pending),
                            None,
                            normalized,
                            ScopeAbortCause::TrustedExtension {
                                boundary: TrustedExtensionBoundary::DependencyResolver,
                                code: "resolver-panicked",
                            },
                        ));
                    }
                    Ok(Err(cause)) => {
                        return Err(self.dependency_failure(
                            Some(pending),
                            None,
                            normalized,
                            ScopeAbortCause::DependencyPolicy(cause),
                        ));
                    }
                    Ok(Ok(authorized)) => authorized,
                };
                (normalized, authorized)
            }
        };

        Ok((pending, normalized, authorized))
    }

    fn prepare_authorized_target<'scope>(
        &'scope self,
        mut pending: PendingResourceRelation,
        normalized: MeteredDependencyPath,
        authorized: AuthorizedDependency<'scope>,
    ) -> Result<
        (
            PendingResourceRelation,
            MeteredDependencyPath,
            AuthorizedDependency<'scope>,
        ),
        DependencyResolutionFailure,
    > {
        let prepared = match self.resource_graph.try_borrow_mut() {
            Ok(mut graph) => graph
                .prepare_target(self.host, &mut pending, authorized.identity())
                .map_err(|cause| cause.into_scope_cause()),
            Err(_) => Err(ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }),
        };
        match prepared {
            Ok(()) => Ok((pending, normalized, authorized)),
            Err(cause) => Err(self.dependency_failure(Some(pending), None, normalized, cause)),
        }
    }

    fn acquire_authorized<'scope>(
        &'scope self,
        pending: PendingResourceRelation,
        normalized: MeteredDependencyPath,
        authorized: AuthorizedDependency<'scope>,
    ) -> Result<
        (
            DependencySnapshot<'scope>,
            PendingResourceRelation,
            MeteredDependencyPath,
            usize,
        ),
        DependencyResolutionFailure,
    > {
        let (io, identity, stable_identity_schema, registered_resolver) = authorized.into_parts();
        let backend_authority = if std::ptr::eq(io, self.primary_io) {
            self.primary_authority.clone()
        } else {
            match catch_unwind(AssertUnwindSafe(|| io.authority_domain())) {
                Ok(authority) => authority,
                Err(payload) => {
                    discard_untrusted_panic_payload(payload);
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        ScopeAbortCause::TrustedExtension {
                            boundary: TrustedExtensionBoundary::AssetIo,
                            code: "dependency-authority-panicked",
                        },
                    ));
                }
            }
        };
        if &backend_authority != identity.authority() {
            return Err(self.dependency_failure(
                Some(pending),
                Some(identity),
                normalized,
                ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "authorized-backend-authority-mismatch",
                },
            ));
        }
        let cached = {
            let graph = self.resource_graph.borrow();
            match graph.acquisition_state(&identity) {
                Some(ResourceAcquisitionState::Available {
                    cache_index,
                    bytes,
                    stable_identity_schema,
                }) => Some((cache_index, bytes, stable_identity_schema)),
                Some(ResourceAcquisitionState::Failed {
                    cache_index,
                    stable_identity_schema: cached_stable_identity_schema,
                }) => {
                    if !stable_identity_matches(
                        stable_identity_schema,
                        cached_stable_identity_schema,
                    ) {
                        return Err(self.dependency_failure(
                            Some(pending),
                            Some(identity),
                            normalized,
                            ScopeAbortCause::DescriptorContract {
                                code: "dependency-resolver-identity-mismatch",
                            },
                        ));
                    }
                    return Err(self.cached_dependency_failure(
                        pending,
                        identity,
                        normalized,
                        cache_index,
                        DependencyCacheDisposition::Reused,
                    ));
                }
                Some(ResourceAcquisitionState::Resolving) => {
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-resolution-reentered",
                        },
                    ));
                }
                None => None,
            }
        };
        if let Some((origin_node, bytes, cached_stable_identity_schema)) = cached {
            if !stable_identity_matches(stable_identity_schema, cached_stable_identity_schema) {
                return Err(self.dependency_failure(
                    Some(pending),
                    Some(identity),
                    normalized,
                    ScopeAbortCause::DescriptorContract {
                        code: "dependency-resolver-identity-mismatch",
                    },
                ));
            }
            let snapshot_identity = match self
                .clone_dependency_snapshot_identity(&identity, cached_stable_identity_schema)
            {
                Ok(identity) => identity,
                Err(cause) => {
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        cause,
                    ));
                }
            };
            return Ok((
                DependencySnapshot::new(
                    self.dependency_origin_issuer,
                    io,
                    origin_node,
                    snapshot_identity,
                    bytes,
                    DependencyCacheDisposition::Reused,
                ),
                pending,
                normalized,
                origin_node,
            ));
        }

        let capacity = match self.resource_graph.try_borrow_mut() {
            Ok(mut graph) => graph.reserve_acquisition_capacity(self.host),
            Err(_) => Err(ResourceGraphError::Scope(
                ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                },
            )),
        };
        if let Err(cause) = capacity {
            return Err(self.dependency_failure(
                Some(pending),
                Some(identity),
                normalized,
                cause.into_scope_cause(),
            ));
        }
        let cache_identity_path_reservation =
            match self.reserve_dependency_storage_cause(identity.path().as_str().len() as u64) {
                Ok(reservation) => reservation,
                Err(cause) => {
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        cause,
                    ));
                }
            };
        let cache_identity = match identity.try_clone() {
            Ok(cache_identity) => cache_identity,
            Err(_) => {
                return Err(self.dependency_failure(
                    Some(pending),
                    Some(identity),
                    normalized,
                    ScopeAbortCause::Allocation {
                        resource: "dependency-cache-identity",
                    },
                ));
            }
        };
        let marked = match self
            .resource_graph
            .borrow_mut()
            .begin_acquisition(cache_identity, cache_identity_path_reservation)
        {
            Ok(marked) => marked,
            Err(cause) => {
                return Err(self.dependency_failure(
                    Some(pending),
                    Some(identity),
                    normalized,
                    cause.into_scope_cause(),
                ));
            }
        };
        if !marked {
            return Err(self.dependency_failure(
                Some(pending),
                Some(identity),
                normalized,
                ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-state-raced",
                },
            ));
        }

        let acquired = self
            .open_and_read_dependency(io, &identity)
            .and_then(|bytes| {
                let snapshot_bytes = self.share_dependency_snapshot(bytes)?;
                let snapshot_identity =
                    self.clone_dependency_snapshot_identity(&identity, stable_identity_schema)?;
                Ok((snapshot_bytes, snapshot_identity))
            });
        match acquired {
            Ok((snapshot_bytes, snapshot_identity)) => {
                let origin_node = match self.resource_graph.borrow_mut().complete_acquisition(
                    &identity,
                    Arc::clone(&snapshot_bytes),
                    stable_identity_schema,
                ) {
                    Ok(origin_node) => origin_node,
                    Err(_) => {
                        let _ = self
                            .resource_graph
                            .borrow_mut()
                            .abandon_acquisition(&identity);
                        return Err(self.dependency_failure(
                            Some(pending),
                            Some(identity),
                            normalized,
                            ScopeAbortCause::DescriptorContract {
                                code: "dependency-cache-transition-missing",
                            },
                        ));
                    }
                };
                Ok((
                    DependencySnapshot::new(
                        self.dependency_origin_issuer,
                        io,
                        origin_node,
                        snapshot_identity,
                        snapshot_bytes,
                        DependencyCacheDisposition::Fresh,
                    ),
                    pending,
                    normalized,
                    origin_node,
                ))
            }
            Err(cause) => Err(self.finish_dependency_resolution_failure(
                pending,
                identity,
                DependencyFailureObservationContext {
                    stable_identity_schema,
                    registered_resolver,
                    io,
                },
                normalized,
                cause,
            )),
        }
    }

    fn finish_dependency_resolution_failure(
        &self,
        pending: PendingResourceRelation,
        identity: DependencyIdentity,
        observation_context: DependencyFailureObservationContext<'_>,
        normalized: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        let observation = match self.registered_negative_observation(
            observation_context.registered_resolver,
            observation_context.io,
            &identity,
            observation_context.stable_identity_schema,
            &cause,
        ) {
            Ok(observation) => observation,
            Err(proof_cause) => {
                if self
                    .resource_graph
                    .borrow_mut()
                    .abandon_acquisition(&identity)
                    .is_err()
                {
                    return self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-cache-transition-missing",
                        },
                    );
                }
                return self.dependency_failure(
                    Some(pending),
                    Some(identity),
                    normalized,
                    proof_cause,
                );
            }
        };
        let cached_index = self
            .resource_graph
            .borrow_mut()
            .complete_failed_acquisition(
                &identity,
                cause,
                observation_context.stable_identity_schema,
                observation,
            );
        match cached_index {
            Ok(index) => self.cached_dependency_failure(
                pending,
                identity,
                normalized,
                index,
                DependencyCacheDisposition::Fresh,
            ),
            Err(cause) => {
                if self
                    .resource_graph
                    .borrow_mut()
                    .abandon_acquisition(&identity)
                    .is_err()
                {
                    return self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-cache-transition-missing",
                        },
                    );
                }
                self.dependency_failure(Some(pending), Some(identity), normalized, cause)
            }
        }
    }

    fn registered_negative_observation<'target>(
        &self,
        registered_resolver: Option<&'static RegisteredResolverProfile>,
        io: &'target dyn AssetIo,
        identity: &'target DependencyIdentity,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
        cause: &ScopeAbortCause,
    ) -> Result<PendingNegativeObservation<'target>, ScopeAbortCause> {
        let Some(resolver) = registered_resolver else {
            return Ok(PendingNegativeObservation::None);
        };
        let registered_cause = match cause {
            ScopeAbortCause::AssetOpen(error) if error.kind() == AssetOpenErrorKind::NotFound => {
                RegisteredUnavailableCause::Missing
            }
            ScopeAbortCause::AssetOpen(_) => RegisteredUnavailableCause::OpenFailed,
            ScopeAbortCause::AssetRead(error)
                if !matches!(error.kind(), AssetReadErrorKind::ContractViolation(_)) =>
            {
                RegisteredUnavailableCause::ReadFailed
            }
            _ => return Ok(PendingNegativeObservation::None),
        };
        let Some(plan) = resolver.plan_negative_proof(io, identity.path(), registered_cause) else {
            return Ok(PendingNegativeObservation::None);
        };
        let stable_identity_schema =
            stable_identity_schema.ok_or(ScopeAbortCause::DescriptorContract {
                code: "registered-negative-proof-missing-stable-identity",
            })?;
        let canonical_bytes =
            registered_negative_proof_canonical_len(&plan, stable_identity_schema).map_err(
                |_| ScopeAbortCause::DescriptorContract {
                    code: "registered-negative-proof-canonical-shape",
                },
            )?;
        let caps = plan.scheme().semantic_subject().caps();
        if canonical_bytes > caps.canonical_bytes() {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "registered-negative-proof-scheme-cap-exceeded",
            });
        }
        let charges = [
            (
                ImportDimension::NegativeProofCanonicalBytes,
                canonical_bytes,
            ),
            (
                ImportDimension::NegativeProofTemporaryBytes,
                caps.temporary_bytes(),
            ),
            (
                ImportDimension::NegativeProofConstructionWork,
                caps.construction_work(),
            ),
            (
                ImportDimension::NegativeProofVerificationWork,
                caps.verification_work(),
            ),
        ];
        let proof = self.host.with_observed_work(charges, |work| {
            let proof = plan
                .verify(identity.path(), registered_cause)
                .map_err(NegativeProofActionError::Proof)?;
            work.complete([
                canonical_bytes,
                caps.temporary_bytes(),
                caps.construction_work(),
                caps.verification_work(),
            ])
            .map_err(NegativeProofActionError::Store)?;
            Ok(proof)
        });
        match proof {
            Ok(proof) => Ok(PendingNegativeObservation::Proof(proof)),
            Err(ImportObservedWorkFailure::Store(cause))
            | Err(ImportObservedWorkFailure::Action(NegativeProofActionError::Store(cause))) => {
                Err(cause.into_scope_abort_cause())
            }
            Err(ImportObservedWorkFailure::Action(NegativeProofActionError::Proof(cause))) => {
                Err(registered_negative_proof_error(cause))
            }
        }
    }

    fn open_and_read_dependency(
        &self,
        io: &dyn AssetIo,
        identity: &DependencyIdentity,
    ) -> AssetSnapshotResult {
        self.poll_cause()?;
        self.charge_open_attempt()?;
        let opened = catch_unwind(AssertUnwindSafe(|| io.open(identity.path())))
            .map_err(|payload| {
                discard_untrusted_panic_payload(payload);
                ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "asset-open-panicked",
                }
            })?
            .map_err(ScopeAbortCause::AssetOpen)?;
        if opened.authority_domain() != identity.authority() || opened.path() != identity.path() {
            return Err(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "opened-object-identity-mismatch",
            });
        }
        self.charge_open_consumed()?;
        self.admit_dependency_acquisition_work()?;
        let bytes = read_opened_asset(
            self.host,
            opened,
            ImportLedgerPhase::DependencyAcquisition,
            ImportProgressPhase::AcquireDependency,
            ImportDimension::DependencyBytes,
            self.limits.max_dependency_asset_bytes,
            self.limits.max_total_asset_bytes,
        )?;
        self.charge_dependency_acquisition_work_consumed()?;
        Ok(bytes)
    }

    fn share_dependency_snapshot(
        &self,
        bytes: AccountedVec<u8>,
    ) -> Result<Arc<SharedDependencySnapshot>, ScopeAbortCause> {
        share_dependency_snapshot_with(bytes, |control_bytes| {
            self.reserve_dependency_storage_cause(control_bytes)
        })
    }

    fn clone_dependency_snapshot_identity(
        &self,
        source: &DependencyIdentity,
        stable_schema: Option<&'static StableResourceIdentitySchema>,
    ) -> Result<DependencySnapshotIdentity, ScopeAbortCause> {
        let reservation =
            self.reserve_dependency_storage_cause(source.path().as_str().len() as u64)?;
        let identity = source
            .try_clone()
            .map_err(|_| ScopeAbortCause::Allocation {
                resource: "dependency-snapshot-identity",
            })?;
        Ok(DependencySnapshotIdentity::new(
            identity,
            stable_schema,
            reservation,
        ))
    }

    fn charge_logical_reference(&self) -> Result<(), DependencyResolutionFailure> {
        let result = self
            .host
            .charge_attempted(ImportDimension::DependencyReference, 1)
            .and_then(|()| {
                self.host
                    .charge_consumed(ImportDimension::DependencyReference, 1)
            });
        let Err(cause) = result else {
            return Ok(());
        };
        Err(self.dependency_failure_without_path(None, cause.into_scope_abort_cause()))
    }

    fn charge_open_attempt(&self) -> Result<(), ScopeAbortCause> {
        self.host
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn charge_open_consumed(&self) -> Result<(), ScopeAbortCause> {
        self.host
            .charge_consumed(ImportDimension::AssetOpen, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn admit_dependency_acquisition_work(&self) -> Result<(), ScopeAbortCause> {
        self.host
            .charge_attempted(ImportDimension::DependencyAcquisitionWork, 1)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let state = self
            .host
            .dimension_state(ImportDimension::DependencyAcquisitionWork)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let value = state.consumed().checked_add(1).ok_or({
            ScopeAbortCause::Ledger(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Consumed,
                dimension: ImportDimension::DependencyAcquisitionWork,
            })
        })?;
        if value > self.limits.max_dependency_acquisition_work {
            return Err(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: ImportDimension::DependencyAcquisitionWork,
                value,
                limit: self.limits.max_dependency_acquisition_work,
            }));
        }
        Ok(())
    }

    fn charge_dependency_acquisition_work_consumed(&self) -> Result<(), ScopeAbortCause> {
        self.host
            .charge_consumed(ImportDimension::DependencyAcquisitionWork, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn poll_cause(&self) -> Result<(), ScopeAbortCause> {
        self.host
            .poll()
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resolve_optional_failure(
        &self,
        role: &DependencyRoleId,
        failure: DependencyResolutionFailure,
    ) -> ParseResult<DependencyOutcome<'_>> {
        let mut failure = self.take_dependency_failure(failure);
        let (eligible, owner, disposition) = {
            let cache = self
                .resource_graph
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let cause = failure.cause(&cache).map_err(|cause| self.abort(cause))?;
            let disposition = failure
                .disposition(&cache)
                .map_err(|cause| self.abort(cause))?;
            (optional_eligible(cause), cause.owner(), disposition)
        };
        if !self
            .loss_policy
            .allows(ImportLossKind::OptionalDependencyUnavailable)
            || !eligible
        {
            return Err(self.abort_dependency_context(failure));
        }
        let Some(evidence_path) = failure.evidence.evidence_path() else {
            return Err(self.abort_dependency_context(failure));
        };
        let evidence = self.clone_optional_dependency_evidence(
            failure.identity.as_ref(),
            evidence_path.path(),
            &owner,
            failure.cache_disposition,
        )?;
        let retained_evidence = self.clone_optional_dependency_evidence(
            failure.identity.as_ref(),
            evidence_path.path(),
            &owner,
            failure.cache_disposition,
        )?;
        {
            let mut retained = self
                .optional_dependency_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            retained
                .try_reserve(
                    1,
                    "optional-dependency-evidence-capacity",
                    |capacity_bytes| self.reserve_dependency_storage_cause(capacity_bytes),
                )
                .map_err(|cause| self.abort(cause))?;
            if let Err(retained_evidence) = retained.try_push_reserved(retained_evidence) {
                drop(retained_evidence);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "optional-dependency-evidence-slots-not-admitted",
                }));
            }
        }
        self.complete_unavailable_relation(&mut failure, disposition)?;
        self.emit_diagnostic(
            DiagnosticSeverity::Warning,
            DiagnosticCode::new("import.optional_dependency_unavailable")
                .map_err(|cause| self.abort(ScopeAbortCause::Diagnostic(cause)))?,
            format_args!("optional dependency role {role} is unavailable"),
        )?;
        Ok(DependencyOutcome::unavailable(evidence))
    }

    fn complete_unavailable_relation(
        &self,
        failure: &mut DependencyResolutionContext,
        disposition: ResourceUnavailableDisposition,
    ) -> ParseResult<()> {
        let relation = failure.relation.take().ok_or_else(|| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-terminal-relation-missing",
            })
        })?;
        let normalized_reference = failure.normalized_manifest_reference();
        self.resource_graph
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .complete_unavailable(
                self.host,
                relation,
                normalized_reference,
                failure.identity.as_ref(),
                disposition,
            )
            .map_err(|cause| self.abort(cause.into_scope_cause()))
    }

    fn abort_dependency_failure(&self, failure: DependencyResolutionFailure) -> ParseFailure {
        self.abort_dependency_context(self.take_dependency_failure(failure))
    }

    fn abort_dependency_context(&self, failure: DependencyResolutionContext) -> ParseFailure {
        let (disposition, mut cause, relation, identity, evidence) = {
            let graph = match self.resource_graph.try_borrow() {
                Ok(graph) => graph,
                Err(_) => {
                    let parse_failure = self.abort(ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    });
                    drop(failure);
                    return parse_failure;
                }
            };
            let disposition = failure
                .disposition(&graph)
                .unwrap_or_else(|cause| unavailable_disposition(&cause));
            let (cause, relation, identity, evidence) = failure.into_parts(&graph);
            (disposition, cause, relation, identity, evidence)
        };
        let mut graph = self
            .resource_graph
            .try_borrow_mut()
            .expect("resource graph borrow was released before mutation");
        let normalized_reference = evidence.manifest_reference();
        if let Some(relation) = relation
            && let Err(graph_cause) = graph.complete_unavailable(
                self.host,
                relation,
                normalized_reference,
                identity.as_ref(),
                disposition,
            )
        {
            cause = graph_cause.into_scope_cause();
        }
        drop(graph);
        drop(identity);
        let parse_failure = self.abort(cause);
        drop(evidence);
        parse_failure
    }

    fn clone_optional_dependency_evidence(
        &self,
        identity: Option<&DependencyIdentity>,
        normalized_path: &AssetPath,
        owner: &crate::CauseOwner,
        cache_disposition: DependencyCacheDisposition,
    ) -> ParseResult<OptionalDependencyEvidence> {
        let normalized_path_reservation =
            self.reserve_dependency_storage(normalized_path.as_str().len() as u64)?;
        let normalized_path = normalized_path.try_clone().map_err(|_| {
            self.abort(ScopeAbortCause::Allocation {
                resource: "optional-dependency-evidence-path",
            })
        })?;
        let (identity, identity_path_reservation) = match identity {
            Some(identity) => {
                let reservation =
                    self.reserve_dependency_storage(identity.path().as_str().len() as u64)?;
                let identity = identity.try_clone().map_err(|_| {
                    self.abort(ScopeAbortCause::Allocation {
                        resource: "optional-dependency-evidence-identity",
                    })
                })?;
                (Some(identity), Some(reservation))
            }
            None => (None, None),
        };
        Ok(OptionalDependencyEvidence::new(
            identity,
            normalized_path,
            owner.clone(),
            cache_disposition,
            identity_path_reservation,
            normalized_path_reservation,
        ))
    }

    fn reserve_dependency_storage(&self, bytes: u64) -> ParseResult<ImportStoreReservation> {
        self.reserve_dependency_storage_cause(bytes)
            .map_err(|cause| self.abort(cause))
    }

    fn reserve_dependency_storage_cause(
        &self,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        self.host
            .reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                bytes,
            )
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resize_dependency_storage_cause(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ScopeAbortCause> {
        self.host
            .resize(reservation, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        let mut terminal = self.terminal.borrow_mut();
        if terminal.is_none() {
            *terminal = Some(cause);
        }
        ParseFailure::scope()
    }

    fn preflight_active(&self) -> ParseResult<()> {
        if self.callback_active.get() {
            return Err(self.reentrant_capability_failure());
        }
        if self.terminal.borrow().is_some() {
            Err(ParseFailure::scope())
        } else {
            Ok(())
        }
    }

    pub(crate) fn source_scope_preflight(&self) -> ParseResult<()> {
        self.preflight_active()
    }

    pub(crate) fn source_scope_abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        self.abort(cause)
    }

    pub(crate) fn source_scope_charge_batch(
        &self,
        charges: &[(ImportDimension, u64); 2],
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .charge_observed_batch(charges)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn source_scope_charge_attempted(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn source_scope_charge_consumed(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .charge_consumed(dimension, amount)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn charge_source_scope_bundle_consumed(
        &self,
        loss_bytes: u64,
        diagnostic_bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .charge_consumed_batch(&[
                (ImportDimension::DiagnosticCount, 1),
                (ImportDimension::DiagnosticBytes, diagnostic_bytes),
                (ImportDimension::LossEvidenceCount, 1),
                (ImportDimension::LossEvidenceBytes, loss_bytes),
                (ImportDimension::SourceScopeRecordCount, 1),
            ])
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn reentrant_capability_failure(&self) -> ParseFailure {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    }

    fn enter_parse_callback(&self) -> ParseResult<ParseCallbackGuard<'_>> {
        if self.callback_active.replace(true) {
            return Err(self.reentrant_capability_failure());
        }
        Ok(ParseCallbackGuard {
            active: &self.callback_active,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStorageClass {
    BackendDecode,
    DecodedText,
    LineIndex,
    FormatNormalization,
    SceneConstruction,
}

impl ImportStorageClass {
    pub(crate) const fn accounting(self) -> (ImportLedgerPhase, ImportDimension) {
        match self {
            Self::BackendDecode => (
                ImportLedgerPhase::BackendDecode,
                ImportDimension::ScratchBytes,
            ),
            Self::DecodedText => (
                ImportLedgerPhase::BackendDecode,
                ImportDimension::DecodedTextBytes,
            ),
            Self::LineIndex => (
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::LineIndexBytes,
            ),
            Self::FormatNormalization => (
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::ScratchBytes,
            ),
            Self::SceneConstruction => (
                ImportLedgerPhase::SceneConstruction,
                ImportDimension::CanonicalOutputBytes,
            ),
        }
    }
}

pub(crate) const METERED_COPY_POLL_INTERVAL: usize = 4 * 1024;

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_METERED_ALLOCATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_METERED_QUALIFICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_metered_allocation() -> bool {
    FAIL_NEXT_METERED_ALLOCATION.with(|fault| fault.replace(false))
}

#[cfg(test)]
pub(crate) fn inject_next_metered_allocation_failure() {
    FAIL_NEXT_METERED_ALLOCATION.with(|fault| fault.set(true));
}

#[cfg(not(test))]
const fn fail_next_metered_allocation() -> bool {
    false
}

#[cfg(test)]
fn fail_next_metered_qualification() -> bool {
    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.replace(false))
}

#[cfg(not(test))]
const fn fail_next_metered_qualification() -> bool {
    false
}

fn metered_copy_chunk_len<T>() -> usize {
    METERED_COPY_POLL_INTERVAL
        .checked_div(size_of::<T>())
        .unwrap_or(METERED_COPY_POLL_INTERVAL)
        .max(1)
}

fn append_metered_string_with_meter(
    meter: &StorageMeter<'_>,
    output: &mut String,
    value: &str,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < value.len() {
        meter.poll()?;
        let mut end = start
            .checked_add(METERED_COPY_POLL_INTERVAL)
            .map_or(value.len(), |end| end.min(value.len()));
        while end > start && !value.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&value[start..end]);
        start = end;
    }
    Ok(())
}

impl StorageMeter<'_> {
    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        let mut terminal = self.terminal.borrow_mut();
        if terminal.is_none() {
            *terminal = Some(cause);
        }
        ParseFailure::scope()
    }

    fn preflight_active(&self) -> ParseResult<()> {
        if self.callback_active.get() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }));
        }
        if self.terminal.borrow().is_some() {
            Err(ParseFailure::scope())
        } else {
            Ok(())
        }
    }

    fn poll(&self) -> ParseResult<()> {
        self.preflight_active()?;
        self.host
            .poll()
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn reserve(
        &self,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<Option<ImportStoreReservation>> {
        self.preflight_active()?;
        if bytes == 0 {
            return Ok(None);
        }
        let (phase, dimension) = class.accounting();
        self.host
            .reserve(phase, dimension, bytes)
            .map(Some)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn resize_exact(
        &self,
        reservation: &mut Option<ImportStoreReservation>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        match (reservation.as_mut(), bytes) {
            (None, 0) => Ok(()),
            (Some(_), 0) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-unused-capacity-reservation",
            })),
            (Some(token), bytes) => self
                .host
                .resize(token, bytes)
                .map_err(|cause| self.abort(cause.into_scope_abort_cause())),
            (None, _) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-capacity-without-reservation",
            })),
        }
    }

    fn capacity_error(&self, resource: &'static str, cause: CapacityPlanError) -> ParseFailure {
        self.abort(crate::metered_storage::capacity_plan_cause(resource, cause))
    }

    fn allocation_failure(&self, resource: &'static str) -> ParseFailure {
        self.abort(ScopeAbortCause::Allocation { resource })
    }

    fn qualification_failure(&self, resource: &'static str) -> ParseFailure {
        self.capacity_error(
            resource,
            CapacityPlanError::CapacityEnvelopeViolated {
                policy_revision: baozi_operation::CAPACITY_POLICY_REVISION,
                desired_capacity: 1,
                envelope_capacity: 1,
                actual_capacity: 2,
            },
        )
    }
}

impl<T> MeteredVec<'_, T> {
    pub fn try_reserve(&mut self, additional: usize) -> ParseResult<()> {
        self.meter.poll()?;
        let required = self
            .values
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required <= self.values.capacity() {
            return Ok(());
        }

        let (plan, replacement_plan) =
            metered_vec_replacement_plans::<T>(self.values.len(), self.values.capacity(), required)
                .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let admitted = replacement_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let mut token = self.meter.reserve(self.class, admitted)?;
        plan.verify_preflight(self.values.len(), self.values.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        replacement_plan
            .verify_preflight(0, 0)
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let mut replacement = Vec::new();
        if fail_next_metered_allocation() {
            return Err(self.meter.allocation_failure(self.resource));
        }
        replacement
            .try_reserve_exact(replacement_plan.allocator_additional())
            .map_err(|_| self.meter.allocation_failure(self.resource))?;
        let complete_bytes = replacement_plan
            .finish(replacement.len(), replacement.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        plan.finish(self.values.len(), replacement.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        self.meter.resize_exact(&mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(self.meter.qualification_failure(self.resource));
        }
        self.meter.poll()?;

        assert!(
            replacement.capacity() >= self.values.len(),
            "a prepared metered vector must retain every live element"
        );
        replacement.append(&mut self.values);
        let retired_values = std::mem::replace(&mut self.values, replacement);
        let retired_token = std::mem::replace(&mut self.capacity, token);
        drop(retired_values);
        drop(retired_token);
        Ok(())
    }

    pub fn try_push(&mut self, value: T) -> ParseResult<()> {
        self.try_reserve(1)?;
        self.values.push(value);
        Ok(())
    }

    /// Pushes into a slot admitted by an earlier reserve without polling or entering the allocator.
    pub fn try_push_reserved(&mut self, value: T) -> ParseResult<()> {
        self.meter.preflight_active()?;
        if self.values.len() == self.values.capacity() {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-capacity-not-reserved",
            }));
        }
        self.values.push(value);
        Ok(())
    }

    pub fn try_extend_from_slice(&mut self, values: &[T]) -> ParseResult<()>
    where
        T: Copy,
    {
        self.try_reserve(values.len())?;
        for chunk in values.chunks(metered_copy_chunk_len::<T>()) {
            self.meter.poll()?;
            self.values.extend_from_slice(chunk);
        }
        Ok(())
    }

    /// Extends storage admitted by an earlier reserve without polling or entering the allocator.
    ///
    /// A caller-owned work boundary must cover the copy. The fixed chunk ceiling prevents one
    /// call from weakening cancellation latency.
    pub fn try_extend_from_slice_reserved(&mut self, values: &[T]) -> ParseResult<()>
    where
        T: Copy,
    {
        self.meter.preflight_active()?;
        let required = self
            .values
            .len()
            .checked_add(values.len())
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required > self.values.capacity() {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-capacity-not-reserved",
            }));
        }
        if values.len() > metered_copy_chunk_len::<T>() {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-copy-chunk-too-large",
            }));
        }
        self.values.extend_from_slice(values);
        Ok(())
    }

    pub fn try_resize(&mut self, len: usize, value: T) -> ParseResult<()>
    where
        T: Copy,
    {
        let additional = len.saturating_sub(self.values.len());
        self.try_reserve(additional)?;
        while self.values.len() < len {
            self.meter.poll()?;
            let chunk_end = self
                .values
                .len()
                .saturating_add(metered_copy_chunk_len::<T>())
                .min(len);
            self.values.resize(chunk_end, value);
        }
        self.values.truncate(len);
        Ok(())
    }

    /// Runs a fallible heap sort without exposing the backing vector or changing its capacity.
    pub fn try_sort_unstable_by<E>(
        &mut self,
        mut compare: impl FnMut(&T, &T) -> Result<std::cmp::Ordering, E>,
    ) -> Result<(), E> {
        if self.values.len() <= 1 {
            return Ok(());
        }
        let len = self.values.len();
        let mut start = len / 2;
        while start > 0 {
            start -= 1;
            sift_down_fallible(&mut self.values, start, len, &mut compare)?;
        }
        let mut end = len;
        while end > 1 {
            end -= 1;
            self.values.swap(0, end);
            sift_down_fallible(&mut self.values, 0, end, &mut compare)?;
        }
        Ok(())
    }

    /// Partitions adjacent duplicates behind the returned retained length without dropping them.
    ///
    /// Callers remain responsible for truncating the duplicate suffix under their own work and
    /// cancellation policy.
    pub fn try_partition_dedup_by<E>(
        &mut self,
        mut equivalent: impl FnMut(&T, &T) -> Result<bool, E>,
    ) -> Result<usize, E> {
        if self.values.len() <= 1 {
            return Ok(self.values.len());
        }
        let mut write = 1;
        for read in 1..self.values.len() {
            if equivalent(&self.values[write - 1], &self.values[read])? {
                continue;
            }
            if write != read {
                self.values.swap(write, read);
            }
            write += 1;
        }
        Ok(write)
    }

    /// Consumes values in order while retaining the capacity token until the last value is gone.
    pub fn try_for_each_owned<E>(
        mut self,
        mut visit: impl FnMut(T) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut values = std::mem::take(&mut self.values);
        for value in values.drain(..) {
            visit(value)?;
        }
        Ok(())
    }
}

fn sift_down_fallible<T, E>(
    values: &mut [T],
    mut root: usize,
    end: usize,
    compare: &mut impl FnMut(&T, &T) -> Result<std::cmp::Ordering, E>,
) -> Result<(), E> {
    while root < end / 2 {
        let child = root * 2 + 1;
        let mut greater = child;
        if child + 1 < end
            && compare(&values[child], &values[child + 1])? == std::cmp::Ordering::Less
        {
            greater = child + 1;
        }
        if compare(&values[root], &values[greater])? != std::cmp::Ordering::Less {
            return Ok(());
        }
        values.swap(root, greater);
        root = greater;
    }
    Ok(())
}

impl<K: Ord, V> MeteredOrderedMap<'_, K, V> {
    pub fn try_reserve(&mut self, additional: usize) -> ParseResult<()> {
        self.meter.poll()?;
        let required = self
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required <= self.value.capacity() {
            return Ok(());
        }

        let plan = self
            .value
            .capacity_plan(additional)
            .map_err(|cause| metered_map_error(&self.meter, self.resource, cause))?;
        let admitted = plan
            .replacement_envelope_bytes()
            .map_err(|cause| metered_map_error(&self.meter, self.resource, cause))?;
        let mut token = self.meter.reserve(self.class, admitted)?;
        let replacement = self
            .value
            .prepare_capacity_replacement(plan)
            .map_err(|cause| metered_map_error(&self.meter, self.resource, cause))?;
        let complete_bytes = replacement.allocation_bytes();
        self.meter.resize_exact(&mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(self.meter.qualification_failure(self.resource));
        }
        self.meter.poll()?;

        replacement.commit();
        let retired_token = std::mem::replace(&mut self.capacity, token);
        drop(retired_token);
        Ok(())
    }

    pub fn try_insert(&mut self, key: K, value: V) -> ParseResult<Option<V>> {
        let additional = usize::from(!self.value.contains_key(&key));
        self.try_reserve(additional)?;
        self.value
            .insert_reserved(key, value)
            .map_err(|cause| metered_map_error(&self.meter, self.resource, cause))
    }
}

fn metered_map_error(
    meter: &StorageMeter<'_>,
    resource: &'static str,
    cause: OrderedMapError,
) -> ParseFailure {
    match cause {
        OrderedMapError::CapacityOverflow => meter.abort(ScopeAbortCause::ResourceLimit {
            resource,
            limit: u64::MAX,
        }),
        OrderedMapError::AllocationFailed => meter.allocation_failure(resource),
        OrderedMapError::CapacityNotReserved => meter.abort(ScopeAbortCause::DescriptorContract {
            code: "metered-ordered-map-capacity-not-reserved",
        }),
        OrderedMapError::CapacityPlan(cause) => meter.abort(capacity_plan_cause(resource, cause)),
        _ => meter.abort(ScopeAbortCause::DescriptorContract {
            code: "metered-ordered-map-capacity-error-unknown",
        }),
    }
}

impl MeteredString<'_> {
    pub fn try_reserve(&mut self, additional: usize) -> ParseResult<()> {
        self.meter.poll()?;
        let required = self
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required <= self.value.capacity() {
            return Ok(());
        }

        let (plan, replacement_plan) = metered_replacement_plans_with_element_size(
            self.value.len(),
            self.value.capacity(),
            required,
            1,
        )
        .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let admitted = replacement_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let mut token = self.meter.reserve(self.class, admitted)?;
        plan.verify_preflight(self.value.len(), self.value.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        replacement_plan
            .verify_preflight(0, 0)
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let mut replacement = String::new();
        replacement
            .try_reserve_exact(replacement_plan.allocator_additional())
            .map_err(|_| self.meter.allocation_failure(self.resource))?;
        let complete_bytes = replacement_plan
            .finish(replacement.len(), replacement.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        plan.finish(self.value.len(), replacement.capacity())
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        self.meter.resize_exact(&mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(self.meter.qualification_failure(self.resource));
        }
        append_metered_string_with_meter(&self.meter, &mut replacement, &self.value)?;
        self.meter.poll()?;

        let retired_value = std::mem::replace(&mut self.value, replacement);
        let retired_token = std::mem::replace(&mut self.capacity, token);
        drop(retired_value);
        drop(retired_token);
        Ok(())
    }

    pub fn try_push_str(&mut self, value: &str) -> ParseResult<()> {
        self.try_reserve(value.len())?;
        append_metered_string_with_meter(&self.meter, &mut self.value, value)
    }

    /// Appends bytes admitted by an earlier reserve without polling or entering the allocator.
    ///
    /// A caller-owned work boundary must cover the copy. The fixed chunk ceiling prevents one
    /// call from weakening cancellation latency.
    pub fn try_push_str_reserved(&mut self, value: &str) -> ParseResult<()> {
        self.meter.preflight_active()?;
        let required = self
            .value
            .len()
            .checked_add(value.len())
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required > self.value.capacity() {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-string-capacity-not-reserved",
            }));
        }
        if value.len() > METERED_COPY_POLL_INTERVAL {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-string-copy-chunk-too-large",
            }));
        }
        self.value.push_str(value);
        Ok(())
    }

    /// Appends one character admitted by an earlier reserve without polling or allocation.
    pub fn try_push_char_reserved(&mut self, value: char) -> ParseResult<()> {
        let mut encoded = [0_u8; 4];
        self.try_push_str_reserved(value.encode_utf8(&mut encoded))
    }

    pub fn try_push_char(&mut self, value: char) -> ParseResult<()> {
        let mut encoded = [0_u8; 4];
        let encoded = value.encode_utf8(&mut encoded);
        self.try_push_str(encoded)
    }

    pub fn try_push_usize(&mut self, value: usize) -> ParseResult<()> {
        self.try_write_fmt(format_args!("{value}"))
    }

    pub fn try_write_fmt(&mut self, arguments: fmt::Arguments<'_>) -> ParseResult<()> {
        self.meter.preflight_active()?;
        let mut writer = MeteredStringWriter {
            value: self,
            failure: None,
        };
        let result = fmt::write(&mut writer, arguments);
        match (result, writer.failure) {
            (_, Some(failure)) => Err(failure),
            (Ok(()), None) => Ok(()),
            (Err(fmt::Error), None) => {
                Err(writer
                    .value
                    .meter
                    .abort(ScopeAbortCause::DescriptorContract {
                        code: "metered-string-format-failed",
                    }))
            }
        }
    }
}

struct MeteredStringWriter<'a, 'scope> {
    value: &'a mut MeteredString<'scope>,
    failure: Option<ParseFailure>,
}

impl fmt::Write for MeteredStringWriter<'_, '_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        match self.value.try_push_str(value) {
            Ok(()) => Ok(()),
            Err(failure) => {
                self.failure = Some(failure);
                Err(fmt::Error)
            }
        }
    }
}

impl MeteredDecodedText<'_> {
    pub fn try_push_str(&mut self, value: &str) -> ParseResult<()> {
        self.meter.preflight_active()?;
        self.validate_append(value.len())?;
        append_metered_string_with_meter(&self.meter, &mut self.value, value)
    }

    /// Appends decoded bytes under a caller-owned bounded work chunk without polling again.
    pub fn try_push_str_reserved(&mut self, value: &str) -> ParseResult<()> {
        self.meter.preflight_active()?;
        self.validate_append(value.len())?;
        if value.len() > METERED_COPY_POLL_INTERVAL {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-copy-chunk-too-large",
            }));
        }
        self.value.push_str(value);
        Ok(())
    }

    fn validate_append(&self, additional: usize) -> ParseResult<()> {
        let required = self
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.meter.allocation_failure(self.resource))?;
        if required > self.decoded_len || required > self.value.capacity() {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-length-exhausted",
            }));
        }
        Ok(())
    }

    pub fn finish_exact(self) -> ParseResult<Self> {
        self.meter.poll()?;
        if self.value.len() != self.decoded_len {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-length-mismatch",
            }));
        }
        Ok(self)
    }
}

struct ImportLiveReservation<'scope> {
    reservation: ImportStoreReservation,
    scope: PhantomData<&'scope ()>,
}

impl ImportLiveReservation<'_> {
    fn into_reservation(self) -> ImportStoreReservation {
        self.reservation
    }
}

struct DependencyResolutionFailure;

struct DependencyFailureObservationContext<'io> {
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    registered_resolver: Option<&'static RegisteredResolverProfile>,
    io: &'io dyn AssetIo,
}

struct MeteredDependencyPath {
    path: AssetPath,
    _reservation: ImportStoreReservation,
}

impl MeteredDependencyPath {
    fn new(path: AssetPath, reservation: ImportStoreReservation) -> Self {
        Self {
            path,
            _reservation: reservation,
        }
    }

    fn path(&self) -> &AssetPath {
        &self.path
    }
}

struct DependencyResolutionContext {
    relation: Option<PendingResourceRelation>,
    identity: Option<DependencyIdentity>,
    evidence: DependencyFailureEvidence,
    cause: DependencyResolutionCause,
    cache_disposition: DependencyCacheDisposition,
}

enum DependencyFailureEvidence {
    Normalized(MeteredDependencyPath),
    Rejected(Option<MeteredDependencyPath>),
}

impl DependencyFailureEvidence {
    fn evidence_path(&self) -> Option<&MeteredDependencyPath> {
        match self {
            Self::Normalized(path) => Some(path),
            Self::Rejected(path) => path.as_ref(),
        }
    }

    fn manifest_reference(&self) -> Option<&AssetPath> {
        match self {
            Self::Normalized(path) => Some(path.path()),
            Self::Rejected(_) => None,
        }
    }
}

enum DependencyResolutionCause {
    Owned(ScopeAbortCause),
    Cached { index: usize },
}

impl DependencyResolutionContext {
    fn cause<'a>(
        &'a self,
        graph: &'a ResourceGraph,
    ) -> Result<&'a ScopeAbortCause, ScopeAbortCause> {
        match &self.cause {
            DependencyResolutionCause::Owned(cause) => Ok(cause),
            DependencyResolutionCause::Cached { index } => graph
                .cached_failure(*index)
                .map(CachedDependencyFailure::cause)
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-failure-index-invalid",
                }),
        }
    }

    fn disposition(
        &self,
        graph: &ResourceGraph,
    ) -> Result<ResourceUnavailableDisposition, ScopeAbortCause> {
        match &self.cause {
            DependencyResolutionCause::Owned(cause) => Ok(unavailable_disposition(cause)),
            DependencyResolutionCause::Cached { index } => graph
                .cached_failure(*index)
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-failure-index-invalid",
                })
                .and_then(cached_unavailable_disposition),
        }
    }

    fn into_parts(
        self,
        graph: &ResourceGraph,
    ) -> (
        ScopeAbortCause,
        Option<PendingResourceRelation>,
        Option<DependencyIdentity>,
        DependencyFailureEvidence,
    ) {
        let Self {
            relation,
            identity,
            evidence,
            cause,
            ..
        } = self;
        let cause = match cause {
            DependencyResolutionCause::Owned(cause) => cause,
            DependencyResolutionCause::Cached { index } => graph
                .cached_failure(index)
                .map(CachedDependencyFailure::replay_scope_cause)
                .unwrap_or(ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-failure-index-invalid",
                }),
        };
        (cause, relation, identity, evidence)
    }

    fn normalized_manifest_reference(&self) -> Option<&AssetPath> {
        self.evidence.manifest_reference()
    }
}

fn optional_eligible(cause: &ScopeAbortCause) -> bool {
    match cause {
        ScopeAbortCause::AssetRead(error) => {
            !matches!(error.kind(), AssetReadErrorKind::ContractViolation(_))
        }
        ScopeAbortCause::Path(_)
        | ScopeAbortCause::AssetOpen(_)
        | ScopeAbortCause::DependencyPolicy(_)
        | ScopeAbortCause::ResourceLimit {
            resource: "asset-bytes" | "dependency-reference-bytes",
            ..
        }
        | ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            dimension:
                ImportDimension::DependencyBytes
                | ImportDimension::TotalAssetBytes
                | ImportDimension::DependencyAcquisitionWork,
            ..
        }) => true,
        _ => false,
    }
}

fn share_dependency_snapshot_with(
    bytes: AccountedVec<u8>,
    reserve: impl FnOnce(u64) -> Result<ImportStoreReservation, ScopeAbortCause>,
) -> Result<Arc<SharedDependencySnapshot>, ScopeAbortCause> {
    let control_bytes =
        shared_dependency_snapshot_allocation_bytes().ok_or(ScopeAbortCause::ResourceLimit {
            resource: "dependency-snapshot-sharing",
            limit: u64::MAX,
        })?;
    let control_bytes =
        u64::try_from(control_bytes).map_err(|_| ScopeAbortCause::ResourceLimit {
            resource: "dependency-snapshot-sharing",
            limit: u64::MAX,
        })?;
    let reservation = reserve(control_bytes)?;
    Ok(Arc::new(SharedDependencySnapshot::new(bytes, reservation)))
}

pub(crate) fn read_opened_asset(
    host: ImportStoreHost<'_>,
    mut opened: OpenedAsset,
    ledger_phase: ImportLedgerPhase,
    progress_phase: ImportProgressPhase,
    dimension: ImportDimension,
    per_resource_limit: u64,
    aggregate_limit: u64,
) -> AssetSnapshotResult {
    host.poll()
        .map_err(|cause| cause.into_scope_abort_cause())?;
    let effective_limit = opened
        .backend_hard_limit()
        .map_or(per_resource_limit, |hard| hard.min(per_resource_limit));
    let observed_length = opened.length();
    let exact_length = match observed_length {
        Some(AssetLength::Exact(length)) => Some(length),
        Some(AssetLength::Hint(_)) | None => None,
    };
    let progress_total = if dimension == ImportDimension::PrimaryBytes {
        exact_length
    } else {
        None
    };
    let mut attempted_precharged = 0_u64;
    if let Some(length) = exact_length {
        charge_asset_attempted(host, dimension, length)?;
        attempted_precharged = length;
        if length > effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
    }

    let mut bytes = AccountedVec::new();
    let aggregate_capacity_limit = if exact_length.is_some() {
        u64::MAX
    } else {
        remaining_attempted_asset_bytes(host, ImportDimension::TotalAssetBytes, aggregate_limit)?
    };
    let capacity_hint = observed_length
        .map(AssetLength::bytes)
        .unwrap_or(0)
        .min(effective_limit)
        .min(aggregate_capacity_limit);
    if capacity_hint != 0 {
        let capacity = usize::try_from(capacity_hint).map_err(|_| ScopeAbortCause::Allocation {
            resource: "asset-snapshot",
        })?;
        bytes.try_reserve_capacity(capacity, "asset-snapshot", |capacity_bytes| {
            host.reserve(ledger_phase, dimension, capacity_bytes)
                .map_err(|cause| cause.into_scope_abort_cause())
        })?;
    }

    let mut chunk = [0_u8; 16 * 1024];
    loop {
        host.poll()
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let requested = if exact_length.is_some() {
            chunk.len()
        } else {
            let resource_remaining = effective_limit.saturating_sub(attempted_precharged);
            if resource_remaining == 0 {
                return Err(ScopeAbortCause::ResourceLimit {
                    resource: "asset-bytes",
                    limit: effective_limit,
                });
            }
            let aggregate_remaining = remaining_attempted_asset_bytes(
                host,
                ImportDimension::TotalAssetBytes,
                aggregate_limit,
            )?;
            if aggregate_remaining == 0 {
                return Err(attempted_asset_limit_cause(
                    host,
                    ImportDimension::TotalAssetBytes,
                    aggregate_limit,
                ));
            }
            let allowance = resource_remaining
                .min(aggregate_remaining)
                .min(chunk.len() as u64);
            let requested =
                if allowance > 1 && allowance == resource_remaining.min(aggregate_remaining) {
                    allowance - 1
                } else {
                    allowance
                };
            charge_asset_attempted(host, dimension, requested)?;
            attempted_precharged = attempted_precharged.saturating_add(requested);
            requested as usize
        };
        let before = opened.bytes_read();
        let read = catch_unwind(AssertUnwindSafe(|| {
            opened.read_chunk(&mut chunk[..requested])
        }));
        let after = opened.bytes_read();
        let observed = after.checked_sub(before).ok_or({
            ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "asset-read-counter-regressed",
            }
        })?;
        if exact_length.is_none() && observed != 0 {
            charge_asset_consumed(host, dimension, observed)?;
        }
        if exact_length.is_some() && after > attempted_precharged {
            charge_asset_attempted(host, dimension, after - attempted_precharged)?;
            attempted_precharged = after;
        }
        if exact_length.is_some() && observed != 0 {
            charge_asset_consumed(host, dimension, observed)?;
        }
        if after > effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
        let count = match read {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "asset-read-panicked",
                });
            }
            Ok(Err(cause)) => return Err(ScopeAbortCause::AssetRead(cause)),
            Ok(Ok(count)) => count,
        };
        if count == 0 {
            break;
        }
        let count_u64 = count as u64;
        if count_u64 != observed {
            return Err(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "asset-read-counter-mismatch",
            });
        }
        if exact_length.is_none() && attempted_precharged == effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
        if exact_length.is_none()
            && remaining_attempted_asset_bytes(
                host,
                ImportDimension::TotalAssetBytes,
                aggregate_limit,
            )? == 0
        {
            return Err(attempted_asset_limit_cause(
                host,
                ImportDimension::TotalAssetBytes,
                aggregate_limit,
            ));
        }
        let next_len = (bytes.len() as u64).saturating_add(count_u64);
        if next_len > effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
        if bytes.capacity().saturating_sub(bytes.len()) < count {
            bytes.try_reserve(count, "asset-snapshot", |capacity_bytes| {
                host.reserve(ledger_phase, dimension, capacity_bytes)
                    .map_err(|cause| cause.into_scope_abort_cause())
            })?;
        }
        bytes
            .try_extend_from_slice_reserved(&chunk[..count])
            .map_err(|()| ScopeAbortCause::DescriptorContract {
                code: "asset-snapshot-slots-not-admitted",
            })?;
        let completed = host
            .dimension_state(dimension)
            .map_err(|cause| cause.into_scope_abort_cause())?
            .consumed();
        host.report_progress(
            progress_phase,
            ImportProgressUnit::Bytes,
            completed,
            progress_total,
        )
        .map_err(|cause| cause.into_scope_abort_cause())?;
    }

    Ok(bytes)
}

fn charge_asset_attempted(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ScopeAbortCause> {
    host.charge_attempted(dimension, amount)
        .and_then(|()| host.charge_attempted(ImportDimension::TotalAssetBytes, amount))
        .map_err(|cause| cause.into_scope_abort_cause())
}

fn remaining_attempted_asset_bytes(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    limit: u64,
) -> Result<u64, ScopeAbortCause> {
    let attempted = host
        .dimension_state(dimension)
        .map_err(|cause| cause.into_scope_abort_cause())?
        .attempted();
    Ok(limit.saturating_sub(attempted))
}

fn attempted_asset_limit_cause(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    limit: u64,
) -> ScopeAbortCause {
    let attempted = host
        .dimension_state(dimension)
        .map_or(limit, |evidence| evidence.attempted());
    ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
        class: CounterClass::Attempted,
        dimension,
        value: attempted.saturating_add(1),
        limit,
    })
}

fn charge_asset_consumed(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ScopeAbortCause> {
    host.charge_consumed_batch(&[
        (dimension, amount),
        (ImportDimension::TotalAssetBytes, amount),
    ])
    .map_err(|cause| cause.into_scope_abort_cause())
}

fn capacity_plan_cause(resource: &'static str, cause: CapacityPlanError) -> ScopeAbortCause {
    match cause {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            }
        }
        CapacityPlanError::InvalidState { .. } | CapacityPlanError::StalePlan { .. } => {
            ScopeAbortCause::DescriptorContract {
                code: "capacity-plan-contract-failed",
            }
        }
        cause @ CapacityPlanError::CapacityEnvelopeViolated { .. } => {
            ScopeAbortCause::CapacityEnvelope { resource, cause }
        }
        _ => ScopeAbortCause::DescriptorContract {
            code: "capacity-plan-contract-failed",
        },
    }
}

fn dependency_normalization_capacity_envelope(
    primary_source: &AssetPath,
    raw_reference: &str,
) -> Result<u64, ScopeAbortCause> {
    let resource = "dependency-path-normalization";
    let base_component_capacity = primary_source
        .as_str()
        .bytes()
        .filter(|byte| *byte == b'/')
        .count()
        .checked_add(1)
        .ok_or(ScopeAbortCause::ResourceLimit {
            resource,
            limit: u64::MAX,
        })?;
    let incoming_component_capacity = raw_reference
        .bytes()
        .filter(|byte| *byte == b'/' || *byte == b'\\')
        .count()
        .checked_add(1)
        .ok_or(ScopeAbortCause::ResourceLimit {
            resource,
            limit: u64::MAX,
        })?;
    let parts_capacity = base_component_capacity
        .saturating_sub(1)
        .checked_add(incoming_component_capacity)
        .ok_or(ScopeAbortCause::ResourceLimit {
            resource,
            limit: u64::MAX,
        })?;
    let base_directory_bytes = primary_source
        .as_str()
        .rsplit_once('/')
        .map_or(0, |(directory, _)| directory.len());
    let normalized_capacity = if base_directory_bytes == 0 {
        raw_reference.len()
    } else {
        base_directory_bytes
            .checked_add(1)
            .and_then(|bytes| bytes.checked_add(raw_reference.len()))
            .ok_or(ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            })?
    }
    .min(AssetPath::MAX_BYTES);
    let plans = [
        CapacityPlan::for_type::<&str>(0, 0, base_component_capacity),
        CapacityPlan::for_type::<u8>(0, 0, raw_reference.len()),
        CapacityPlan::for_type::<&str>(0, 0, parts_capacity),
        CapacityPlan::for_type::<u8>(0, 0, normalized_capacity),
    ];
    plans.into_iter().try_fold(0_u64, |total, plan| {
        let bytes = plan
            .map_err(|cause| capacity_plan_cause(resource, cause))?
            .envelope_delta_bytes()
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        total
            .checked_add(bytes)
            .ok_or(ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            })
    })
}

fn dependency_path_cause(cause: PathError) -> ScopeAbortCause {
    match cause {
        PathError::AllocationFailed => ScopeAbortCause::Allocation {
            resource: "dependency-path-normalization",
        },
        cause => ScopeAbortCause::Path(cause),
    }
}

fn stable_identity_matches(
    expected: Option<&'static StableResourceIdentitySchema>,
    cached: Option<&'static StableResourceIdentitySchema>,
) -> bool {
    match (expected, cached) {
        (Some(expected), Some(cached)) => {
            expected.owner() == cached.owner()
                && expected.schema_id() == cached.schema_id()
                && expected.schema_version() == cached.schema_version()
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::ImportOperationControl;
    use crate::store::{
        ImportStoreBootstrapRequirements, ImportStoreHostError, ImportStoreSessionOwner,
        try_import_store_bootstrap,
    };
    use baozi_io::AssetPath;
    use baozi_io::{AssetLength, AuthorityDomain};
    use baozi_operation::{
        PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits,
    };
    use std::io::Cursor;

    struct TestStore {
        owner: ImportStoreSessionOwner,
        _account: PhysicalStorageAccount,
    }

    impl TestStore {
        fn host(&self) -> ImportStoreHost<'_> {
            self.owner.host()
        }
    }

    fn test_store(limits: &ResourceLimits) -> TestStore {
        let requirements = ImportStoreBootstrapRequirements::for_node_scene(limits)
            .expect("test retained-store requirements are valid");
        let bootstrap = try_import_store_bootstrap(limits, requirements)
            .expect("test import store bootstrap must be valid");
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
                .expect("test factory limits are valid"),
        )
        .expect("test account factory must allocate");
        let account = factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX)
                    .expect("test account limits are valid"),
            )
            .expect("test physical account must allocate");
        let owner = ImportStoreSessionOwner::try_start(
            ImportOperationControl::default(),
            &account,
            bootstrap,
        )
        .expect("test import store owner must start");
        TestStore {
            owner,
            _account: account,
        }
    }

    fn current_live_bytes(host: ImportStoreHost<'_>) -> u64 {
        ImportDimension::ALL
            .into_iter()
            .map(|dimension| {
                host.dimension_state(dimension)
                    .expect("test import dimension must remain observable")
                    .live_bytes()
            })
            .try_fold(0_u64, u64::checked_add)
            .expect("test live-byte total fits u64")
    }

    #[test]
    fn completed_dependency_bytes_drop_when_snapshot_sharing_reservation_fails() {
        let store = test_store(&ResourceLimits::default());
        let host = store.host();
        let baseline = current_live_bytes(host);
        let payload = vec![0x5a; 64 * 1024];
        let mut bytes = AccountedVec::new();
        bytes
            .try_reserve_capacity(payload.len(), "dependency-snapshot", |capacity_bytes| {
                host.reserve(
                    ImportLedgerPhase::DependencyAcquisition,
                    ImportDimension::DependencyBytes,
                    capacity_bytes,
                )
                .map_err(ImportStoreHostError::into_scope_abort_cause)
            })
            .expect("the completed dependency buffer must be admitted");
        bytes
            .try_extend_from_slice_reserved(&payload)
            .expect("the admitted dependency buffer must hold the complete payload");
        assert_eq!(bytes.len(), payload.len());
        assert!(current_live_bytes(host) > baseline);

        let requested_control_bytes = Cell::new(0_u64);
        let result = share_dependency_snapshot_with(bytes, |control_bytes| {
            requested_control_bytes.set(control_bytes);
            Err(ScopeAbortCause::Allocation {
                resource: "injected-dependency-snapshot-sharing",
            })
        });

        assert!(matches!(
            result,
            Err(ScopeAbortCause::Allocation {
                resource: "injected-dependency-snapshot-sharing"
            })
        ));
        assert!(requested_control_bytes.get() > 0);
        assert_eq!(current_live_bytes(host), baseline);
    }

    fn assert_not_send_sync<T: ?Sized>() {
        let _ = std::any::type_name::<T>();
    }

    #[test]
    fn parse_scope_is_intentionally_session_local() {
        assert_not_send_sync::<ParseScope<'_>>();
    }

    #[test]
    fn optional_eligibility_is_limited_to_dependency_input_failures() {
        assert!(optional_eligible(&ScopeAbortCause::ResourceLimit {
            resource: "asset-bytes",
            limit: 1,
        }));
        assert!(optional_eligible(&ScopeAbortCause::ResourceLimit {
            resource: "dependency-reference-bytes",
            limit: 1,
        }));
        assert!(!optional_eligible(&ScopeAbortCause::ResourceLimit {
            resource: "resource-graph-nodes",
            limit: 1,
        }));
        assert!(!optional_eligible(&ScopeAbortCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "panic",
        }));
        assert!(!optional_eligible(&ScopeAbortCause::AssetRead(
            baozi_io::AssetReadError::contract_violation(
                baozi_io::AssetReadContractViolation::CountExceedsBuffer {
                    returned: 2,
                    capacity: 1,
                },
            ),
        )));
    }

    #[test]
    fn opened_asset_snapshot_is_unique_metered_vec_storage() {
        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let payload = b"metered snapshot".to_vec();
        let opened = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("scene.bin").unwrap(),
            Cursor::new(payload.clone()),
            Some(AssetLength::Exact(payload.len() as u64)),
        );

        let snapshot = read_opened_asset(
            host,
            opened,
            ImportLedgerPhase::PrimarySnapshot,
            ImportProgressPhase::AcquirePrimary,
            ImportDimension::PrimaryBytes,
            limits.max_primary_asset_bytes,
            limits.max_total_asset_bytes,
        )
        .unwrap();

        assert_eq!(&*snapshot, payload.as_slice());
        assert!(snapshot.capacity_bytes() >= payload.len() as u64);
        assert!(current_live_bytes(host) > 0);

        drop(snapshot);
        assert_eq!(current_live_bytes(host), 0);
    }

    struct OwnedIterationDropProbe<'a> {
        id: u8,
        host: ImportStoreHost<'a>,
        observations: &'a RefCell<Vec<(u8, bool)>>,
    }

    impl Drop for OwnedIterationDropProbe<'_> {
        fn drop(&mut self) {
            let capacity_is_live = self
                .host
                .dimension_state(ImportDimension::CanonicalOutputBytes)
                .expect("canonical-output dimension")
                .live_bytes()
                > 0;
            self.observations
                .borrow_mut()
                .push((self.id, capacity_is_live));
        }
    }

    fn assert_owned_iteration_drop_observations(
        observations: &RefCell<Vec<(u8, bool)>>,
        host: ImportStoreHost<'_>,
    ) {
        let mut observations = observations.borrow().clone();
        observations.sort_unstable_by_key(|(id, _)| *id);
        assert_eq!(
            observations,
            vec![(0, true), (1, true), (2, true)],
            "the current and remaining elements must all drop under the live capacity reservation"
        );
        assert_eq!(current_live_bytes(host), 0);
    }

    #[test]
    fn owned_iteration_error_drops_current_and_remaining_elements_before_capacity() {
        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host,
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let observations = RefCell::new(Vec::new());
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::SceneConstruction,
            resource: "owned-iteration-error",
        };
        for id in 0..3 {
            values
                .try_push(OwnedIterationDropProbe {
                    id,
                    host,
                    observations: &observations,
                })
                .expect("owned-iteration fixture capacity");
        }

        let result = values.try_for_each_owned(|probe| {
            assert_eq!(probe.id, 0, "the first visitor call must inject failure");
            Err::<(), _>("injected visitor failure")
        });

        assert_eq!(result, Err("injected visitor failure"));
        assert_owned_iteration_drop_observations(&observations, host);
    }

    #[test]
    fn owned_iteration_panic_drops_current_and_remaining_elements_before_capacity() {
        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host,
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let observations = RefCell::new(Vec::new());
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::SceneConstruction,
            resource: "owned-iteration-panic",
        };
        for id in 0..3 {
            values
                .try_push(OwnedIterationDropProbe {
                    id,
                    host,
                    observations: &observations,
                })
                .expect("owned-iteration fixture capacity");
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = values.try_for_each_owned(|probe| -> Result<(), ()> {
                assert_eq!(probe.id, 0, "the first visitor call must inject panic");
                panic!("injected owned-iteration visitor panic");
            });
        }));

        assert!(result.is_err(), "the visitor panic must be observed");
        assert_owned_iteration_drop_observations(&observations, host);
    }

    #[test]
    fn reserved_vector_copy_rejects_an_unbounded_chunk_without_mutation() {
        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host,
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::FormatNormalization,
            resource: "reserved-vector-copy-bound",
        };
        let source = vec![0_u8; METERED_COPY_POLL_INTERVAL + 1];
        values
            .try_reserve(source.len())
            .expect("copy fixture capacity must be admitted");

        assert!(values.try_extend_from_slice_reserved(&source).is_err());
        assert!(values.is_empty(), "a rejected copy must be atomic");
        assert!(matches!(
            terminal.borrow().as_ref(),
            Some(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-copy-chunk-too-large",
            })
        ));
    }

    #[test]
    fn reserved_string_copy_rejects_an_unbounded_chunk_without_mutation() {
        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host,
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let mut value = MeteredString {
            value: String::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::FormatNormalization,
            resource: "reserved-string-copy-bound",
        };
        let source = "x".repeat(METERED_COPY_POLL_INTERVAL + 1);
        value
            .try_reserve(source.len())
            .expect("copy fixture capacity must be admitted");

        assert!(value.try_push_str_reserved(&source).is_err());
        assert!(value.is_empty(), "a rejected copy must be atomic");
        assert!(matches!(
            terminal.borrow().as_ref(),
            Some(ScopeAbortCause::DescriptorContract {
                code: "metered-string-copy-chunk-too-large",
            })
        ));
    }

    #[test]
    fn dedup_partition_defers_duplicate_drops_to_the_caller() {
        struct DedupDropProbe<'a> {
            key: u8,
            drops: &'a std::cell::Cell<usize>,
        }

        impl Drop for DedupDropProbe<'_> {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }

        let limits = ResourceLimits::default();
        let store = test_store(&limits);
        let host = store.host();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host,
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let drops = std::cell::Cell::new(0usize);
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::FormatNormalization,
            resource: "dedup-partition",
        };
        for key in [1, 1, 2, 2] {
            values
                .try_push(DedupDropProbe { key, drops: &drops })
                .expect("dedup fixture capacity");
        }

        let retained = values
            .try_partition_dedup_by(|left, right| Ok::<_, ()>(left.key == right.key))
            .expect("dedup comparison");
        assert_eq!(retained, 2);
        assert_eq!(values.len(), 4);
        assert_eq!(drops.get(), 0, "partitioning must not run destructors");

        values.truncate(retained);
        assert_eq!(drops.get(), 2);
        drop(values);
        assert_eq!(drops.get(), 4);
    }
}
