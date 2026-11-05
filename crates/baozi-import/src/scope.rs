use crate::asset_open::OpenedAssetOwner;
use crate::authoring::SceneAuthoringBatch;
use crate::config::{ConformancePolicy, ImportLossKind, LossPolicy};
use crate::dependency::{
    AuthorizedDependency, CachedDependencyFailure, DependencyCacheDisposition,
    DependencyFailureContext, DependencyIdentity, DependencyOrigin, DependencyOriginIssuer,
    DependencyOriginNode, DependencyOutcome, DependencyReference, DependencyRequirement,
    DependencyRoleId, DependencySnapshot, DependencySnapshotIdentity, OptionalDependencyEvidence,
    PendingNegativeObservation, ResolverBinding, SharedDependencySnapshot,
    shared_dependency_snapshot_allocation_bytes,
};
use crate::descriptor::TextDecodingMode;
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCause, DiagnosticCode, DiagnosticDetailSink,
    DiagnosticDropReason, DiagnosticLineColumn, DiagnosticPrepareCause, DiagnosticSeverity,
    DiagnosticSink, DiagnosticSource, PreparedDiagnostic, TextDiagnosticLocation,
    valid_diagnostic_code,
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
    MeteredVec, StorageIdentity, metered_replacement_plan_with_element_size,
    metered_vec_replacement_plan,
};
use crate::registry::RegistryEntry;
use crate::resource_graph::{
    AvailableResourceIdentity, PendingResourceRelation, ResourceAcquisitionState, ResourceGraph,
    ResourceGraphError, ResourceUnavailableDisposition, cached_unavailable_disposition,
    registered_negative_proof_canonical_len, unavailable_disposition,
};
use crate::source_scope::{
    FinalizedSourceScopeRecord, LiveSourceScopeRecord, SourceScopeEntityKind,
    SourceScopeRecordBuilder, SourceScopeSelectionOutcome, SourceScopeSelectionRequest,
};
use crate::store::{ImportStoreReservation, ImportStoreSessionOwner, scene_validation_limits};
use crate::text_input::{
    ConformedTextInput, TextConformanceResult, TextInputOptions, TextInputRecoveryTracker,
};
use baozi_core::__scene_integration::integration::SceneConstruction;
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

mod parser_execution;
mod storage;
pub(crate) use parser_execution::ParseReadRow;
pub use parser_execution::{
    ParseChildren, ParseErrorView, ParseParser, ParsePieceError, ParseRoot, ParseScanReturn,
    ParseWorkLayout,
};
pub use storage::ParseStorage;

pub type ParseResult<T> = Result<T, ParseFailure>;
type AssetSnapshotResult = Result<AccountedVec<u8>, ScopeAbortCause>;

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

/// Opaque parser transport. Only a selected [`ParseContext`] can create it.
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

pub(crate) fn latch_abort(
    terminal: &RefCell<Option<ScopeAbortCause>>,
    cause: ScopeAbortCause,
) -> ParseFailure {
    let mut slot = terminal.borrow_mut();
    if slot.is_none() {
        *slot = Some(cause);
    }
    ParseFailure::scope()
}

fn preflight_terminal(terminal: &RefCell<Option<ScopeAbortCause>>) -> ParseResult<()> {
    if terminal.borrow().is_some() {
        Err(ParseFailure::scope())
    } else {
        Ok(())
    }
}

/// Single-capture status plus detail storage not yet transferred into its actual error owner.
#[derive(Default)]
pub(crate) struct FailureEvidenceStorage {
    pub(crate) reservation: Option<ImportStoreReservation>,
    pub(crate) format_captured: bool,
}

impl FailureEvidenceStorage {
    pub(crate) fn is_active(&self) -> bool {
        self.reservation.is_some() || self.format_captured
    }
}

/// Exclusive outer parse authority. Scene submission occurs outside execution regions.
pub struct ParseContext<'session, 'inv: 'session> {
    entry: RegistryEntry,
    primary_io: &'session dyn AssetIo,
    primary_authority: &'session AuthorityDomain,
    primary_source: &'session AssetPath,
    primary_bytes: &'session [u8],
    resolver: ResolverBinding<'session>,
    dependency_origin_issuer: &'session DependencyOriginIssuer,
    conformance_policy: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: &'session ResourceLimits,
    diagnostics: &'session RefCell<DiagnosticSink>,
    resource_graph: &'session RefCell<ResourceGraph>,
    optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    format_failure_storage: &'session RefCell<FailureEvidenceStorage>,
    dependency_failure: RefCell<Option<DependencyResolutionContext>>,
    dependency_failure_context: RefCell<Option<DependencyFailureContext>>,
    callback_active: Cell<bool>,
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    store_owner: &'session mut ImportStoreSessionOwner<'inv>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

/// Common policy, evidence and storage services borrowed from one numeric invocation.
/// Retained evidence uses that invocation's real work owner; ordinary storage remains separate.
pub struct ParseServices<'borrow, 'op, 'session> {
    store: crate::store::ImportStoreServices<'borrow, 'op>,
    views: &'borrow parser_execution::ParseViews<'borrow, 'session>,
}

pub(crate) struct ParseContextParts<'session, 'inv: 'session> {
    pub entry: RegistryEntry,
    pub primary_io: &'session dyn AssetIo,
    pub primary_authority: &'session AuthorityDomain,
    pub primary_source: &'session AssetPath,
    pub primary_bytes: &'session [u8],
    pub resolver: ResolverBinding<'session>,
    pub dependency_origin_issuer: &'session DependencyOriginIssuer,
    pub conformance_policy: ConformancePolicy,
    pub loss_policy: LossPolicy,
    pub limits: &'session ResourceLimits,
    pub store_owner: &'session mut ImportStoreSessionOwner<'inv>,
    pub diagnostics: &'session RefCell<DiagnosticSink>,
    pub resource_graph: &'session RefCell<ResourceGraph>,
    pub optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    pub loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    pub source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    pub format_failure_storage: &'session RefCell<FailureEvidenceStorage>,
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

impl<'session, 'inv: 'session> ParseContext<'session, 'inv> {
    pub(crate) fn new(parts: ParseContextParts<'session, 'inv>) -> Self {
        Self {
            entry: parts.entry,
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
            diagnostics: parts.diagnostics,
            resource_graph: parts.resource_graph,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            source_scope_record: parts.source_scope_record,
            format_failure_storage: parts.format_failure_storage,
            dependency_failure: RefCell::new(None),
            dependency_failure_context: RefCell::new(None),
            callback_active: Cell::new(false),
            terminal: parts.terminal,
            not_send_or_sync: PhantomData,
        }
    }

    pub fn source(&self) -> &'session AssetPath {
        self.primary_source
    }

    pub const fn conformance_policy(&self) -> ConformancePolicy {
        self.conformance_policy
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.loss_policy
    }

    pub fn primary_bytes(&self) -> &'session [u8] {
        self.primary_bytes
    }

    pub fn diagnostic_source(&self) -> DiagnosticSource<'session> {
        DiagnosticSource::new(
            self.primary_authority.clone(),
            self.primary_source,
            self.primary_bytes,
        )
    }

    pub fn limits(&self) -> &'session ResourceLimits {
        self.limits
    }

    pub fn root_origin(&self) -> DependencyOrigin<'session> {
        DependencyOrigin::primary(self.dependency_origin_issuer, self.primary_io)
    }

    pub(crate) fn begin_importer_boundary(&self) -> bool {
        self.store_owner.begin_importer_boundary()
    }
    pub(crate) fn finish_importer_boundary(
        &self,
        outcome: Option<crate::registry::ParseInvocationError>,
    ) {
        self.store_owner.finish_importer_boundary(outcome);
    }
    pub(crate) fn importer_reentered(&self) -> ParseFailure {
        latch_abort(
            self.terminal,
            ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            },
        )
    }
    pub(crate) fn check_execution_running(&self) -> ParseResult<()> {
        self.store_owner
            .check_execution_running()
            .map_err(|cause| latch_abort(self.terminal, cause.into_scope_abort_cause()))
    }

    /// The result cannot borrow the execution region, while session-owned storage may leave it.
    pub fn with_execution<R, E: From<ParseFailure>>(
        &mut self,
        action: impl for<'op> FnOnce(&mut ParseRoot<'_, 'op, 'session>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.with_parser_execution(|root| {
            // Preserve the original ordinary entry's checkpoint after owner entry.
            root.poll().map_err(E::from)?;
            action(root)
        })
    }

    pub fn submit(&mut self, input: crate::authoring::StaticSceneBuilder<'_>) -> ParseResult<()> {
        preflight_terminal(self.terminal)?;
        self.check_execution_running()?;
        let content_digest = self
            .resource_graph
            .try_borrow()
            .map_err(|_| {
                latch_abort(
                    self.terminal,
                    ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    },
                )
            })?
            .primary_content_digest()
            .map_err(|cause| latch_abort(self.terminal, cause.into_scope_cause()))?;
        let format = self.entry.descriptor().id().clone();
        crate::authoring::static_scene::submit(self, input, &format, content_digest)
    }

    pub fn submit_complete(&mut self, input: SceneAuthoringBatch<'_>) -> ParseResult<()> {
        preflight_terminal(self.terminal)?;
        self.check_execution_running()?;
        self.with_execution(|exec| {
            exec.resource_graph
                .try_borrow_mut()
                .map_err(|_| exec.reentrant_capability_failure())?
                .seal(&mut exec.store)
                .map_err(|cause| exec.abort(cause.into_scope_cause()))
        })?;
        let terminal = self.terminal;
        let construction = SceneConstruction::try_new(input, scene_validation_limits(self.limits))
            .map_err(|cause| {
                latch_abort(
                    terminal,
                    match cause.scene_validation_error() {
                        Some(detail) => ScopeAbortCause::SceneValidation(detail),
                        None => ScopeAbortCause::DescriptorContract {
                            code: "canonical-scene-request-rejected",
                        },
                    },
                )
            })?;
        self.check_execution_running()?;
        self.store_owner
            .submit_scene(construction)
            .map_err(|cause| {
                let code = match cause {
                    crate::store::ImportStoreActivationError::StoreUnavailable(_) => {
                        "canonical-scene-store-unavailable"
                    }
                    crate::store::ImportStoreActivationError::Control(cause) => {
                        return latch_abort(terminal, ScopeAbortCause::Control(cause));
                    }
                    crate::store::ImportStoreActivationError::SceneValidation(detail) => {
                        return latch_abort(terminal, ScopeAbortCause::SceneValidation(detail));
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
                latch_abort(terminal, ScopeAbortCause::DescriptorContract { code })
            })
    }

    pub(crate) fn take_terminal_abort(&self) -> Option<ScopeAbort> {
        self.terminal.borrow_mut().take().map(|cause| {
            match self.dependency_failure_context.borrow_mut().take() {
                Some(context) => ScopeAbort::with_dependency_context(cause, context),
                None => ScopeAbort::new(cause),
            }
        })
    }
}

impl<'op, 'session> ParseServices<'_, 'op, 'session> {
    pub fn source(&self) -> &'session AssetPath {
        self.views.source
    }

    pub const fn conformance_policy(&self) -> ConformancePolicy {
        self.views.conformance_policy
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.views.loss_policy
    }

    pub fn primary_bytes(&self) -> &'session [u8] {
        self.views.bytes
    }

    pub fn diagnostic_source(&self) -> DiagnosticSource<'session> {
        DiagnosticSource::new(
            self.views.primary_authority.clone(),
            self.views.source,
            self.views.bytes,
        )
    }

    pub fn limits(&self) -> &'session ResourceLimits {
        self.views.limits
    }

    pub fn root_origin(&self) -> DependencyOrigin<'session> {
        DependencyOrigin::primary(self.views.issuer, self.views.primary_io)
    }
}

impl<'op, 'session> ParseRoot<'_, 'op, 'session> {
    fn poll_active(&mut self) -> ParseResult<()> {
        self.poll()
    }

    fn reentrant_capability_failure(&self) -> ParseFailure {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    }

    fn capacity_plan_error(
        &mut self,
        resource: &'static str,
        cause: CapacityPlanError,
    ) -> ParseFailure {
        match self.services() {
            Ok(services) => services.capacity_plan_error(resource, cause),
            Err(cause) => cause,
        }
    }

    pub fn with_conformed_text_input<'source, T>(
        &mut self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
        action: impl for<'input> FnOnce(
            &mut ParseChildren<'_, 'op, 'session>,
            &'input ConformedTextInput<'session>,
            &mut TextInputRecoveryTracker<'input, 'session>,
        ) -> ParseResult<T>,
    ) -> ParseResult<TextConformanceResult<T>> {
        self.children()?
            .with_conformed_text_input(source, options, action)
    }

    pub fn text_input_record_cursor<'source>(
        &mut self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
    ) -> ParseResult<
        TextConformanceResult<crate::text_input::TextInputRecordCursor<'session, 'source>>,
    > {
        self.children()?.text_input_record_cursor(source, options)
    }
}

impl<'borrow, 'op, 'session> ParseChildren<'borrow, 'op, 'session> {
    pub fn with_conformed_text_input<'source, T>(
        &mut self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
        action: impl for<'input> FnOnce(
            &mut ParseChildren<'borrow, 'op, 'session>,
            &'input ConformedTextInput<'session>,
            &mut TextInputRecoveryTracker<'input, 'session>,
        ) -> ParseResult<T>,
    ) -> ParseResult<TextConformanceResult<T>> {
        let entry = self.views.entry;
        crate::text_input::with_conformed_text_input(
            self,
            entry.descriptor().text_profile(),
            source,
            options,
            action,
        )
    }

    pub fn text_input_record_cursor<'source>(
        &mut self,
        source: DiagnosticSource<'source>,
        options: TextInputOptions,
    ) -> ParseResult<
        TextConformanceResult<crate::text_input::TextInputRecordCursor<'session, 'source>>,
    > {
        let entry = self.views.entry;
        crate::text_input::TextInputRecordCursor::new(
            self,
            entry.descriptor().text_profile(),
            source,
            options,
        )
    }
}

impl<'op, 'session> ParseServices<'_, 'op, 'session> {
    pub fn storage(&mut self) -> ParseResult<ParseStorage<'_, 'op, 'session>> {
        self.preflight_active()?;
        let identity = self.storage_identity();
        let raw = self.store.ordinary();
        Ok(ParseStorage::new(raw, identity, self.views.callback_active))
    }

    pub(crate) fn session_issuer(&self) -> &'session DependencyOriginIssuer {
        self.views.issuer
    }

    pub(crate) fn check_session(&self, issuer: &DependencyOriginIssuer) -> ParseResult<()> {
        self.preflight_active()?;
        if std::ptr::eq(self.views.issuer, issuer) {
            Ok(())
        } else {
            Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-storage-foreign-session",
            }))
        }
    }

    /// Starts one incremental source-selection and closure evidence record.
    ///
    /// The outcome must identify this document. An explicit request must resolve
    /// to the same declared source scene before any evidence storage is adopted.
    pub fn source_scope_builder(
        &mut self,
        selection_request: SourceScopeSelectionRequest,
        selection_outcome: SourceScopeSelectionOutcome,
    ) -> ParseResult<SourceScopeRecordBuilder<'session>> {
        self.preflight_active()?;
        if selection_outcome.document().asset_path() != self.source() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-selection-document-mismatch",
            }));
        }
        if let SourceScopeSelectionRequest::ExplicitScene { scene_index } = selection_request
            && selection_outcome.scene_index() != Some(scene_index)
        {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-selection-request-mismatch",
            }));
        }
        SourceScopeRecordBuilder::new(self, selection_request, selection_outcome)
    }

    /// Creates vector storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_vec<T>(
        &mut self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredVec<'session, T> {
        MeteredVec {
            values: Vec::new(),
            capacity: None,
            identity: self.storage_identity(),
            class,
            resource,
        }
    }

    /// Creates string storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_string(
        &mut self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredString<'session> {
        MeteredString {
            value: String::new(),
            capacity: None,
            identity: self.storage_identity(),
            class,
            resource,
        }
    }

    /// Creates ordered-map storage whose backing allocation cannot outlive this parse capability.
    pub fn metered_ordered_map<K, V>(
        &mut self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredOrderedMap<'session, K, V> {
        MeteredOrderedMap {
            value: OrderedMap::new(),
            capacity: None,
            identity: self.storage_identity(),
            class,
            resource,
        }
    }

    /// Reserves an opaque backend allowance that cannot be transferred into a Scene value.
    pub fn metered_opaque_allowance(
        &mut self,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<MeteredOpaqueAllowance<'session>> {
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
        &mut self,
        decoded_len: usize,
        resource: &'static str,
    ) -> ParseResult<MeteredDecodedText<'session>> {
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
            identity: self.storage_identity(),
            resource,
        })
    }

    fn storage_identity(&self) -> StorageIdentity<'session> {
        StorageIdentity {
            issuer: self.views.issuer,
            terminal: self.views.terminal,
        }
    }
}

impl<'op, 'session> ParseServices<'_, 'op, 'session> {
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
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    pub(crate) fn reserve_storage_cause(
        &mut self,
        storage: ImportStorageClass,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        let (phase, dimension) = storage.accounting();
        self.store
            .reserve(phase, dimension, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    pub(crate) fn reserve_retained_storage_cause(
        &mut self,
        storage: ImportStorageClass,
        bytes: u64,
        allocations: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        let (phase, dimension) = storage.accounting();
        self.store
            .reserve_retained(phase, dimension, bytes, allocations)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resize_storage_reservation(
        &self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ScopeAbortCause> {
        self.store
            .resize(reservation, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
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
        &mut self,
        code: DiagnosticCode,
        source: DiagnosticSource<'_>,
        raw_range: DiagnosticByteRange,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.record_format_recovery_with_location(code, source, raw_range, None, message)
    }

    pub(crate) fn record_format_recovery_with_location(
        &mut self,
        code: DiagnosticCode,
        source: DiagnosticSource<'_>,
        raw_range: DiagnosticByteRange,
        line_column: Option<DiagnosticLineColumn>,
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
            .store
            .charge_attempted(ImportDimension::RecoveryCount, 1)
            .and_then(|()| {
                self.store
                    .charge_consumed(ImportDimension::RecoveryCount, 1)
            })
            .and_then(|()| {
                self.store
                    .charge_attempted(ImportDimension::RecoveryRawBytes, raw_bytes)
            })
            .and_then(|()| {
                self.store
                    .charge_consumed(ImportDimension::RecoveryRawBytes, raw_bytes)
            });
        if let Err(cause) = charge {
            return Err(self.abort(cause.into_scope_abort_cause()));
        }

        let descriptor_allows_recovery =
            self.views
                .entry
                .descriptor()
                .text_profile()
                .is_some_and(|profile| {
                    profile.allows_decoding_mode(TextDecodingMode::ReplaceInvalidUtf8)
                });
        let disposition = if !descriptor_allows_recovery
            || self.views.conformance_policy != ConformancePolicy::AllowDeclaredRecovery
        {
            ImportLossDisposition::RejectedByConformance
        } else if !self
            .views
            .loss_policy
            .allows(ImportLossKind::FormatRecovery)
        {
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
        self.emit_diagnostic_with_location(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Normalize,
            source,
            Some(raw_range),
            line_column,
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

    /// Records one format-identified source semantic omission under the caller's frozen loss policy.
    ///
    /// The source entity identity belongs in `object_path`; the typed subject keeps loss evidence
    /// queryable even when the diagnostic message is truncated.
    #[allow(clippy::too_many_arguments)]
    pub fn record_source_semantic_loss(
        &mut self,
        code: DiagnosticCode,
        subject: SourceScopeEntityKind,
        object_path: &str,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let disposition = if self
            .views
            .loss_policy
            .allows(ImportLossKind::SourceSemanticLoss)
        {
            ImportLossDisposition::Applied
        } else {
            ImportLossDisposition::RejectedByLossPolicy
        };
        let source = self.diagnostic_source();
        let evidence = self.prepare_loss_evidence(
            ImportLossKind::SourceSemanticLoss,
            ImportLossSubject::SourceEntity(subject),
            code,
            &source,
            None,
            disposition,
        )?;
        self.commit_loss_evidence(evidence)?;
        self.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Normalize,
            source,
            None,
            Some(object_path),
            details,
            message,
        )?;

        match disposition {
            ImportLossDisposition::Applied => Ok(()),
            ImportLossDisposition::RejectedByLossPolicy => {
                Err(self.abort(ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }))
            }
            ImportLossDisposition::RejectedByConformance => {
                Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "source-semantic-loss-invalid-disposition",
                }))
            }
        }
    }

    /// Submits one finalized source-scope record to this parse session's sealed slot.
    pub fn record_source_scope(
        &mut self,
        finalized: FinalizedSourceScopeRecord<'session>,
    ) -> ParseResult<()> {
        let diagnostics = self.views.diagnostics;
        let loss_evidence = self.views.loss_evidence;
        let source_scope_record = self.views.source_scope_record;

        self.preflight_active()?;
        if !finalized.is_issued_by(self.views.issuer) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "source-scope-record-owner-mismatch",
            }));
        }
        let mut source_scope_slot = source_scope_record
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
                source: self.views.source,
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
                    self.views.limits.max_diagnostics,
                ),
                DiagnosticDropReason::StoredByteLimit => (
                    "source-scope-collapse-diagnostic-bytes",
                    self.views.limits.max_diagnostic_bytes,
                ),
            };
            return Err(self.abort(ScopeAbortCause::ResourceLimit { resource, limit }));
        }

        let mut loss_evidence = match loss_evidence.try_borrow_mut() {
            Ok(evidence) => evidence,
            Err(_) => {
                self.discard_diagnostic_event(diagnostic)?;
                drop(loss);
                return Err(self.reentrant_capability_failure());
            }
        };
        let mut diagnostics = match diagnostics.try_borrow_mut() {
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
        &mut self,
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
        &mut self,
        kind: ImportLossKind,
        subject: ImportLossSubject,
        code: DiagnosticCode,
        source: &DiagnosticSource<'_>,
        raw_range: Option<DiagnosticByteRange>,
        disposition: ImportLossDisposition,
    ) -> ParseResult<PreparedLossEvidence> {
        let loss_evidence = self.views.loss_evidence;

        self.store
            .charge_attempted(ImportDimension::LossEvidenceCount, 1)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
        let source_plan = CapacityPlan::for_type::<u8>(0, 0, source.path().as_str().len())
            .map_err(|cause| self.capacity_plan_error("loss-evidence-source", cause))?;
        let source_envelope = source_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error("loss-evidence-source", cause))?;
        let mut reservation = self
            .store
            .reserve_retained(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::LossEvidenceBytes,
                source_envelope,
                u64::from(source_envelope != 0),
            )
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;

        let retained_source = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("loss-evidence-source"))?;
        let source_bytes = u64::try_from(retained_source.capacity_bytes())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        source_plan
            .finish(0, retained_source.capacity_bytes())
            .map_err(|cause| self.capacity_plan_error("loss-evidence-source", cause))?;
        if source_bytes != source_envelope {
            self.store
                .resize(&mut reservation, source_bytes)
                .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?;
        }

        let (prior_capacity_bytes, capacity_result) = {
            let mut evidence = loss_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let prior_capacity_bytes = evidence.capacity_bytes();
            let result = evidence.try_reserve(1, "loss-evidence-capacity", |capacity_bytes| {
                self.store
                    .reserve_retained(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::LossEvidenceBytes,
                        capacity_bytes,
                        u64::from(capacity_bytes != 0),
                    )
                    .map_err(|cause| cause.into_scope_abort_cause())
            });
            (prior_capacity_bytes, result)
        };
        capacity_result.map_err(|cause| self.abort(cause))?;
        let capacity_bytes = loss_evidence
            .try_borrow()
            .map_err(|_| self.reentrant_capability_failure())?
            .capacity_bytes()
            .saturating_sub(prior_capacity_bytes);
        let actual_bytes = source_bytes
            .checked_add(capacity_bytes)
            .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-bytes", u64::MAX))?;
        let charge = self
            .store
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

    fn commit_loss_evidence(&mut self, prepared: PreparedLossEvidence) -> ParseResult<()> {
        let loss_evidence = self.views.loss_evidence;

        let mut loss_evidence = loss_evidence
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

    fn charge_loss_evidence_consumed(&mut self, stored_bytes: u64) -> ParseResult<()> {
        self.store
            .charge_consumed_batch(&[
                (ImportDimension::LossEvidenceCount, 1),
                (ImportDimension::LossEvidenceBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    pub fn emit_diagnostic(
        &mut self,
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
                source: self.views.source,
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
        &mut self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        source: DiagnosticSource<'_>,
        byte_range: Option<DiagnosticByteRange>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.emit_diagnostic_with_location(
            severity,
            code,
            phase,
            source,
            byte_range,
            None,
            object_path,
            details,
            message,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_text_diagnostic_with(
        &mut self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        location: TextDiagnosticLocation<'_>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        if !location.is_primary_source() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "text-diagnostic-location-not-primary",
            }));
        }
        self.emit_diagnostic_with_location(
            severity,
            code,
            phase,
            self.diagnostic_source(),
            Some(location.byte_range()),
            Some(location.line_column()),
            object_path,
            details,
            message,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_diagnostic_with_location(
        &mut self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        source: DiagnosticSource<'_>,
        byte_range: Option<DiagnosticByteRange>,
        line_column: Option<DiagnosticLineColumn>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.validate_diagnostic_location(&source, byte_range, line_column)?;
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

    fn validate_diagnostic_location(
        &self,
        source: &DiagnosticSource<'_>,
        byte_range: Option<DiagnosticByteRange>,
        line_column: Option<DiagnosticLineColumn>,
    ) -> ParseResult<()> {
        let Some(byte_range) = byte_range else {
            if line_column.is_some() {
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "diagnostic-line-column-without-byte-range",
                }));
            }
            return Ok(());
        };
        let source_len = u64::try_from(source.bytes().len())
            .map_err(|_| self.resource_limit_exceeded("diagnostic-source-bytes", u64::MAX))?;
        if byte_range.end() <= source_len {
            return Ok(());
        }
        Err(self.abort(ScopeAbortCause::Diagnostic(
            DiagnosticCause::ByteRangeOutOfBounds {
                end: byte_range.end(),
                source_len,
            },
        )))
    }

    fn emit_diagnostic_event(
        &mut self,
        metadata: DiagnosticEventMetadata<'_>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        let prepared = self.prepare_diagnostic_event(metadata, details, message)?;
        self.commit_diagnostic_event(prepared)
    }

    fn prepare_diagnostic_event(
        &mut self,
        metadata: DiagnosticEventMetadata<'_>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<PreparedDiagnosticEvent> {
        let diagnostics = self.views.diagnostics;

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
        let admission = diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .begin(code, source, byte_range, line_column, object_path);
        let initial_failure = admission.failure().cloned();
        let initial_charge = self
            .store
            .charge_attempted(ImportDimension::DiagnosticCount, 1)
            .and_then(|()| {
                self.store.charge_attempted(
                    ImportDimension::DiagnosticBytes,
                    admission.fixed_attempted_bytes(),
                )
            });
        if let Err(cause) = initial_charge {
            diagnostics.borrow_mut().discard_admission();
            return Err(self.abort(cause.into_scope_abort_cause()));
        }
        if let Some(cause) = initial_failure {
            diagnostics.borrow_mut().discard_admission();
            return Err(self.abort(ScopeAbortCause::Diagnostic(cause)));
        }

        let stores_diagnostic = admission.storage_reservation_bytes() != 0;
        let mut reservation = if !stores_diagnostic {
            None
        } else {
            Some(
                self.store
                    .reserve_retained(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::DiagnosticBytes,
                        admission.payload_capacity_envelope(),
                        admission.payload_allocation_count(),
                    )
                    .map_err(|cause| self.abort(cause.into_scope_abort_cause()))?,
            )
        };
        let mut structural_reservation = if stores_diagnostic {
            Some(
                self.reserve_retained_storage_cause(
                    ImportStorageClass::FormatNormalization,
                    admission.structural_capacity_envelope(),
                    u64::from(admission.structural_capacity_envelope() != 0),
                )
                .map_err(|cause| self.abort(cause))?,
            )
        } else {
            None
        };
        if reservation.is_some() {
            let mut diagnostics = diagnostics
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

        let prepared = diagnostics
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
                if let Err(cause) = self.store.charge_attempted(
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

        if let Err(cause) = self.store.charge_attempted(
            ImportDimension::DiagnosticBytes,
            prepared.additional_attempted_bytes(),
        ) {
            diagnostics.borrow_mut().discard_prepared(prepared);
            return Err(self.abort(cause.into_scope_abort_cause()));
        }

        let (payload_reservation, structural_reservation) = if prepared.is_stored() {
            let Some(upper_bound) = reservation.take() else {
                diagnostics.borrow_mut().discard_prepared(prepared);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-reservation",
                }));
            };
            let payload_capacity_bytes = prepared.payload_capacity_bytes();
            let structural_bytes = prepared.structural_bytes();
            let mut exact = upper_bound;
            let payload_resize = self
                .store
                .resize(&mut exact, payload_capacity_bytes)
                .map_err(|cause| cause.into_scope_abort_cause());
            if let Err(cause) = payload_resize {
                diagnostics.borrow_mut().discard_prepared(prepared);
                drop(exact);
                return Err(self.abort(cause));
            }
            let Some(structural_upper_bound) = structural_reservation.take() else {
                diagnostics.borrow_mut().discard_prepared(prepared);
                drop(exact);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-structural-reservation",
                }));
            };
            let mut structural_exact = structural_upper_bound;
            let structural_resize = self
                .store
                .resize(&mut structural_exact, structural_bytes)
                .map_err(|cause| cause.into_scope_abort_cause());
            if let Err(cause) = structural_resize {
                diagnostics.borrow_mut().discard_prepared(prepared);
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

    fn commit_diagnostic_event(&mut self, event: PreparedDiagnosticEvent) -> ParseResult<()> {
        let diagnostics = self.views.diagnostics;

        if !event.is_stored() {
            return Ok(());
        }
        let mut diagnostics = match diagnostics.try_borrow_mut() {
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

    fn charge_diagnostic_consumed(&mut self, stored_bytes: u64) -> ParseResult<()> {
        self.store
            .charge_consumed_batch(&[
                (ImportDimension::DiagnosticCount, 1),
                (ImportDimension::DiagnosticBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn discard_diagnostic_event(&self, event: PreparedDiagnosticEvent) -> ParseResult<()> {
        let diagnostics = self.views.diagnostics;

        let mut diagnostics = diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        event.discard(&mut diagnostics);
        Ok(())
    }

    fn reserve_diagnostic_scratch_cause(
        &mut self,
        capacity_bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        self.store
            .reserve_retained(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::ScratchBytes,
                capacity_bytes,
                u64::from(capacity_bytes != 0),
            )
            .map_err(|cause| cause.into_scope_abort_cause())
    }
}

impl<'op, 'session> ParseRoot<'_, 'op, 'session> {
    /// Acquires one dependency relative to the resource that authored its hostile reference.
    ///
    /// Requiredness is derived exclusively from the selected format's frozen role declaration.
    pub fn dependency(
        &mut self,
        origin: &DependencyOrigin<'session>,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> ParseResult<DependencyOutcome<'session>> {
        let resource_graph = self.resource_graph;

        self.poll_active()?;
        if !origin.is_issued_by(self.views.issuer) {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-origin-foreign-session",
            }));
        }
        let requirement = self.frozen_dependency_requirement(role)?;

        let source = match origin.node() {
            DependencyOriginNode::Primary => {
                if !std::ptr::eq(origin.io(), self.views.primary_io) {
                    return Err(self.abort(ScopeAbortCause::DescriptorContract {
                        code: "dependency-origin-primary-backend-mismatch",
                    }));
                }
                resource_graph
                    .try_borrow()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .source_for_primary()
                    .map_err(|cause| self.abort(cause.into_scope_cause()))?
            }
            DependencyOriginNode::Dependency(index) => resource_graph
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
        let pending = resource_graph
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .begin_relation(&mut self.store, source, role, requirement, raw_reference)
            .map_err(|cause| self.abort(cause.into_scope_cause()))?;

        let authorized = match origin.node() {
            DependencyOriginNode::Primary => self.authorize_dependency(
                pending,
                origin.io(),
                self.views.primary_authority,
                self.views.source,
                role,
                raw_reference,
            ),
            DependencyOriginNode::Dependency(index) => {
                let cache = resource_graph
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
                resource_graph
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .complete_available(
                        &mut self.store,
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
                Err(self.abort_required_dependency_failure(origin, role, raw_reference, failure))
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
        self.views
            .entry
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
    pub fn into_available_dependency(
        &mut self,
        outcome: DependencyOutcome<'session>,
    ) -> ParseResult<DependencySnapshot<'session>> {
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
}

impl ParseServices<'_, '_, '_> {
    pub fn format_failure<P>(&mut self, producer: P) -> ParseFailure
    where
        P: FormatFailureEvidenceProducer,
    {
        parser_execution::capture_format_failure(
            parser_execution::FormatFailureHost::services(
                self.store.failure_storage(),
                self.views.entry,
                self.views.terminal,
                self.views.callback_active,
                self.views.failure_storage,
            ),
            producer,
        )
    }
}

impl<'op, 'session> ParseRoot<'_, 'op, 'session> {
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
        let dependency_failure = self.dependency_failure;

        let mut stored = dependency_failure.borrow_mut();
        if stored.is_none() {
            *stored = Some(context);
        }
        DependencyResolutionFailure
    }

    fn take_dependency_failure(
        &self,
        _failure: DependencyResolutionFailure,
    ) -> DependencyResolutionContext {
        let dependency_failure = self.dependency_failure;

        dependency_failure
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
        &mut self,
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
        &mut self,
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
        &mut self,
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

    fn authorize_dependency(
        &mut self,
        pending: PendingResourceRelation,
        origin_io: &'session dyn AssetIo,
        origin_authority: &AuthorityDomain,
        origin_source: &AssetPath,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> Result<
        (
            PendingResourceRelation,
            MeteredDependencyPath,
            AuthorizedDependency<'session>,
        ),
        DependencyResolutionFailure,
    > {
        if raw_reference.len() > self.views.limits.max_path_length {
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
                    limit: self.views.limits.max_path_length as u64,
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

    fn prepare_authorized_target(
        &mut self,
        mut pending: PendingResourceRelation,
        normalized: MeteredDependencyPath,
        authorized: AuthorizedDependency<'session>,
    ) -> Result<
        (
            PendingResourceRelation,
            MeteredDependencyPath,
            AuthorizedDependency<'session>,
        ),
        DependencyResolutionFailure,
    > {
        let resource_graph = self.resource_graph;

        let prepared = match resource_graph.try_borrow_mut() {
            Ok(mut graph) => graph
                .prepare_target(&mut self.store, &mut pending, authorized.identity())
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

    fn acquire_authorized(
        &mut self,
        pending: PendingResourceRelation,
        normalized: MeteredDependencyPath,
        authorized: AuthorizedDependency<'session>,
    ) -> Result<
        (
            DependencySnapshot<'session>,
            PendingResourceRelation,
            MeteredDependencyPath,
            usize,
        ),
        DependencyResolutionFailure,
    > {
        let resource_graph = self.resource_graph;

        let (io, identity, stable_identity_schema, registered_resolver) = authorized.into_parts();
        let backend_authority = if std::ptr::eq(io, self.views.primary_io) {
            self.views.primary_authority.clone()
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
            let graph = resource_graph.borrow();
            match graph.acquisition_state(&mut self.store, &identity) {
                Ok(Some(ResourceAcquisitionState::Available {
                    cache_index,
                    bytes,
                    stable_identity_schema,
                })) => Some((cache_index, bytes, stable_identity_schema)),
                Ok(Some(ResourceAcquisitionState::Failed {
                    cache_index,
                    stable_identity_schema: cached_stable_identity_schema,
                })) => {
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
                Ok(Some(ResourceAcquisitionState::Resolving)) => {
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-resolution-reentered",
                        },
                    ));
                }
                Ok(None) => None,
                Err(cause) => {
                    return Err(self.dependency_failure(
                        Some(pending),
                        Some(identity),
                        normalized,
                        cause.into_scope_cause(),
                    ));
                }
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
                    self.views.issuer,
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

        let capacity = match resource_graph.try_borrow_mut() {
            Ok(mut graph) => graph.reserve_acquisition_capacity(&mut self.store),
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
        let marked = match resource_graph.borrow_mut().begin_acquisition(
            &mut self.store,
            cache_identity,
            cache_identity_path_reservation,
        ) {
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
                let completed = resource_graph.borrow_mut().complete_acquisition(
                    &mut self.store,
                    &identity,
                    Arc::clone(&snapshot_bytes),
                    stable_identity_schema,
                );
                let origin_node = match completed {
                    Ok(origin_node) => origin_node,
                    Err(cause) => {
                        // Preserve the original admission/control failure. Cleanup is metered too;
                        // a denied cleanup leaves the session-owned resolving row for terminal drop.
                        let _ = resource_graph
                            .borrow_mut()
                            .abandon_acquisition(&mut self.store, &identity);
                        return Err(self.dependency_failure(
                            Some(pending),
                            Some(identity),
                            normalized,
                            cause.into_scope_cause(),
                        ));
                    }
                };
                Ok((
                    DependencySnapshot::new(
                        self.views.issuer,
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
        &mut self,
        pending: PendingResourceRelation,
        identity: DependencyIdentity,
        observation_context: DependencyFailureObservationContext<'_>,
        normalized: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        let resource_graph = self.resource_graph;

        // Operation-terminal failures must not be replaced by another probe admission while
        // trying to cache an unavailable input. Only acquisition-local budgets with authorized
        // optional fallback keep the cache/evidence path; refused cleanup stays session-owned.
        let optional_fallback = pending.requirement() == DependencyRequirement::Optional
            && self
                .views
                .loss_policy
                .allows(ImportLossKind::OptionalDependencyUnavailable)
            && optional_eligible(&cause);
        if matches!(
            &cause,
            ScopeAbortCause::Control(_) | ScopeAbortCause::TrustedExtension { .. }
        ) || (matches!(&cause, ScopeAbortCause::Ledger(_)) && !optional_fallback)
        {
            let _ = resource_graph
                .borrow_mut()
                .abandon_acquisition(&mut self.store, &identity);
            return self.dependency_failure(Some(pending), Some(identity), normalized, cause);
        }
        let observation = match self.registered_negative_observation(
            observation_context.registered_resolver,
            observation_context.io,
            &identity,
            observation_context.stable_identity_schema,
            &cause,
        ) {
            Ok(observation) => observation,
            Err(proof_cause) => {
                let _ = resource_graph
                    .borrow_mut()
                    .abandon_acquisition(&mut self.store, &identity);
                return self.dependency_failure(
                    Some(pending),
                    Some(identity),
                    normalized,
                    proof_cause,
                );
            }
        };
        let cached_index = resource_graph.borrow_mut().complete_failed_acquisition(
            &mut self.store,
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
                let _ = resource_graph
                    .borrow_mut()
                    .abandon_acquisition(&mut self.store, &identity);
                self.dependency_failure(Some(pending), Some(identity), normalized, cause)
            }
        }
    }

    fn registered_negative_observation<'target>(
        &mut self,
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
        let layout = self
            .store
            .bind_group(charges.map(|(dimension, _)| dimension))
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let proof = self
            .store
            .run_fallible_action(&layout, charges.map(|(_, amount)| amount), || {
                let proof = plan
                    .verify(identity.path(), registered_cause)
                    .map_err(registered_negative_proof_error)?;
                Ok(proof)
            })
            .map_err(|cause| cause.into_scope_abort_cause())?;
        match proof {
            Ok(proof) => Ok(PendingNegativeObservation::Proof(proof)),
            Err(cause) => Err(cause),
        }
    }

    fn open_and_read_dependency(
        &mut self,
        io: &dyn AssetIo,
        identity: &DependencyIdentity,
    ) -> AssetSnapshotResult {
        self.poll_cause()?;
        self.charge_open_attempt()?;
        let opened = crate::asset_open::open_asset(
            &mut self.store,
            io,
            identity.path(),
            crate::asset_open::AssetOpenContext::Dependency,
        )?;
        if !opened
            .asset()
            .matches_identity(identity.authority(), identity.path())
        {
            return Err(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "opened-object-identity-mismatch",
            });
        }
        self.charge_open_consumed()?;
        self.admit_dependency_acquisition_work()?;
        let bytes = read_opened_asset(
            &mut self.store,
            opened,
            ImportLedgerPhase::DependencyAcquisition,
            ImportProgressPhase::AcquireDependency,
            ImportDimension::DependencyBytes,
            self.views.limits.max_dependency_asset_bytes,
            self.views.limits.max_total_asset_bytes,
        )?;
        self.charge_dependency_acquisition_work_consumed()?;
        Ok(bytes)
    }

    fn share_dependency_snapshot(
        &mut self,
        bytes: AccountedVec<u8>,
    ) -> Result<Arc<SharedDependencySnapshot>, ScopeAbortCause> {
        share_dependency_snapshot_with(bytes, |control_bytes| {
            self.reserve_dependency_storage_cause(control_bytes)
        })
    }

    fn clone_dependency_snapshot_identity(
        &mut self,
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

    fn charge_logical_reference(&mut self) -> Result<(), DependencyResolutionFailure> {
        let result = self
            .store
            .charge_attempted(ImportDimension::DependencyReference, 1)
            .and_then(|()| {
                self.store
                    .charge_consumed(ImportDimension::DependencyReference, 1)
            });
        let Err(cause) = result else {
            return Ok(());
        };
        Err(self.dependency_failure_without_path(None, cause.into_scope_abort_cause()))
    }

    fn charge_open_attempt(&mut self) -> Result<(), ScopeAbortCause> {
        self.store
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn charge_open_consumed(&mut self) -> Result<(), ScopeAbortCause> {
        self.store
            .charge_consumed(ImportDimension::AssetOpen, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn admit_dependency_acquisition_work(&mut self) -> Result<(), ScopeAbortCause> {
        self.store
            .charge_attempted(ImportDimension::DependencyAcquisitionWork, 1)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let state = self
            .store
            .dimension_state(ImportDimension::DependencyAcquisitionWork)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let value = state.consumed().checked_add(1).ok_or({
            ScopeAbortCause::Ledger(LedgerCause::ArithmeticOverflow {
                class: CounterClass::Consumed,
                dimension: ImportDimension::DependencyAcquisitionWork,
            })
        })?;
        if value > self.views.limits.max_dependency_acquisition_work {
            return Err(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Consumed,
                dimension: ImportDimension::DependencyAcquisitionWork,
                value,
                limit: self.views.limits.max_dependency_acquisition_work,
            }));
        }
        Ok(())
    }

    fn charge_dependency_acquisition_work_consumed(&mut self) -> Result<(), ScopeAbortCause> {
        self.store
            .charge_consumed(ImportDimension::DependencyAcquisitionWork, 1)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn poll_cause(&mut self) -> Result<(), ScopeAbortCause> {
        self.store
            .poll()
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resolve_optional_failure(
        &mut self,
        role: &DependencyRoleId,
        failure: DependencyResolutionFailure,
    ) -> ParseResult<DependencyOutcome<'session>> {
        let resource_graph = self.resource_graph;
        let optional_dependency_evidence = self.optional_dependency_evidence;

        let mut failure = self.take_dependency_failure(failure);
        let (eligible, owner, disposition) = {
            let cache = resource_graph
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let cause = failure.cause(&cache).map_err(|cause| self.abort(cause))?;
            let disposition = failure
                .disposition(&cache)
                .map_err(|cause| self.abort(cause))?;
            (optional_eligible(cause), cause.owner(), disposition)
        };
        if !self
            .views
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
            let mut retained = optional_dependency_evidence
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
        let code = DiagnosticCode::new("import.optional_dependency_unavailable")
            .map_err(|cause| self.abort(ScopeAbortCause::Diagnostic(cause)))?;
        self.services()?.emit_diagnostic(
            DiagnosticSeverity::Warning,
            code,
            format_args!("optional dependency role {role} is unavailable"),
        )?;
        Ok(DependencyOutcome::unavailable(evidence))
    }

    fn complete_unavailable_relation(
        &mut self,
        failure: &mut DependencyResolutionContext,
        disposition: ResourceUnavailableDisposition,
    ) -> ParseResult<()> {
        let resource_graph = self.resource_graph;

        let relation = failure.relation.take().ok_or_else(|| {
            self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-terminal-relation-missing",
            })
        })?;
        let normalized_reference = failure.normalized_manifest_reference();
        resource_graph
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .complete_unavailable(
                &mut self.store,
                relation,
                normalized_reference,
                failure.identity.as_ref(),
                disposition,
            )
            .map_err(|cause| self.abort(cause.into_scope_cause()))
    }

    fn abort_dependency_failure(&mut self, failure: DependencyResolutionFailure) -> ParseFailure {
        let context = self.take_dependency_failure(failure);
        self.abort_dependency_context(context)
    }

    fn abort_required_dependency_failure(
        &mut self,
        origin: &DependencyOrigin<'_>,
        role: &DependencyRoleId,
        raw_reference: &str,
        failure: DependencyResolutionFailure,
    ) -> ParseFailure {
        let resource_graph = self.resource_graph;
        let format_failure_storage = self.views.failure_storage;
        let dependency_failure_context = self.dependency_failure_context;

        let failure = self.take_dependency_failure(failure);
        let captured = (|| {
            let graph =
                resource_graph
                    .try_borrow()
                    .map_err(|_| ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    })?;
            // A ledger/control terminal may prohibit further admission. Preserve that original
            // failure without attempting an allocation or weakening requiredness.
            if !matches!(
                failure.cause(&graph)?,
                ScopeAbortCause::Path(_)
                    | ScopeAbortCause::AssetOpen(_)
                    | ScopeAbortCause::AssetRead(_)
                    | ScopeAbortCause::DependencyPolicy(_)
            ) {
                return Ok(None);
            }
            if format_failure_storage.borrow().is_active() {
                return Err(ScopeAbortCause::DescriptorContract {
                    code: "failure-evidence-storage-already-active",
                });
            }
            let (origin_authority, origin_path) = match origin.node() {
                DependencyOriginNode::Primary => (self.views.primary_authority, self.views.source),
                DependencyOriginNode::Dependency(index) => {
                    let identity = graph.origin_identity(index).ok_or(
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-origin-stale",
                        },
                    )?;
                    (identity.authority(), identity.path())
                }
            };
            DependencyFailureContext::try_capture(
                &mut self.store,
                origin_authority,
                origin_path,
                role,
                raw_reference,
                failure.evidence.manifest_reference(),
                failure.identity.as_ref(),
            )
            .map(Some)
        })();
        match captured {
            Ok(Some((context, reservation))) => {
                // The single failure slot keeps this physical lease coupled to the cause until
                // the terminal owner destroys it, including rich and minimal outcomes.
                format_failure_storage.borrow_mut().reservation = Some(reservation);
                *dependency_failure_context.borrow_mut() = Some(context);
            }
            Ok(None) => {}
            Err(cause) => return self.abort(cause),
        }
        self.abort_dependency_context(failure)
    }

    fn abort_dependency_context(&mut self, failure: DependencyResolutionContext) -> ParseFailure {
        let resource_graph = self.resource_graph;

        let (disposition, mut cause, relation, identity, evidence) = {
            let graph = match resource_graph.try_borrow() {
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
        let mut graph = resource_graph
            .try_borrow_mut()
            .expect("resource graph borrow was released before mutation");
        let normalized_reference = evidence.manifest_reference();
        if let Some(relation) = relation
            && let Err(graph_cause) = graph.complete_unavailable(
                &mut self.store,
                relation,
                normalized_reference,
                identity.as_ref(),
                disposition,
            )
        {
            // A terminal control/ledger/extension cause already identifies the failed action.
            // Further best-effort graph accounting must not replace it with a later excess.
            if !matches!(
                &cause,
                ScopeAbortCause::Control(_)
                    | ScopeAbortCause::Ledger(_)
                    | ScopeAbortCause::TrustedExtension { .. }
            ) {
                cause = graph_cause.into_scope_cause();
            }
        }
        drop(graph);
        drop(identity);
        let parse_failure = self.abort(cause);
        drop(evidence);
        parse_failure
    }

    fn clone_optional_dependency_evidence(
        &mut self,
        identity: Option<&DependencyIdentity>,
        normalized_path: &AssetPath,
        owner: &crate::CauseOwner,
        cache_disposition: DependencyCacheDisposition,
    ) -> ParseResult<OptionalDependencyEvidence> {
        let (normalized_path, normalized_path_reservation) = self
            .clone_retained_dependency_path(normalized_path, "optional-dependency-evidence-path")?;
        let (identity, identity_path_reservation) = match identity {
            Some(identity) => {
                let (path, reservation) = self.clone_retained_dependency_path(
                    identity.path(),
                    "optional-dependency-evidence-identity",
                )?;
                let identity = DependencyIdentity::new(identity.authority().clone(), path);
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

    fn clone_retained_dependency_path(
        &mut self,
        path: &AssetPath,
        resource: &'static str,
    ) -> ParseResult<(AssetPath, ImportStoreReservation)> {
        let plan = CapacityPlan::for_type::<u8>(0, 0, path.as_str().len())
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        let mut reservation = self
            .reserve_dependency_storage_cause(bytes)
            .map_err(|cause| self.abort(cause))?;
        let path = path
            .try_clone()
            .map_err(|_| self.allocation_failure(resource))?;
        let bytes = plan
            .finish(0, path.capacity_bytes())
            .map_err(|cause| self.capacity_plan_error(resource, cause))?;
        self.resize_dependency_storage_cause(&mut reservation, bytes)
            .map_err(|cause| self.abort(cause))?;
        Ok((path, reservation))
    }

    fn reserve_dependency_storage_cause(
        &mut self,
        bytes: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        self.store
            .reserve_retained(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                bytes,
                u64::from(bytes != 0),
            )
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn resize_dependency_storage_cause(
        &mut self,
        reservation: &mut ImportStoreReservation,
        bytes: u64,
    ) -> Result<(), ScopeAbortCause> {
        self.store
            .resize(reservation, bytes)
            .map_err(|cause| cause.into_scope_abort_cause())
    }

    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        latch_abort(self.views.terminal, cause)
    }
}

impl ParseServices<'_, '_, '_> {
    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        latch_abort(self.views.terminal, cause)
    }

    fn preflight_active(&self) -> ParseResult<()> {
        if self.views.callback_active.get() {
            return Err(self.reentrant_capability_failure());
        }
        preflight_terminal(self.views.terminal)
    }

    pub fn poll(&mut self) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
            .poll()
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    pub(crate) fn source_scope_abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        self.abort(cause)
    }

    pub(crate) fn source_scope_charge_batch(
        &mut self,
        charges: &[(ImportDimension, u64); 2],
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
            .charge_observed_batch(charges)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn source_scope_charge_attempted(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn source_scope_charge_consumed(
        &mut self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
            .charge_consumed(dimension, amount)
            .map_err(|cause| self.abort(cause.into_scope_abort_cause()))
    }

    fn charge_source_scope_bundle_consumed(
        &mut self,
        loss_bytes: u64,
        diagnostic_bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.store
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

fn append_metered_string(
    exec: &mut ParseServices<'_, '_, '_>,
    output: &mut String,
    value: &str,
) -> ParseResult<()> {
    storage::append_metered_string(&mut exec.storage()?, output, value)
}

impl StorageIdentity<'_> {
    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        latch_abort(self.terminal, cause)
    }

    fn preflight_active(&self) -> ParseResult<()> {
        preflight_terminal(self.terminal)
    }

    fn check(&self, exec: &ParseServices<'_, '_, '_>) -> ParseResult<()> {
        exec.check_session(self.issuer)?;
        self.preflight_active()
    }

    fn reserve(
        &self,
        exec: &mut ParseServices<'_, '_, '_>,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<Option<ImportStoreReservation>> {
        self.check(exec)?;
        if bytes == 0 {
            return Ok(None);
        }
        exec.reserve_storage_cause(class, bytes)
            .map(Some)
            .map_err(|cause| self.abort(cause))
    }

    fn resize_exact(
        &self,
        exec: &mut ParseServices<'_, '_, '_>,
        reservation: &mut Option<ImportStoreReservation>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.check(exec)?;
        match (reservation.as_mut(), bytes) {
            (None, 0) => Ok(()),
            (Some(_), 0) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-unused-capacity-reservation",
            })),
            (Some(token), bytes) => exec
                .resize_storage_reservation(token, bytes)
                .map_err(|cause| self.abort(cause)),
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

impl<'session, T> MeteredVec<'session, T> {
    pub fn try_reserve(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        additional: usize,
    ) -> ParseResult<()> {
        exec.storage()?.reserve_vec(self, additional)
    }

    pub fn try_push(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        value: T,
    ) -> ParseResult<()> {
        exec.storage()?.push_vec(self, value)
    }

    /// Pushes into a slot admitted by an earlier reserve without polling or entering the allocator.
    pub fn try_push_reserved(&mut self, value: T) -> ParseResult<()> {
        self.identity.preflight_active()?;
        if self.values.len() == self.values.capacity() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-capacity-not-reserved",
            }));
        }
        self.values.push(value);
        Ok(())
    }

    pub fn try_extend_from_slice(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        values: &[T],
    ) -> ParseResult<()>
    where
        T: Copy,
    {
        self.try_reserve(exec, values.len())?;
        for chunk in values.chunks(metered_copy_chunk_len::<T>()) {
            self.identity.check(exec)?;
            exec.poll()?;
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
        self.identity.preflight_active()?;
        let required = self
            .values
            .len()
            .checked_add(values.len())
            .ok_or_else(|| self.identity.allocation_failure(self.resource))?;
        if required > self.values.capacity() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-capacity-not-reserved",
            }));
        }
        if values.len() > metered_copy_chunk_len::<T>() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-vector-copy-chunk-too-large",
            }));
        }
        self.values.extend_from_slice(values);
        Ok(())
    }

    pub fn try_resize(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        len: usize,
        value: T,
    ) -> ParseResult<()>
    where
        T: Copy,
    {
        exec.storage()?.resize_vec(self, len, value)
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

impl<'session, K: Ord, V> MeteredOrderedMap<'session, K, V> {
    pub fn try_reserve(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        additional: usize,
    ) -> ParseResult<()> {
        self.identity.check(exec)?;
        exec.poll()?;
        let required = self
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.identity.allocation_failure(self.resource))?;
        if required <= self.value.capacity() {
            return Ok(());
        }

        let plan = self
            .value
            .capacity_plan(additional)
            .map_err(|cause| metered_map_error(&self.identity, self.resource, cause))?;
        let admitted = plan
            .replacement_envelope_bytes()
            .map_err(|cause| metered_map_error(&self.identity, self.resource, cause))?;
        let mut token = self.identity.reserve(exec, self.class, admitted)?;
        let replacement = self
            .value
            .prepare_capacity_replacement(plan)
            .map_err(|cause| metered_map_error(&self.identity, self.resource, cause))?;
        let complete_bytes = replacement.allocation_bytes();
        self.identity
            .resize_exact(exec, &mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(self.identity.qualification_failure(self.resource));
        }
        self.identity.check(exec)?;
        exec.poll()?;

        replacement.commit();
        let retired_token = std::mem::replace(&mut self.capacity, token);
        drop(retired_token);
        Ok(())
    }

    pub fn try_insert(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        key: K,
        value: V,
    ) -> ParseResult<Option<V>> {
        let additional = usize::from(!self.value.contains_key(&key));
        self.try_reserve(exec, additional)?;
        self.value
            .insert_reserved(key, value)
            .map_err(|cause| metered_map_error(&self.identity, self.resource, cause))
    }
}

fn metered_map_error(
    meter: &StorageIdentity<'_>,
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

impl<'session> MeteredString<'session> {
    pub fn try_reserve(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        additional: usize,
    ) -> ParseResult<()> {
        exec.storage()?.reserve_string(self, additional)
    }

    pub fn try_push_str(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        value: &str,
    ) -> ParseResult<()> {
        exec.storage()?.append_string(self, value)
    }

    /// Appends bytes admitted by an earlier reserve without polling or entering the allocator.
    ///
    /// A caller-owned work boundary must cover the copy. The fixed chunk ceiling prevents one
    /// call from weakening cancellation latency.
    pub fn try_push_str_reserved(&mut self, value: &str) -> ParseResult<()> {
        self.identity.preflight_active()?;
        let required = self
            .value
            .len()
            .checked_add(value.len())
            .ok_or_else(|| self.identity.allocation_failure(self.resource))?;
        if required > self.value.capacity() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-string-capacity-not-reserved",
            }));
        }
        if value.len() > METERED_COPY_POLL_INTERVAL {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
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

    pub fn try_push_char(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        value: char,
    ) -> ParseResult<()> {
        let mut encoded = [0_u8; 4];
        let encoded = value.encode_utf8(&mut encoded);
        self.try_push_str(exec, encoded)
    }

    pub fn try_push_usize(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        value: usize,
    ) -> ParseResult<()> {
        self.try_write_fmt(exec, format_args!("{value}"))
    }

    pub fn try_write_fmt(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        arguments: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        exec.storage()?.write_string_fmt(self, arguments)
    }
}

impl<'session> MeteredDecodedText<'session> {
    pub fn try_push_str(
        &mut self,
        exec: &mut ParseServices<'_, '_, 'session>,
        value: &str,
    ) -> ParseResult<()> {
        self.identity.check(exec)?;
        self.validate_append(value.len())?;
        append_metered_string(exec, &mut self.value, value)
    }

    /// Appends decoded bytes under a caller-owned bounded work chunk without polling again.
    pub fn try_push_str_reserved(&mut self, value: &str) -> ParseResult<()> {
        self.identity.preflight_active()?;
        self.validate_append(value.len())?;
        if value.len() > METERED_COPY_POLL_INTERVAL {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
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
            .ok_or_else(|| self.identity.allocation_failure(self.resource))?;
        if required > self.decoded_len || required > self.value.capacity() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "decoded-text-length-exhausted",
            }));
        }
        Ok(())
    }

    pub fn finish_exact(self, exec: &mut ParseServices<'_, '_, 'session>) -> ParseResult<Self> {
        self.identity.check(exec)?;
        exec.poll()?;
        if self.value.len() != self.decoded_len {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
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
            dimension: ImportDimension::DependencyBytes | ImportDimension::DependencyAcquisitionWork,
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
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    mut opened: OpenedAssetOwner,
    ledger_phase: ImportLedgerPhase,
    progress_phase: ImportProgressPhase,
    dimension: ImportDimension,
    per_resource_limit: u64,
    aggregate_limit: u64,
) -> AssetSnapshotResult {
    let result = read_asset_snapshot(
        exec,
        opened.asset_mut(),
        ledger_phase,
        progress_phase,
        dimension,
        per_resource_limit,
        aggregate_limit,
    );
    opened.finish(result)
}

fn read_asset_snapshot(
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    opened: &mut OpenedAsset,
    ledger_phase: ImportLedgerPhase,
    progress_phase: ImportProgressPhase,
    dimension: ImportDimension,
    per_resource_limit: u64,
    aggregate_limit: u64,
) -> AssetSnapshotResult {
    exec.poll()
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
        admit_asset_bytes(exec, dimension, length, per_resource_limit, aggregate_limit)?;
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
        remaining_asset_bytes(exec, ImportDimension::TotalAssetBytes, aggregate_limit)?
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
            exec.reserve(ledger_phase, dimension, capacity_bytes)
                .map_err(|cause| cause.into_scope_abort_cause())
        })?;
    }

    let mut chunk = [0_u8; 16 * 1024];
    loop {
        exec.poll()
            .map_err(|cause| cause.into_scope_abort_cause())?;
        // Attempted bytes admit payload capacity once. A short read keeps the unused
        // allowance for the next call; it does not refund or readmit that capacity.
        let mut allowance = attempted_precharged.saturating_sub(opened.bytes_read());
        if allowance == 0 && exact_length.is_none() {
            let resource_remaining = effective_limit.saturating_sub(attempted_precharged);
            let aggregate_remaining =
                remaining_asset_bytes(exec, ImportDimension::TotalAssetBytes, aggregate_limit)?;
            allowance = resource_remaining
                .min(aggregate_remaining)
                .min(chunk.len() as u64);
            if allowance != 0 {
                admit_asset_bytes(
                    exec,
                    dimension,
                    allowance,
                    per_resource_limit,
                    aggregate_limit,
                )?;
                attempted_precharged += allowance;
            }
        }
        // A nonempty one-byte request distinguishes EOF from excess input when no
        // payload credit remains. This is validation/read work, never stored payload.
        let eof_probe = allowance == 0;
        let requested = if eof_probe {
            1
        } else {
            allowance.min(chunk.len() as u64) as usize
        };
        let before = opened.bytes_read();
        let read = read_asset_chunk(exec, opened, &mut chunk[..requested]);
        let after = opened.bytes_read();
        let observed = after.checked_sub(before).ok_or({
            ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "asset-read-counter-regressed",
            }
        })?;
        if after > attempted_precharged {
            // Retain truthful observed excess in attempted evidence when the existing
            // counters permit it. Failed admission never fabricates consumed bytes.
            admit_asset_bytes(
                exec,
                dimension,
                observed,
                per_resource_limit,
                aggregate_limit,
            )?;
        }
        if observed != 0 {
            charge_asset_consumed(exec, dimension, observed)?;
        }
        let count = read?.map_err(ScopeAbortCause::AssetRead)?;
        let count_u64 = count as u64;
        if count_u64 != observed {
            return Err(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "asset-read-counter-mismatch",
            });
        }
        if count == 0 {
            break;
        }
        if after > effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
        if eof_probe {
            return Err(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: ImportDimension::TotalAssetBytes,
                value: exec
                    .dimension_state(ImportDimension::TotalAssetBytes)
                    .map_err(|cause| cause.into_scope_abort_cause())?
                    .attempted(),
                limit: aggregate_limit,
            }));
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
                exec.reserve(ledger_phase, dimension, capacity_bytes)
                    .map_err(|cause| cause.into_scope_abort_cause())
            })?;
        }
        bytes
            .try_extend_from_slice_reserved(&chunk[..count])
            .map_err(|()| ScopeAbortCause::DescriptorContract {
                code: "asset-snapshot-slots-not-admitted",
            })?;
        let completed = exec
            .dimension_state(dimension)
            .map_err(|cause| cause.into_scope_abort_cause())?
            .consumed();
        exec.report_progress(
            progress_phase,
            ImportProgressUnit::Bytes,
            completed,
            progress_total,
        )
        .map_err(|cause| cause.into_scope_abort_cause())?;
    }

    Ok(bytes)
}

fn read_asset_chunk(
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    opened: &mut OpenedAsset,
    buffer: &mut [u8],
) -> Result<Result<usize, baozi_io::AssetReadError>, ScopeAbortCause> {
    let layout = exec
        .bind_group([ImportDimension::WorkUnits])
        .map_err(|cause| cause.into_scope_abort_cause())?;
    exec.run_fallible_action(&layout, [1], || {
        match catch_unwind(AssertUnwindSafe(|| opened.read_chunk(buffer))) {
            Ok(read) => {
                // EOF and returned read errors complete a call; a panic does not.
                Ok(read)
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                Err(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "asset-read-panicked",
                })
            }
        }
    })
    .map_err(|cause| cause.into_scope_abort_cause())?
}

fn admit_asset_bytes(
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    dimension: ImportDimension,
    amount: u64,
    per_resource_limit: u64,
    aggregate_limit: u64,
) -> Result<(), ScopeAbortCause> {
    // Snapshot acquisition is serial. Unspent admission remains in the operation
    // ledger and can cover the next resource, including a different byte class.
    // Each dimension needs its own allowance; aggregate credit is not charged twice.
    for (dimension, limit) in [
        (
            dimension,
            if dimension == ImportDimension::PrimaryBytes {
                per_resource_limit
            } else {
                aggregate_limit
            },
        ),
        (ImportDimension::TotalAssetBytes, aggregate_limit),
    ] {
        let state = exec
            .dimension_state(dimension)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        if state.attempted() > limit {
            return Err(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension,
                value: state.attempted(),
                limit,
            }));
        }
        let unused = state.attempted().saturating_sub(state.consumed());
        if amount > unused {
            exec.charge_attempted(dimension, amount - unused)
                .map_err(|cause| cause.into_scope_abort_cause())?;
        }
    }
    Ok(())
}

fn remaining_asset_bytes(
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    dimension: ImportDimension,
    limit: u64,
) -> Result<u64, ScopeAbortCause> {
    let state = exec
        .dimension_state(dimension)
        .map_err(|cause| cause.into_scope_abort_cause())?;
    if state.attempted() > limit {
        return Err(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension,
            value: state.attempted(),
            limit,
        }));
    }
    Ok(limit.saturating_sub(state.consumed()))
}

fn charge_asset_consumed(
    exec: &mut crate::store::ImportRootExecution<'_, '_>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ScopeAbortCause> {
    exec.charge_consumed_batch(&[
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
pub(crate) mod testing;

#[cfg(test)]
mod ownership_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::ImportOperationControl;
    use crate::store::{
        ImportRootExecution, ImportStoreBodyParts, ImportStoreBootstrapRequirements,
        ImportStoreHostError, ImportStoreInvocationRoot, ImportStoreSessionOwner,
        try_import_store_bootstrap,
    };
    use baozi_core::__scene_integration::integration::SceneParseOutcome;
    use baozi_io::AssetPath;
    use baozi_io::{AssetLength, AuthorityDomain};
    use baozi_operation::{
        DimensionPolicy, PhysicalStorageAccount, PhysicalStorageAccountFactory,
        PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
    };
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestStore<'inv> {
        owner: ImportStoreSessionOwner<'inv>,
        account: PhysicalStorageAccount,
    }

    fn assert_store_released(account: PhysicalStorageAccount) {
        let released = account.snapshot();
        assert_eq!(released.live_retained_bytes(), 0);
        assert_eq!(released.live_retained_allocations(), 0);
        assert!(!released.poisoned());
    }

    fn test_store_with_control<'inv>(
        parts: ImportStoreBodyParts<'inv>,
        limits: &ResourceLimits,
        control: ImportOperationControl,
    ) -> TestStore<'inv> {
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
        let owner = ImportStoreSessionOwner::try_start_in_body(parts, control, &account, bootstrap)
            .expect("test import store owner must start");
        TestStore { owner, account }
    }

    fn with_store_body(
        limits: ResourceLimits,
        control: ImportOperationControl,
        body: impl for<'inv> FnOnce(&mut TestStore<'inv>),
    ) {
        let mut root = ImportStoreInvocationRoot::new();
        let mut startup = Some((control, body));
        let mut account = None;
        let finish = root
            .run_body(|parts, disposition| {
                let (control, body) = startup.take().unwrap();
                let mut store = test_store_with_control(parts, &limits, control);
                body(&mut store);
                let TestStore {
                    owner,
                    account: started_account,
                } = store;
                owner
                    .request_finish(disposition, SceneParseOutcome::Rejected)
                    .unwrap();
                account = Some(started_account);
            })
            .expect("the inspected import body must record its actual disposition");
        let _finish = finish;
        drop(root);
        assert_store_released(account.unwrap());
    }

    fn with_store_root_body(
        limits: ResourceLimits,
        control: ImportOperationControl,
        body: impl for<'op> FnOnce(&mut ImportRootExecution<'_, 'op>),
    ) {
        with_store_body(limits, control, |store| {
            store
                .owner
                .with_root_execution(|root| {
                    body(root);
                    Ok::<_, ImportStoreHostError>(())
                })
                .expect("test root execution must be available");
        });
    }

    fn current_live_bytes(host: &ImportRootExecution<'_, '_>) -> u64 {
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
        with_store_root_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |host| {
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
            },
        );
    }

    #[test]
    fn session_storage_crosses_regions_with_real_parent_and_child_channels() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let mut values = context
                        .with_execution(|exec| -> ParseResult<_> {
                            let parent = exec.bind_group([ImportWorkClass::Records])?;
                            let child = exec.bind_group([
                                ImportWorkClass::Tokens,
                                ImportWorkClass::DecodeWork,
                            ])?;
                            let mut values = exec.services()?.metered_vec(
                                ImportStorageClass::FormatNormalization,
                                "region-owned-values",
                            );
                            exec.parent(&parent, [2], |parent| -> ParseResult<()> {
                                assert_eq!(parent.remaining(), [2]);
                                for value in [3_u32, 5] {
                                    parent.child(&child, [1, 1], |child| -> ParseResult<()> {
                                        child.push_growing([1, 1], &mut values, value)
                                    })?;
                                }
                                Ok(())
                            })?;
                            Ok(values)
                        })
                        .unwrap();
                    context
                        .with_execution(|exec| -> ParseResult<()> {
                            values.try_extend_from_slice(&mut exec.services()?, &[7, 11, 13])?;
                            assert_eq!(values.as_slice(), &[3, 5, 7, 11, 13]);
                            for (dimension, amount) in [
                                (ImportDimension::Records, 2),
                                (ImportDimension::Tokens, 2),
                                (ImportDimension::DecodeWork, 2),
                            ] {
                                let state = exec.store.dimension_state(dimension).unwrap();
                                assert_eq!((state.attempted(), state.consumed()), (amount, amount));
                            }
                            Ok(())
                        })
                        .unwrap();
                });
            },
        );
    }

    #[test]
    fn incomplete_callback_keeps_the_exact_completed_prefix_and_latches_failure() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let result = context.with_execution(|exec| -> ParseResult<()> {
                        let group = exec.bind_group([ImportWorkClass::Tokens])?;
                        let result = exec.child(&group, [3], |child| child.run([1], || Ok(())));
                        let state = exec.store.dimension_state(ImportDimension::Tokens).unwrap();
                        assert_eq!((state.attempted(), state.consumed()), (3, 1));
                        result
                    });
                    assert!(result.is_err());
                    assert!(matches!(
                        context.take_terminal_abort().unwrap().cause(),
                        ScopeAbortCause::DescriptorContract {
                            code: "parse-work-callback-incomplete"
                        },
                    ));
                });
            },
        );
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
    fn dependency_cache_completion_budget_failure_is_not_an_optional_input_failure() {
        #[derive(Clone, Copy)]
        enum FailurePoint {
            AvailableTransition,
            FailedTransition,
            SnapshotSharing,
            Acquisition,
        }

        struct BudgetImporter;

        impl crate::Format for BudgetImporter {
            type Options = crate::NoFormatOptions;

            fn compile_defaults(
                &self,
                scope: &mut crate::RegistryBuildScope<'_>,
            ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError>
            {
                crate::NoFormatOptions::compile_defaults(scope)
            }

            fn descriptor(&self) -> crate::FormatDescriptor {
                crate::FormatDescriptor::builder(
                    baozi_core::FormatId::new("org.baozi.test.cache-completion").unwrap(),
                    "Cache completion regression",
                )
                .extensions(["bin"])
                .encoding(crate::FormatEncoding::Binary)
                .dependency_roles([
                    (
                        DependencyRoleId::new("optional").unwrap(),
                        DependencyRequirement::Optional,
                    ),
                    (
                        DependencyRoleId::new("required").unwrap(),
                        DependencyRequirement::Required,
                    ),
                ])
                .build()
                .unwrap()
            }

            fn detect(
                &self,
                _probe: &crate::Probe<'_>,
                _hint: &crate::ReadHint,
                _scope: &mut crate::DetectionScope<'_, '_>,
            ) -> crate::DetectionResult<crate::DetectionOutcome> {
                Ok(crate::DetectionOutcome::NoMatch)
            }

            fn parse(
                &self,
                _options: &Self::Options,
                _scope: &mut ParseContext<'_, '_>,
            ) -> ParseResult<()> {
                unreachable!("this test exercises the selected ParseContext directly")
            }
        }

        let mut registry = crate::RegistryBuilder::new();
        registry.register(BudgetImporter).unwrap();
        let registry = registry
            .freeze(crate::RegistryBuildLimits::default())
            .unwrap();
        let entry = registry.entries().next().unwrap();
        for (failure_point, requirement, allow_optional) in [
            (
                FailurePoint::AvailableTransition,
                DependencyRequirement::Optional,
                true,
            ),
            (
                FailurePoint::FailedTransition,
                DependencyRequirement::Optional,
                true,
            ),
            (
                FailurePoint::SnapshotSharing,
                DependencyRequirement::Optional,
                true,
            ),
            (
                FailurePoint::Acquisition,
                DependencyRequirement::Required,
                true,
            ),
            (
                FailurePoint::Acquisition,
                DependencyRequirement::Optional,
                false,
            ),
        ] {
            let available = !matches!(failure_point, FailurePoint::FailedTransition);
            let acquisition_denied = matches!(failure_point, FailurePoint::Acquisition);
            let mut limits = ResourceLimits::default();
            if acquisition_denied {
                limits.max_dependency_acquisition_work = 0;
            }
            with_store_body(limits.clone(), ImportOperationControl::default(), |store| {
                let baseline = store.account.snapshot().live_retained_bytes();
                {
                    let io = baozi_io::MemoryAssetIo::new();
                    let dependency_path = AssetPath::new("dependency.bin").unwrap();
                    let payload = b"complete dependency snapshot";
                    if available {
                        io.try_insert(dependency_path, Arc::<[u8]>::from(payload.as_slice()))
                            .unwrap();
                    }
                    let authority = io.authority_domain();
                    let source = AssetPath::new("primary.bin").unwrap();
                    let issuer = DependencyOriginIssuer::new();
                    let diagnostics = RefCell::new(DiagnosticSink::new(16, 4096, 512, 8192));
                    let graph = RefCell::new(ResourceGraph::new(limits.clone()));
                    store
                        .owner
                        .with_root_execution(|host| {
                            graph
                                .borrow_mut()
                                .bind_primary(host, &authority, &source, b"primary")
                                .unwrap();
                            Ok::<_, ImportStoreHostError>(())
                        })
                        .unwrap();
                    let optional_evidence = RefCell::new(AccountedVec::new());
                    let loss_evidence = RefCell::new(AccountedVec::new());
                    let source_scope = RefCell::new(None);
                    let format_failure_storage = RefCell::new(FailureEvidenceStorage::default());
                    let terminal = RefCell::new(None);
                    let loss_policy = if allow_optional {
                        LossPolicy::preserve_or_fail()
                            .allow(ImportLossKind::OptionalDependencyUnavailable)
                    } else {
                        LossPolicy::preserve_or_fail()
                    };
                    let mut context = ParseContext::new(ParseContextParts {
                        entry: entry.clone(),
                        primary_io: &io,
                        primary_authority: &authority,
                        primary_source: &source,
                        primary_bytes: b"primary",
                        resolver: ResolverBinding::registered(
                            baozi_io::__semantic_identity::same_authority_resolver_v1(),
                        ),
                        dependency_origin_issuer: &issuer,
                        conformance_policy: ConformancePolicy::Strict,
                        loss_policy,
                        limits: &limits,
                        store_owner: &mut store.owner,
                        diagnostics: &diagnostics,
                        resource_graph: &graph,
                        optional_dependency_evidence: &optional_evidence,
                        loss_evidence: &loss_evidence,
                        source_scope_record: &source_scope,
                        format_failure_storage: &format_failure_storage,
                        terminal: &terminal,
                    });
                    let _ = context.with_execution(|scope| -> ParseResult<()> {
                        let origin = scope.services()?.root_origin();
                        let role = DependencyRoleId::new(match requirement {
                            DependencyRequirement::Required => "required",
                            DependencyRequirement::Optional => "optional",
                        })
                        .unwrap();
                        assert_eq!(
                            scope.frozen_dependency_requirement(&role).unwrap(),
                            requirement
                        );
                        scope.charge_logical_reference().unwrap_or_else(|_| {
                            panic!("logical reference must fit before the injected limit")
                        });
                        let source_node = graph.borrow().source_for_primary().unwrap();
                        let pending = graph
                            .borrow_mut()
                            .begin_relation(
                                &mut scope.store,
                                source_node,
                                &role,
                                requirement,
                                "dependency.bin",
                            )
                            .unwrap();
                        let (pending, normalized, authorized) = scope
                            .authorize_dependency(
                                pending,
                                &io,
                                &authority,
                                &source,
                                &role,
                                "dependency.bin",
                            )
                            .unwrap_or_else(|_| panic!("dependency authorization must succeed"));
                        let (pending, normalized, authorized) = scope
                            .prepare_authorized_target(pending, normalized, authorized)
                            .unwrap_or_else(|_| panic!("dependency target planning must succeed"));
                        graph
                            .borrow_mut()
                            .reserve_acquisition_capacity(&mut scope.store)
                            .unwrap();
                        let (failure, dimension, class, value, limit) =
                            if matches!(failure_point, FailurePoint::SnapshotSharing) {
                                let (io, identity, stable_identity_schema, registered_resolver) =
                                    authorized.into_parts();
                                let identity_storage = scope
                                    .reserve_dependency_storage_cause(
                                        identity.path().as_str().len() as u64,
                                    )
                                    .unwrap();
                                assert!(
                                    graph
                                        .borrow_mut()
                                        .begin_acquisition(
                                            &mut scope.store,
                                            identity.try_clone().unwrap(),
                                            identity_storage
                                        )
                                        .unwrap()
                                );
                                let bytes = scope.open_and_read_dependency(io, &identity).unwrap();
                                assert_eq!(&*bytes, payload.as_slice());
                                let scratch = scope
                                    .store
                                    .dimension_state(ImportDimension::ScratchBytes)
                                    .unwrap();
                                scope
                                    .store
                                    .tighten_dimension_policy(
                                        ImportDimension::ScratchBytes,
                                        DimensionPolicy::reservable_bytes(
                                            u64::MAX,
                                            u64::MAX,
                                            0,
                                            scratch.live_bytes(),
                                        ),
                                    )
                                    .unwrap();
                                let graph_work = scope
                                    .store
                                    .dimension_state(ImportDimension::ResourceGraphWork)
                                    .unwrap();
                                scope
                                    .store
                                    .tighten_dimension_policy(
                                        ImportDimension::ResourceGraphWork,
                                        DimensionPolicy::counter(
                                            u64::MAX,
                                            graph_work.consumed(),
                                            0,
                                        ),
                                    )
                                    .unwrap();
                                let cause = match scope.share_dependency_snapshot(bytes) {
                                    Err(cause) => cause,
                                    Ok(_) => {
                                        panic!(
                                            "snapshot sharing must exceed the live scratch limit"
                                        )
                                    }
                                };
                                let failure = scope.finish_dependency_resolution_failure(
                                    pending,
                                    identity,
                                    DependencyFailureObservationContext {
                                        stable_identity_schema,
                                        registered_resolver,
                                        io,
                                    },
                                    normalized,
                                    cause,
                                );
                                (
                                    failure,
                                    ImportDimension::ScratchBytes,
                                    CounterClass::Live,
                                    scratch.live_bytes()
                                        + shared_dependency_snapshot_allocation_bytes().unwrap()
                                            as u64,
                                    scratch.live_bytes(),
                                )
                            } else {
                                let before = scope
                                    .store
                                    .dimension_state(ImportDimension::ResourceGraphWork)
                                    .unwrap();
                                // Get and mark_resolving examine one slot each; the completion probe must fail.
                                let limit = before.consumed() + 2;
                                scope
                                    .store
                                    .tighten_dimension_policy(
                                        ImportDimension::ResourceGraphWork,
                                        DimensionPolicy::counter(u64::MAX, limit, 0),
                                    )
                                    .unwrap();
                                let failure = match scope
                                    .acquire_authorized(pending, normalized, authorized)
                                {
                                    Err(failure) => failure,
                                    Ok(_) => panic!(
                                        "cache completion must reject its first unadmitted probe"
                                    ),
                                };
                                if acquisition_denied {
                                    (
                                        failure,
                                        ImportDimension::DependencyAcquisitionWork,
                                        CounterClass::Consumed,
                                        1,
                                        0,
                                    )
                                } else {
                                    (
                                        failure,
                                        ImportDimension::ResourceGraphWork,
                                        CounterClass::Consumed,
                                        limit + 1,
                                        limit,
                                    )
                                }
                            };
                        let is_original_cause = |cause: &ScopeAbortCause| {
                            matches!(
                                cause,
                                ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                                    class: actual_class,
                                    dimension: actual_dimension,
                                    value: actual_value,
                                    limit: actual_limit,
                                }) if *actual_dimension == dimension && *actual_class == class
                                    && *actual_value == value && *actual_limit == limit
                            )
                        };
                        assert!(is_original_cause(
                            scope
                                .dependency_failure
                                .borrow()
                                .as_ref()
                                .unwrap()
                                .cause(&graph.borrow())
                                .unwrap()
                        ));
                        let opened = scope
                            .store
                            .dimension_state(ImportDimension::AssetOpen)
                            .unwrap();
                        assert_eq!(opened.attempted(), 1);
                        assert_eq!(opened.consumed(), u64::from(available));
                        assert_eq!(
                            scope
                                .store
                                .dimension_state(ImportDimension::DependencyBytes)
                                .unwrap()
                                .consumed(),
                            if available && !acquisition_denied {
                                payload.len() as u64
                            } else {
                                0
                            }
                        );
                        assert_eq!(
                            scope
                                .store
                                .dimension_state(ImportDimension::DependencyAcquisitionWork)
                                .unwrap()
                                .consumed(),
                            u64::from(available && !acquisition_denied)
                        );
                        match requirement {
                            DependencyRequirement::Required => {
                                let _ = scope.abort_required_dependency_failure(
                                    &origin,
                                    &role,
                                    "dependency.bin",
                                    failure,
                                );
                            }
                            DependencyRequirement::Optional => {
                                assert!(scope.resolve_optional_failure(&role, failure).is_err());
                            }
                        }
                        assert!(is_original_cause(terminal.borrow().as_ref().unwrap()));
                        assert_eq!(
                            scope
                                .store
                                .dimension_state(ImportDimension::CanonicalOutputBytes)
                                .unwrap()
                                .live_bytes(),
                            0,
                        );
                        Ok(())
                    });
                    let _abort = context.take_terminal_abort().unwrap();
                    assert!(optional_evidence.borrow().is_empty());
                    assert!(loss_evidence.borrow().is_empty());
                    drop(context);
                    assert!(store.owner.completed_scene_boundary().unwrap().is_none());
                    assert!(!store.account.snapshot().poisoned());
                }
                assert_eq!(store.account.snapshot().live_retained_bytes(), baseline);
            });
        }
    }

    #[test]
    fn opened_asset_snapshot_is_unique_metered_vec_storage() {
        let limits = ResourceLimits::default();
        with_store_root_body(limits.clone(), ImportOperationControl::default(), |host| {
            let baseline = current_live_bytes(host);
            let payload = b"metered snapshot".to_vec();
            let opened = OpenedAsset::new(
                AuthorityDomain::new(),
                AssetPath::new("scene.bin").unwrap(),
                Cursor::new(payload.clone()),
                Some(AssetLength::Exact(payload.len() as u64)),
            );

            let snapshot = read_opened_asset(
                host,
                OpenedAssetOwner::new(opened),
                ImportLedgerPhase::PrimarySnapshot,
                ImportProgressPhase::AcquirePrimary,
                ImportDimension::PrimaryBytes,
                limits.max_primary_asset_bytes,
                limits.max_total_asset_bytes,
            )
            .unwrap();

            assert_eq!(&*snapshot, payload.as_slice());
            assert!(snapshot.capacity_bytes() >= payload.len() as u64);
            assert_eq!(
                current_live_bytes(host),
                baseline + snapshot.capacity_bytes()
            );

            drop(snapshot);
            assert_eq!(current_live_bytes(host), baseline);
        });
    }

    #[test]
    fn serial_asset_reads_reuse_aggregate_credit_across_resource_classes() {
        for length in [None, Some(AssetLength::Hint(1024))] {
            let limits = ResourceLimits {
                max_total_asset_bytes: 2,
                ..ResourceLimits::default()
            };
            with_store_root_body(limits.clone(), ImportOperationControl::default(), |host| {
                let baseline = current_live_bytes(host);
                for (dimension, ledger_phase, progress_phase) in [
                    (
                        ImportDimension::PrimaryBytes,
                        ImportLedgerPhase::PrimarySnapshot,
                        ImportProgressPhase::AcquirePrimary,
                    ),
                    (
                        ImportDimension::DependencyBytes,
                        ImportLedgerPhase::DependencyAcquisition,
                        ImportProgressPhase::AcquireDependency,
                    ),
                ] {
                    let opened = OpenedAsset::new(
                        AuthorityDomain::new(),
                        AssetPath::new("one-byte.bin").unwrap(),
                        Cursor::new(b"a"),
                        length,
                    );
                    let work_before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                    let snapshot = read_opened_asset(
                        host,
                        OpenedAssetOwner::new(opened),
                        ledger_phase,
                        progress_phase,
                        dimension,
                        4,
                        2,
                    )
                    .expect("unspent total-byte credit must carry into the next resource");
                    assert_eq!(&*snapshot, b"a");
                    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                    assert_eq!(work.attempted() - work_before.attempted(), 2);
                    assert_eq!(work.consumed() - work_before.consumed(), 2);
                    assert_eq!(
                        current_live_bytes(host),
                        baseline + snapshot.capacity_bytes()
                    );
                    drop(snapshot);
                    assert_eq!(current_live_bytes(host), baseline);
                }
                let total = host
                    .dimension_state(ImportDimension::TotalAssetBytes)
                    .unwrap();
                assert_eq!(total.attempted(), 2);
                assert_eq!(total.consumed(), 2);
                let primary = host.dimension_state(ImportDimension::PrimaryBytes).unwrap();
                assert_eq!(primary.attempted(), 2);
                assert_eq!(primary.consumed(), 1);
                let dependency = host
                    .dimension_state(ImportDimension::DependencyBytes)
                    .unwrap();
                assert_eq!(dependency.attempted(), 1);
                assert_eq!(dependency.consumed(), 1);
                assert_eq!(current_live_bytes(host), baseline);
            });
        }
    }

    #[test]
    fn empty_opened_assets_validate_eof_at_zero_byte_limits() {
        for length in [
            None,
            Some(AssetLength::Hint(1024)),
            Some(AssetLength::Exact(0)),
        ] {
            let limits = ResourceLimits {
                max_primary_asset_bytes: 0,
                max_total_asset_bytes: 0,
                ..ResourceLimits::default()
            };
            with_store_root_body(limits.clone(), ImportOperationControl::default(), |host| {
                let opened = OpenedAsset::new(
                    AuthorityDomain::new(),
                    AssetPath::new("empty.bin").unwrap(),
                    Cursor::new(b""),
                    length,
                )
                .with_backend_hard_limit(0);

                let work_before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                let snapshot = read_opened_asset(
                    host,
                    OpenedAssetOwner::new(opened),
                    ImportLedgerPhase::PrimarySnapshot,
                    ImportProgressPhase::AcquirePrimary,
                    ImportDimension::PrimaryBytes,
                    0,
                    0,
                )
                .expect("a nonempty EOF probe must accept empty input at zero credit");

                assert!(snapshot.is_empty());
                assert_eq!(snapshot.capacity_bytes(), 0);
                for dimension in [
                    ImportDimension::PrimaryBytes,
                    ImportDimension::TotalAssetBytes,
                ] {
                    let state = host.dimension_state(dimension).unwrap();
                    assert_eq!(state.attempted(), 0);
                    assert_eq!(state.consumed(), 0);
                    assert_eq!(state.live_bytes(), 0);
                }
                let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                assert_eq!(work.attempted() - work_before.attempted(), 1);
                assert_eq!(work.consumed() - work_before.consumed(), 1);
            });
        }
    }

    #[test]
    fn opened_assets_preserve_exact_length_truncation_and_overrun_evidence() {
        for payload in [b"abc".as_slice(), b"abcde".as_slice()] {
            let limits = ResourceLimits::default();
            with_store_root_body(limits.clone(), ImportOperationControl::default(), |host| {
                let baseline = current_live_bytes(host);
                let opened = OpenedAsset::new(
                    AuthorityDomain::new(),
                    AssetPath::new("exact.bin").unwrap(),
                    Cursor::new(payload.to_vec()),
                    Some(AssetLength::Exact(4)),
                );

                let work_before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                let cause = read_opened_asset(
                    host,
                    OpenedAssetOwner::new(opened),
                    ImportLedgerPhase::PrimarySnapshot,
                    ImportProgressPhase::AcquirePrimary,
                    ImportDimension::PrimaryBytes,
                    8,
                    8,
                )
                .expect_err("declared truncation or overrun must remain a backend contract error");

                let ScopeAbortCause::AssetRead(error) = cause else {
                    panic!("expected an exact-length contract error")
                };
                assert!(
                    matches!(error.kind(),
                        AssetReadErrorKind::ContractViolation(baozi_io::AssetReadContractViolation::ExactLengthTruncated { expected: 4, observed: 3 })
                            if payload.len() == 3
                    ) || matches!(error.kind(),
                        AssetReadErrorKind::ContractViolation(baozi_io::AssetReadContractViolation::ExactLengthExceeded { expected: 4, observed: 5 })
                            if payload.len() == 5
                    )
                );
                for dimension in [
                    ImportDimension::PrimaryBytes,
                    ImportDimension::TotalAssetBytes,
                ] {
                    let state = host.dimension_state(dimension).unwrap();
                    assert_eq!(state.attempted(), if payload.len() == 3 { 4 } else { 5 });
                    assert_eq!(state.consumed(), payload.len() as u64);
                }
                let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                assert_eq!(work.attempted() - work_before.attempted(), 2);
                assert_eq!(work.consumed() - work_before.consumed(), 2);
                assert_eq!(current_live_bytes(host), baseline);
            });
        }
    }

    struct InterruptedAssetReader {
        calls: Arc<AtomicU64>,
        panics: bool,
    }

    impl baozi_io::AssetReader for InterruptedAssetReader {
        fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, baozi_io::AssetReadError> {
            let calls = self.calls.fetch_add(1, Ordering::AcqRel);
            if calls == 0 {
                buffer[0] = b'a';
                return Ok(1);
            }
            assert!(!self.panics, "injected read panic");
            Err(baozi_io::AssetReadError::new(
                AssetReadErrorKind::InvalidData,
            ))
        }
    }

    #[test]
    fn interrupted_asset_reads_preserve_only_completed_byte_and_call_prefixes() {
        for panics in [false, true] {
            with_store_root_body(
                ResourceLimits::default(),
                ImportOperationControl::default(),
                |host| {
                    let baseline = current_live_bytes(host);
                    let calls = Arc::new(AtomicU64::new(0));
                    let opened = OpenedAsset::new(
                        AuthorityDomain::new(),
                        AssetPath::new("interrupted.bin").unwrap(),
                        InterruptedAssetReader {
                            calls: Arc::clone(&calls),
                            panics,
                        },
                        None,
                    );

                    let work_before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                    let cause = read_opened_asset(
                        host,
                        OpenedAssetOwner::new(opened),
                        ImportLedgerPhase::PrimarySnapshot,
                        ImportProgressPhase::AcquirePrimary,
                        ImportDimension::PrimaryBytes,
                        4,
                        4,
                    )
                    .expect_err("the failing read must preserve the prior completed prefix");

                    assert!(
                        matches!(cause, ScopeAbortCause::AssetRead(_)) && !panics
                            || matches!(
                                cause,
                                ScopeAbortCause::TrustedExtension {
                                    boundary: TrustedExtensionBoundary::AssetIo,
                                    code: "asset-read-panicked"
                                }
                            ) && panics
                    );
                    assert_eq!(calls.load(Ordering::Acquire), 2);
                    let primary = host.dimension_state(ImportDimension::PrimaryBytes).unwrap();
                    assert_eq!(primary.attempted(), 4);
                    assert_eq!(primary.consumed(), 1);
                    let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                    assert_eq!(work.attempted() - work_before.attempted(), 2);
                    assert_eq!(
                        work.consumed() - work_before.consumed(),
                        if panics { 1 } else { 2 }
                    );
                    assert_eq!(current_live_bytes(host), baseline);
                },
            );
        }
    }

    #[test]
    fn asset_reads_poll_cancellation_and_admit_work_before_the_next_backend_call() {
        for cancel in [false, true] {
            let limits = ResourceLimits::default();
            let control = if cancel {
                ImportOperationControl::default().with_progress_observer(
                    |event: baozi_operation::ProgressEvent<
                        ImportProgressPhase,
                        ImportProgressUnit,
                    >| {
                        if event.phase() == ImportProgressPhase::AcquirePrimary {
                            baozi_operation::ProgressDecision::Cancel
                        } else {
                            baozi_operation::ProgressDecision::Continue
                        }
                    },
                )
            } else {
                ImportOperationControl::default()
            };
            with_store_root_body(limits.clone(), control, |host| {
                let baseline = current_live_bytes(host);
                let calls = Arc::new(AtomicU64::new(0));
                let opened = OpenedAsset::new(
                    AuthorityDomain::new(),
                    AssetPath::new("controlled.bin").unwrap(),
                    InterruptedAssetReader {
                        calls: Arc::clone(&calls),
                        panics: true,
                    },
                    None,
                );

                let work_before = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                let work_limit = work_before.attempted() + 1;
                host.tighten_dimension_policy(
                    ImportDimension::WorkUnits,
                    DimensionPolicy::counter(work_limit, work_before.consumed() + 1, 0),
                )
                .expect("one backend read must fit after the startup work");
                let cause = read_opened_asset(
                    host,
                    OpenedAssetOwner::new(opened),
                    ImportLedgerPhase::PrimarySnapshot,
                    ImportProgressPhase::AcquirePrimary,
                    ImportDimension::PrimaryBytes,
                    4,
                    4,
                )
                .expect_err("control or work admission must prevent a second backend call");

                assert!(
                    matches!(cause, ScopeAbortCause::Control(_)) && cancel
                        || matches!(
                            cause,
                            ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                                dimension: ImportDimension::WorkUnits,
                                limit,
                                ..
                            }) if limit == work_limit
                        ) && !cancel
                );
                assert_eq!(calls.load(Ordering::Acquire), 1);
                let primary = host.dimension_state(ImportDimension::PrimaryBytes).unwrap();
                assert_eq!(primary.attempted(), 4);
                assert_eq!(primary.consumed(), 1);
                let work = host.dimension_state(ImportDimension::WorkUnits).unwrap();
                assert_eq!(
                    work.attempted() - work_before.attempted(),
                    if cancel { 1 } else { 2 }
                );
                assert_eq!(work.consumed() - work_before.consumed(), 1);
                assert_eq!(current_live_bytes(host), baseline);
            });
        }
    }

    struct OwnedIterationDropProbe<'a> {
        id: u8,
        storage:
            baozi_operation::__retained_store_integration::authority::StoreStorageObservation<'a>,
        observations: &'a RefCell<Vec<(u8, bool)>>,
        expected_live_bytes: u64,
    }

    impl Drop for OwnedIterationDropProbe<'_> {
        fn drop(&mut self) {
            self.observations.borrow_mut().push((
                self.id,
                self.storage.live_bytes() == self.expected_live_bytes,
            ));
        }
    }

    fn assert_owned_iteration_drop_observations(
        observations: &RefCell<Vec<(u8, bool)>>,
        storage: &baozi_operation::__retained_store_integration::authority::StoreStorageObservation<
            '_,
        >,
        baseline: u64,
    ) {
        let mut observations = observations.borrow().clone();
        observations.sort_unstable_by_key(|(id, _)| *id);
        assert_eq!(observations, vec![(0, true), (1, true), (2, true)]);
        assert_eq!(storage.live_bytes(), baseline);
    }

    #[test]
    fn owned_iteration_error_drops_current_and_remaining_elements_before_capacity() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let observations = RefCell::new(Vec::new());
                    context
                        .with_execution(|exec| -> ParseResult<_> {
                            let baseline = exec
                                .store
                                .dimension_state(ImportDimension::CanonicalOutputBytes)
                                .unwrap()
                                .live_bytes();
                            let mut values = exec.services()?.metered_vec(
                                ImportStorageClass::SceneConstruction,
                                "owned-iteration-error",
                            );
                            values.try_reserve(&mut exec.services()?, 3)?;
                            let storage = testing::storage_observation(
                                exec,
                                ImportDimension::CanonicalOutputBytes,
                            );
                            let expected_live_bytes =
                                baseline + values.capacity.as_ref().unwrap().bytes();
                            for id in 0..3 {
                                values.try_push(
                                    &mut exec.services()?,
                                    OwnedIterationDropProbe {
                                        id,
                                        storage,
                                        observations: &observations,
                                        expected_live_bytes,
                                    },
                                )?;
                            }
                            let result = values.try_for_each_owned(|probe| {
                                assert_eq!(probe.id, 0);
                                Err::<(), _>("injected visitor failure")
                            });
                            assert_eq!(result, Err("injected visitor failure"));
                            assert_owned_iteration_drop_observations(
                                &observations,
                                &storage,
                                baseline,
                            );
                            Ok(())
                        })
                        .unwrap();
                });
            },
        );
    }

    #[test]
    fn owned_iteration_panic_drops_current_and_remaining_elements_before_capacity() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let observations = RefCell::new(Vec::new());
                    context
                        .with_execution(|exec| -> ParseResult<_> {
                            let baseline = exec
                                .store
                                .dimension_state(ImportDimension::CanonicalOutputBytes)
                                .unwrap()
                                .live_bytes();
                            let mut values = exec.services()?.metered_vec(
                                ImportStorageClass::SceneConstruction,
                                "owned-iteration-panic",
                            );
                            values.try_reserve(&mut exec.services()?, 3)?;
                            let storage = testing::storage_observation(
                                exec,
                                ImportDimension::CanonicalOutputBytes,
                            );
                            let expected_live_bytes =
                                baseline + values.capacity.as_ref().unwrap().bytes();
                            for id in 0..3 {
                                values.try_push(
                                    &mut exec.services()?,
                                    OwnedIterationDropProbe {
                                        id,
                                        storage,
                                        observations: &observations,
                                        expected_live_bytes,
                                    },
                                )?;
                            }
                            let result = catch_unwind(AssertUnwindSafe(|| {
                                let _ = values.try_for_each_owned(|probe| -> Result<(), ()> {
                                    assert_eq!(probe.id, 0);
                                    panic!("injected owned-iteration visitor panic");
                                });
                            }));
                            assert!(result.is_err());
                            assert_owned_iteration_drop_observations(
                                &observations,
                                &storage,
                                baseline,
                            );
                            Ok(())
                        })
                        .unwrap();
                });
            },
        );
    }

    #[test]
    fn reserved_vector_copy_rejects_an_unbounded_chunk_without_mutation() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let source = vec![0_u8; METERED_COPY_POLL_INTERVAL + 1];
                    let mut values = context
                        .with_execution(|exec| -> ParseResult<_> {
                            let mut values = exec.services()?.metered_vec(
                                ImportStorageClass::FormatNormalization,
                                "reserved-vector-copy-bound",
                            );
                            values.try_reserve(&mut exec.services()?, source.len())?;
                            Ok(values)
                        })
                        .unwrap();
                    assert!(values.try_extend_from_slice_reserved(&source).is_err());
                    assert!(values.is_empty());
                    assert!(matches!(
                        context.take_terminal_abort().unwrap().cause(),
                        ScopeAbortCause::DescriptorContract {
                            code: "metered-vector-copy-chunk-too-large"
                        },
                    ));
                });
            },
        );
    }

    #[test]
    fn reserved_string_copy_rejects_an_unbounded_chunk_without_mutation() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let source = "x".repeat(METERED_COPY_POLL_INTERVAL + 1);
                    let mut value = context
                        .with_execution(|exec| -> ParseResult<_> {
                            let mut value = exec.services()?.metered_string(
                                ImportStorageClass::FormatNormalization,
                                "reserved-string-copy-bound",
                            );
                            value.try_reserve(&mut exec.services()?, source.len())?;
                            Ok(value)
                        })
                        .unwrap();
                    assert!(value.try_push_str_reserved(&source).is_err());
                    assert!(value.is_empty());
                    assert!(matches!(
                        context.take_terminal_abort().unwrap().cause(),
                        ScopeAbortCause::DescriptorContract {
                            code: "metered-string-copy-chunk-too-large"
                        },
                    ));
                });
            },
        );
    }

    #[test]
    fn dedup_partition_defers_duplicate_drops_to_the_caller() {
        struct DedupDropProbe<'a> {
            key: u8,
            drops: &'a Cell<usize>,
        }
        impl Drop for DedupDropProbe<'_> {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let drops = Cell::new(0);
                    let mut values = context
                        .with_execution(|exec| -> ParseResult<_> {
                            let mut values = exec.services()?.metered_vec(
                                ImportStorageClass::FormatNormalization,
                                "dedup-drop-order",
                            );
                            for key in [1, 1, 2, 2] {
                                values.try_push(
                                    &mut exec.services()?,
                                    DedupDropProbe { key, drops: &drops },
                                )?;
                            }
                            Ok(values)
                        })
                        .unwrap();
                    let retained = values
                        .try_partition_dedup_by(|left, right| {
                            Ok::<_, ParseFailure>(left.key == right.key)
                        })
                        .unwrap();
                    assert_eq!(retained, 2);
                    assert_eq!(values.len(), 4);
                    assert_eq!(drops.get(), 0);
                    values.truncate(retained);
                    assert_eq!(drops.get(), 2);
                    drop(values);
                    assert_eq!(drops.get(), 4);
                });
            },
        );
    }
    #[test]
    fn exclusive_storage_crosses_regions_and_legacy_growth_uses_the_same_owner() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let (mut values, mut text) = context
                        .with_parser_execution(|root| -> ParseResult<_> {
                            let group = root.bind_group([
                                ImportWorkClass::Tokens,
                                ImportWorkClass::StringBytes,
                            ])?;
                            root.child(&group, [2, 3], |child| -> ParseResult<_> {
                                let mut values = child.storage()?.metered_vec(
                                    ImportStorageClass::FormatNormalization,
                                    "exclusive-values",
                                );
                                child.push_growing([1, 0], &mut values, 3_u32)?;
                                child.push_growing([1, 0], &mut values, 5_u32)?;
                                let mut text = child.storage()?.metered_string(
                                    ImportStorageClass::FormatNormalization,
                                    "exclusive-text",
                                );
                                child.append_string([0, 3], &mut text, "aé")?;
                                assert_eq!(
                                    child.work_counters(ImportWorkClass::Tokens)?,
                                    (2, 2, 0)
                                );
                                assert_eq!(
                                    child.work_counters(ImportWorkClass::StringBytes)?,
                                    (3, 3, 0)
                                );
                                Ok((values, text))
                            })
                        })
                        .unwrap();
                    context
                        .with_execution(|exec| -> ParseResult<()> {
                            let additional = values.capacity() + 1;
                            values.try_reserve(&mut exec.services()?, additional)?;
                            values.try_extend_from_slice(&mut exec.services()?, &[7, 11, 13])?;
                            text.try_push_str(&mut exec.services()?, "-legacy")?;
                            assert_eq!(values.as_slice(), &[3, 5, 7, 11, 13]);
                            assert_eq!(text.as_str(), "aé-legacy");
                            let tokens =
                                exec.store.dimension_state(ImportDimension::Tokens).unwrap();
                            assert_eq!((tokens.attempted(), tokens.consumed()), (2, 2));
                            Ok(())
                        })
                        .unwrap();
                });
            },
        );
    }

    #[test]
    fn prepared_vector_capacity_preserves_empty_and_nonclone_sources() {
        struct DropElement<'a>(&'a Cell<usize>);
        impl Drop for DropElement<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        with_store_body(ResourceLimits::default(), Default::default(), |store| {
            testing::with_context(&mut store.owner, |context| {
                let drops = Cell::new(0);
                context
                    .with_execution(|exec| -> ParseResult<()> {
                        let observed =
                            testing::storage_observation(exec, ImportDimension::ScratchBytes);
                        let baseline = observed.live_bytes();
                        let mut values = exec.storage()?.metered_vec(
                            ImportStorageClass::FormatNormalization,
                            "prepared-vector",
                        );
                        assert_eq!((values.len(), values.capacity()), (0, 0));
                        assert!(values.capacity.is_none());
                        for populated in [false, true] {
                            if populated {
                                exec.storage()?.reserve_vec(&mut values, 1)?;
                                values.try_push_reserved(DropElement(&drops))?;
                            }
                            let old = (
                                values.as_slice().as_ptr(),
                                values.len(),
                                values.capacity(),
                                values.capacity.as_ref().map(ImportStoreReservation::bytes),
                            );
                            let live = observed.live_bytes();
                            let spare = values.capacity() - values.len();
                            assert!(
                                exec.storage()?
                                    .prepare_vec_replacement(&values, spare)?
                                    .is_none()
                            );
                            assert_eq!(observed.live_bytes(), live);
                            let additional = values.capacity() + 1;
                            let candidate = exec
                                .storage()?
                                .prepare_vec_replacement(&values, additional)?
                                .unwrap();
                            assert!(candidate.is_empty());
                            assert!(candidate.capacity() >= values.len() + additional);
                            assert_eq!(candidate.class, values.class);
                            assert_eq!(candidate.resource, values.resource);
                            assert!(std::ptr::eq(
                                candidate.identity.issuer,
                                values.identity.issuer
                            ));
                            assert!(std::ptr::eq(
                                candidate.identity.terminal,
                                values.identity.terminal
                            ));
                            let bytes = candidate.capacity.as_ref().unwrap().bytes();
                            assert_eq!(
                                bytes,
                                (candidate.capacity() * size_of::<DropElement<'_>>()) as u64
                            );
                            assert_eq!(observed.live_bytes(), live + bytes);
                            assert_eq!(
                                old,
                                (
                                    values.as_slice().as_ptr(),
                                    values.len(),
                                    values.capacity(),
                                    values.capacity.as_ref().map(ImportStoreReservation::bytes),
                                )
                            );
                            drop(candidate);
                            assert_eq!(observed.live_bytes(), live);
                            assert_eq!(drops.get(), 0);
                        }
                        let additional = values.capacity() + 1;
                        let required = values.len() + additional;
                        exec.storage()?.reserve_vec(&mut values, additional)?;
                        assert!(values.capacity() >= required);
                        assert_eq!(values.len(), 1);
                        assert!(std::ptr::eq(values.as_slice()[0].0, &drops));
                        assert_eq!(drops.get(), 0);
                        drop(values);
                        assert_eq!(drops.get(), 1);
                        assert_eq!(observed.live_bytes(), baseline);
                        Ok::<_, ParseFailure>(())
                    })
                    .unwrap();
            });
        });
    }

    #[test]
    fn prepared_string_capacity_preserves_empty_and_populated_sources() {
        with_store_body(ResourceLimits::default(), Default::default(), |store| {
            testing::with_context(&mut store.owner, |context| {
                context
                    .with_execution(|exec| -> ParseResult<()> {
                        let observed =
                            testing::storage_observation(exec, ImportDimension::ScratchBytes);
                        let baseline = observed.live_bytes();
                        let mut text = exec.storage()?.metered_string(
                            ImportStorageClass::FormatNormalization,
                            "prepared-string",
                        );
                        assert_eq!((text.len(), text.capacity()), (0, 0));
                        assert!(text.capacity.is_none());
                        for populated in [false, true] {
                            if populated {
                                text.try_push_str(&mut exec.services()?, "aé")?;
                            }
                            let old = (
                                text.as_str().as_ptr(),
                                text.len(),
                                text.capacity(),
                                text.capacity.as_ref().map(ImportStoreReservation::bytes),
                            );
                            let live = observed.live_bytes();
                            let spare = text.capacity() - text.len();
                            assert!(
                                exec.storage()?
                                    .prepare_string_replacement(&text, spare)?
                                    .is_none()
                            );
                            assert_eq!(observed.live_bytes(), live);
                            let additional = text.capacity() + 1;
                            let candidate = exec
                                .storage()?
                                .prepare_string_replacement(&text, additional)?
                                .unwrap();
                            assert!(candidate.is_empty());
                            assert!(candidate.capacity() >= text.len() + additional);
                            assert_eq!(candidate.class, text.class);
                            assert_eq!(candidate.resource, text.resource);
                            assert!(std::ptr::eq(
                                candidate.identity.issuer,
                                text.identity.issuer
                            ));
                            assert!(std::ptr::eq(
                                candidate.identity.terminal,
                                text.identity.terminal
                            ));
                            let bytes = candidate.capacity.as_ref().unwrap().bytes();
                            assert_eq!(bytes, candidate.capacity() as u64);
                            assert_eq!(observed.live_bytes(), live + bytes);
                            assert_eq!(text.as_str(), if populated { "aé" } else { "" });
                            assert_eq!(
                                old,
                                (
                                    text.as_str().as_ptr(),
                                    text.len(),
                                    text.capacity(),
                                    text.capacity.as_ref().map(ImportStoreReservation::bytes)
                                )
                            );
                            drop(candidate);
                            assert_eq!(observed.live_bytes(), live);
                        }
                        drop(text);
                        assert_eq!(observed.live_bytes(), baseline);
                        Ok::<_, ParseFailure>(())
                    })
                    .unwrap();
            });
        });
    }

    #[test]
    fn prepared_capacity_faults_preserve_old_payload_tokens_and_first_cause() {
        for (string, qualification) in [(false, false), (false, true), (true, true)] {
            with_store_body(ResourceLimits::default(), Default::default(), |store| {
                testing::with_context(&mut store.owner, |context| {
                    let terminal = context.terminal;
                    assert!(
                        context
                            .with_execution(|exec| -> ParseResult<()> {
                                let observed = testing::storage_observation(
                                    exec,
                                    ImportDimension::ScratchBytes,
                                );
                                let baseline = observed.live_bytes();
                                let mut values = exec.storage()?.metered_vec(
                                    ImportStorageClass::FormatNormalization,
                                    "prepared-vector-fault",
                                );
                                values.try_push(&mut exec.services()?, 17_u32)?;
                                let mut text = exec.storage()?.metered_string(
                                    ImportStorageClass::FormatNormalization,
                                    "prepared-string-fault",
                                );
                                text.try_push_str(&mut exec.services()?, "aé")?;
                                let old = (
                                    values.as_slice().as_ptr(),
                                    values.capacity(),
                                    values.capacity.as_ref().unwrap().bytes(),
                                    text.as_str().as_ptr(),
                                    text.capacity(),
                                    text.capacity.as_ref().unwrap().bytes(),
                                );
                                let live = observed.live_bytes();
                                if qualification {
                                    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                                } else {
                                    inject_next_metered_allocation_failure();
                                }
                                let mut storage = exec.storage()?;
                                let result = if string {
                                    storage
                                        .prepare_string_replacement(&text, text.capacity() + 1)
                                        .map(drop)
                                } else {
                                    storage
                                        .prepare_vec_replacement(&values, values.capacity() + 1)
                                        .map(drop)
                                };
                                assert!(result.is_err());
                                assert!(
                                    storage
                                        .prepare_string_replacement(&text, usize::MAX)
                                        .is_err()
                                );
                                assert!(
                                    storage
                                        .prepare_vec_replacement(&values, usize::MAX)
                                        .is_err()
                                );
                                assert_eq!(values.as_slice(), &[17]);
                                assert_eq!(text.as_str(), "aé");
                                assert_eq!(
                                    old,
                                    (
                                        values.as_slice().as_ptr(),
                                        values.capacity(),
                                        values.capacity.as_ref().unwrap().bytes(),
                                        text.as_str().as_ptr(),
                                        text.capacity(),
                                        text.capacity.as_ref().unwrap().bytes()
                                    )
                                );
                                assert_eq!(observed.live_bytes(), live);
                                drop((values, text));
                                assert_eq!(observed.live_bytes(), baseline);
                                let resource = if string {
                                    "prepared-string-fault"
                                } else {
                                    "prepared-vector-fault"
                                };
                                assert!(
                                    matches!(terminal.borrow().as_ref(),
                            Some(ScopeAbortCause::CapacityEnvelope { resource: actual, .. })
                                if qualification && *actual == resource)
                                        || matches!(terminal.borrow().as_ref(),
                                Some(ScopeAbortCause::Allocation { resource: actual })
                                    if !qualification && *actual == resource)
                                );
                                result
                            })
                            .is_err()
                    );
                });
            });
        }
    }

    #[test]
    fn prepared_capacity_requires_exact_old_and_candidate_overlap_budget() {
        for string in [false, true] {
            for shortfall in [0, 1] {
                with_store_body(ResourceLimits::default(), Default::default(), |store| {
                    testing::with_context(&mut store.owner, |context| {
                        let terminal = context.terminal;
                        let result = context.with_execution(|exec| -> ParseResult<()> {
                            let observed = testing::storage_observation(exec, ImportDimension::ScratchBytes);
                            let baseline = observed.live_bytes();
                            let mut values = exec.storage()?.metered_vec::<u32>(
                                ImportStorageClass::FormatNormalization, "prepared-overlap",
                            );
                            let mut text = exec.storage()?.metered_string(
                                ImportStorageClass::FormatNormalization, "prepared-overlap",
                            );
                            if string {
                                text.try_push_str(&mut exec.services()?, "aé")?;
                            } else {
                                values.try_push(&mut exec.services()?, 23)?;
                            }
                            let (len, capacity, old_bytes) = if string {
                                (text.len(), text.capacity(), text.capacity.as_ref().unwrap().bytes())
                            } else {
                                (values.len(), values.capacity(), values.capacity.as_ref().unwrap().bytes())
                            };
                            let additional = capacity + 1;
                            let replacement = if string {
                                metered_replacement_plan_with_element_size(len, capacity, len + additional, 1)
                            } else {
                                metered_vec_replacement_plan::<u32>(len, capacity, len + additional)
                            }.unwrap();
                            let admitted = replacement.envelope_delta_bytes().unwrap();
                            let live = observed.live_bytes();
                            assert_eq!(live, baseline + old_bytes);
                            let limit = live + admitted - shortfall;
                            exec.store.tighten_dimension_policy(
                                ImportDimension::ScratchBytes,
                                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 0, limit),
                            ).unwrap();
                            let candidates = if string {
                                exec.storage()?.prepare_string_replacement(&text, additional)
                                    .map(|candidate| (None, candidate))
                            } else {
                                exec.storage()?.prepare_vec_replacement(&values, additional)
                                    .map(|candidate| (candidate, None))
                            };
                            let result = candidates.map(|(vector, string)| {
                                let token = vector.as_ref().and_then(|value| value.capacity.as_ref())
                                    .or_else(|| string.as_ref().and_then(|value| value.capacity.as_ref()))
                                    .unwrap();
                                assert_eq!(token.bytes(), admitted);
                                assert_eq!(observed.live_bytes(), live + token.bytes());
                                drop((vector, string));
                            });
                            assert_eq!(observed.live_bytes(), live);
                            if shortfall != 0 {
                                assert!(matches!(terminal.borrow().as_ref(),
                                    Some(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                                        class: CounterClass::Live, dimension: ImportDimension::ScratchBytes,
                                        value, limit: actual_limit,
                                    })) if *value == live + admitted && *actual_limit == limit));
                            }
                            drop((values, text));
                            assert_eq!(observed.live_bytes(), baseline);
                            result
                        });
                        assert_eq!(result.is_ok(), shortfall == 0);
                    });
                });
            }
        }
    }

    #[test]
    fn prepared_zst_vector_checks_length_overflow_before_capacity_fit() {
        with_store_body(ResourceLimits::default(), Default::default(), |store| {
            testing::with_context(&mut store.owner, |context| {
                let terminal = context.terminal;
                assert!(
                    context
                        .with_execution(|exec| -> ParseResult<()> {
                            let observed =
                                testing::storage_observation(exec, ImportDimension::ScratchBytes);
                            let baseline = observed.live_bytes();
                            let mut values = exec.storage()?.metered_vec::<()>(
                                ImportStorageClass::FormatNormalization,
                                "prepared-zst",
                            );
                            assert!(
                                exec.storage()?
                                    .prepare_vec_replacement(&values, usize::MAX)?
                                    .is_none()
                            );
                            values.try_push_reserved(())?;
                            let pointer = values.as_slice().as_ptr();
                            assert!(
                                exec.storage()?
                                    .prepare_vec_replacement(&values, usize::MAX - 1)?
                                    .is_none()
                            );
                            let result = exec
                                .storage()?
                                .prepare_vec_replacement(&values, usize::MAX)
                                .map(drop);
                            assert!(result.is_err());
                            assert_eq!(values.as_slice(), &[()]);
                            assert_eq!(values.as_slice().as_ptr(), pointer);
                            assert_eq!(values.capacity(), usize::MAX);
                            assert!(values.capacity.is_none());
                            assert_eq!(observed.live_bytes(), baseline);
                            assert!(matches!(
                                terminal.borrow().as_ref(),
                                Some(ScopeAbortCause::Allocation {
                                    resource: "prepared-zst"
                                })
                            ));
                            result
                        })
                        .is_err()
                );
            });
        });
    }

    #[test]
    fn exclusive_replacement_faults_preserve_old_payload_token_and_verified_prefix() {
        for qualification in [false, true] {
            with_store_body(
                ResourceLimits::default(),
                ImportOperationControl::default(),
                |store| {
                    testing::with_context(&mut store.owner, |context| {
                        let mut values = context
                            .with_execution(|exec| -> ParseResult<_> {
                                let mut values = exec.services()?.metered_vec::<u32>(
                                    ImportStorageClass::FormatNormalization,
                                    "exclusive-replacement",
                                );
                                values.try_reserve(&mut exec.services()?, 4)?;
                                while values.len() < values.capacity() {
                                    values.try_push_reserved(17)?;
                                }
                                Ok(values)
                            })
                            .unwrap();
                        let old = values.as_slice().to_vec();
                        let old_capacity = values.capacity();
                        let old_bytes = values.capacity.as_ref().unwrap().bytes();
                        let terminal = context.terminal;
                        let physical = Cell::new(0_u8);
                        let result = context.with_parser_execution(|root| -> ParseResult<()> {
                            let group = root.bind_group([ImportWorkClass::Tokens])?;
                            root.child(&group, [2], |child| -> ParseResult<()> {
                                child.run([1], || {
                                    physical.set(31);
                                    Ok(())
                                })?;
                                if qualification {
                                    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                                } else {
                                    inject_next_metered_allocation_failure();
                                }
                                let result = child.storage()?.reserve_vec(&mut values, 1);
                                assert!(result.is_err());
                                assert_eq!(
                                    child.work_counters(ImportWorkClass::Tokens)?,
                                    (2, 1, 0)
                                );
                                result
                            })
                        });
                        assert!(result.is_err());
                        assert_eq!(physical.get(), 31);
                        assert_eq!(values.as_slice(), old);
                        assert_eq!(values.capacity(), old_capacity);
                        assert_eq!(values.capacity.as_ref().unwrap().bytes(), old_bytes);
                        if qualification {
                            assert!(matches!(
                                terminal.borrow().as_ref(),
                                Some(ScopeAbortCause::CapacityEnvelope {
                                    resource: "exclusive-replacement",
                                    ..
                                })
                            ));
                        } else {
                            assert!(matches!(
                                terminal.borrow().as_ref(),
                                Some(ScopeAbortCause::Allocation {
                                    resource: "exclusive-replacement"
                                })
                            ));
                        }
                    });
                },
            );
        }
    }

    #[test]
    fn exclusive_replacement_budget_keeps_old_and_new_capacity_live_together() {
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let (mut values, expected_value, limit) = context
                        .with_execution(|exec| -> ParseResult<_> {
                            let mut values = exec.services()?.metered_vec::<u32>(
                                ImportStorageClass::FormatNormalization,
                                "exclusive-overlap",
                            );
                            values.try_reserve(&mut exec.services()?, 4)?;
                            while values.len() < values.capacity() {
                                values.try_push_reserved(23)?;
                            }
                            let old = values.capacity.as_ref().unwrap().bytes();
                            let live = exec
                                .store
                                .dimension_state(ImportDimension::ScratchBytes)
                                .unwrap()
                                .live_bytes();
                            let replacement = metered_vec_replacement_plan::<u32>(
                                values.len(),
                                values.capacity(),
                                values.len() + 1,
                            )
                            .unwrap();
                            let new = replacement.envelope_delta_bytes().unwrap();
                            let limit = live - old + new;
                            exec.store
                                .tighten_dimension_policy(
                                    ImportDimension::ScratchBytes,
                                    DimensionPolicy::reservable_bytes(
                                        ResourceLimits::default().max_live_bytes,
                                        ResourceLimits::default().max_live_bytes,
                                        0,
                                        limit,
                                    ),
                                )
                                .unwrap();
                            Ok((values, live + new, limit))
                        })
                        .unwrap();
                    let old = values.as_slice().to_vec();
                    let old_capacity = values.capacity();
                    let terminal = context.terminal;
                    assert!(
                        context
                            .with_parser_execution(|root| root
                                .storage()?
                                .reserve_vec(&mut values, 1))
                            .is_err()
                    );
                    assert_eq!(values.as_slice(), old);
                    assert_eq!(values.capacity(), old_capacity);
                    assert!(
                        matches!(terminal.borrow().as_ref(), Some(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Live, dimension: ImportDimension::ScratchBytes, value, limit: actual_limit
            })) if *value == expected_value && *actual_limit == limit)
                    );
                });
            },
        );
    }

    #[test]
    fn exclusive_append_cancellation_retains_real_copied_prefix_without_completing_piece() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        struct ArmedCancellation {
            armed: Arc<AtomicBool>,
            polls: Arc<AtomicUsize>,
        }
        impl baozi_operation::CancellationSignal for ArmedCancellation {
            fn is_cancelled(&self) -> bool {
                self.armed.load(Ordering::SeqCst) && self.polls.fetch_add(1, Ordering::SeqCst) >= 2
            }
        }
        let armed = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let control = ImportOperationControl::default().with_cancellation(ArmedCancellation {
            armed: Arc::clone(&armed),
            polls: Arc::clone(&polls),
        });
        with_store_body(ResourceLimits::default(), control, |store| {
            testing::with_context(&mut store.owner, |context| {
                let source = "é".repeat(4097);
                let mut text = context
                    .with_execution(|exec| -> ParseResult<_> {
                        let mut text = exec.services()?.metered_string(
                            ImportStorageClass::FormatNormalization,
                            "exclusive-chunk-copy",
                        );
                        text.try_reserve(&mut exec.services()?, source.len())?;
                        Ok(text)
                    })
                    .unwrap();
                let terminal = context.terminal;
                let result = context.with_parser_execution(|root| -> ParseResult<()> {
                    let group = root.bind_group([ImportWorkClass::StringBytes])?;
                    root.child(&group, [8194], |child| -> ParseResult<()> {
                        armed.store(true, Ordering::SeqCst);
                        let result = child.append_string([8194], &mut text, &source);
                        assert!(result.is_err());
                        assert_eq!(
                            child.work_counters(ImportWorkClass::StringBytes)?,
                            (8194, 0, 0)
                        );
                        result
                    })
                });
                assert!(result.is_err());
                assert_eq!(text.as_str(), &source[..4096]);
                assert_eq!(polls.load(Ordering::SeqCst), 3);
                assert!(matches!(
                    terminal.borrow().as_ref(),
                    Some(ScopeAbortCause::Control(_))
                ));
            });
        });
    }

    #[test]
    fn exclusive_growth_failure_latches_allocation_before_unused_value_drop_panics() {
        struct UnusedValue<'a> {
            terminal: &'a RefCell<Option<ScopeAbortCause>>,
            drops: &'a Cell<usize>,
        }
        impl Drop for UnusedValue<'_> {
            fn drop(&mut self) {
                assert!(matches!(
                    self.terminal.borrow().as_ref(),
                    Some(ScopeAbortCause::Allocation {
                        resource: "exclusive-unused-value"
                    })
                ));
                self.drops.set(self.drops.get() + 1);
                panic!("actual unused growing-push value destructor");
            }
        }
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    let terminal = context.terminal;
                    let drops = Cell::new(0);
                    let next = Cell::new(false);
                    let physical = Cell::new(0_u8);
                    let result = context.with_parser_execution(|root| -> ParseResult<()> {
                        let group = root.bind_group([ImportWorkClass::Tokens])?;
                        root.child(&group, [2], |child| -> ParseResult<()> {
                            child.run([1], || {
                                physical.set(43);
                                Ok(())
                            })?;
                            let mut values = child.storage()?.metered_vec(
                                ImportStorageClass::FormatNormalization,
                                "exclusive-unused-value",
                            );
                            inject_next_metered_allocation_failure();
                            let result = catch_unwind(AssertUnwindSafe(|| {
                                child.push_growing(
                                    [1],
                                    &mut values,
                                    UnusedValue {
                                        terminal,
                                        drops: &drops,
                                    },
                                )
                            }));
                            assert!(result.is_err());
                            assert!(values.is_empty());
                            assert_eq!(values.capacity(), 0);
                            assert!(values.capacity.is_none());
                            assert_eq!(drops.get(), 1);
                            assert_eq!(child.work_counters(ImportWorkClass::Tokens)?, (2, 1, 0));
                            assert!(
                                child
                                    .run([1], || {
                                        next.set(true);
                                        Ok::<_, ParseFailure>(())
                                    })
                                    .is_err()
                            );
                            Err(ParseFailure::scope())
                        })
                    });
                    assert!(result.is_err());
                    assert_eq!(physical.get(), 43);
                    assert!(!next.get());
                    assert_eq!(drops.get(), 1);
                    assert!(matches!(
                        terminal.borrow().as_ref(),
                        Some(ScopeAbortCause::Allocation {
                            resource: "exclusive-unused-value"
                        })
                    ));
                });
            },
        );
    }
    #[test]
    fn exclusive_completion_projects_real_cause_before_returned_value_drop_panics() {
        struct ReturnedValue<'a> {
            terminal: &'a RefCell<Option<ScopeAbortCause>>,
            observed: &'a Cell<bool>,
            drops: &'a Cell<usize>,
        }
        impl Drop for ReturnedValue<'_> {
            fn drop(&mut self) {
                let observed = matches!(
                    self.terminal.borrow().as_ref(),
                    Some(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                        class: CounterClass::Consumed,
                        dimension: ImportDimension::Tokens,
                        value: 2,
                        limit: 1,
                    }))
                );
                self.observed.set(observed);
                assert!(
                    observed,
                    "actual import terminal must precede returned payload disposal"
                );
                self.drops.set(self.drops.get() + 1);
                panic!("actual returned payload destructor after denied completion");
            }
        }
        with_store_body(
            ResourceLimits::default(),
            ImportOperationControl::default(),
            |store| {
                testing::with_context(&mut store.owner, |context| {
                    context
                        .with_execution(|exec| -> ParseResult<()> {
                            exec.store
                                .tighten_dimension_policy(
                                    ImportDimension::Tokens,
                                    DimensionPolicy::counter(8, 1, 0),
                                )
                                .unwrap();
                            Ok(())
                        })
                        .unwrap();
                    let terminal = context.terminal;
                    let observed = Cell::new(false);
                    let drops = Cell::new(0);
                    let physical = Cell::new(0_u8);
                    let next = Cell::new(false);
                    let result = context.with_parser_execution(|root| -> ParseResult<()> {
                        let layout = root.bind_group([ImportWorkClass::Tokens])?;
                        root.child(&layout, [2], |child| -> ParseResult<()> {
                            child.run([1], || {
                                physical.set(1);
                                Ok::<_, ParseFailure>(())
                            })?;
                            let caught = catch_unwind(AssertUnwindSafe(|| {
                                let _returned = child.run([1], || {
                                    physical.set(2);
                                    Ok::<_, ParseFailure>(ReturnedValue {
                                        terminal,
                                        observed: &observed,
                                        drops: &drops,
                                    })
                                })?;
                                next.set(true);
                                Ok::<_, ParseFailure>(())
                            }));
                            let Err(payload) = caught else {
                                panic!("returned payload Drop must panic");
                            };
                            assert_eq!(
                                payload.downcast_ref::<&'static str>().copied(),
                                Some("actual returned payload destructor after denied completion")
                            );
                            assert!(observed.get());
                            assert_eq!(drops.get(), 1);
                            assert_eq!(physical.get(), 2);
                            assert_eq!(child.work_counters(ImportWorkClass::Tokens)?, (2, 1, 0));
                            assert!(
                                child
                                    .run([1], || {
                                        next.set(true);
                                        Ok::<_, ParseFailure>(())
                                    })
                                    .is_err()
                            );
                            Err(ParseFailure::scope())
                        })
                    });
                    assert!(result.is_err());
                    assert!(observed.get());
                    assert_eq!(drops.get(), 1);
                    assert_eq!(physical.get(), 2);
                    assert!(!next.get());
                    assert!(
                        context
                            .with_execution(|_| {
                                next.set(true);
                                Ok::<_, ParseFailure>(())
                            })
                            .is_err()
                    );
                    assert!(!next.get());
                    assert!(matches!(
                        terminal.borrow().as_ref(),
                        Some(ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                            class: CounterClass::Consumed,
                            dimension: ImportDimension::Tokens,
                            value: 2,
                            limit: 1,
                        }))
                    ));
                });
            },
        );
    }

    #[test]
    fn exclusive_old_string_copy_cancel_preserves_original_payload_capacity_and_token() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        struct ArmedCancellation {
            armed: Arc<AtomicBool>,
            polls: Arc<AtomicUsize>,
        }
        impl baozi_operation::CancellationSignal for ArmedCancellation {
            fn is_cancelled(&self) -> bool {
                self.armed.load(Ordering::SeqCst) && self.polls.fetch_add(1, Ordering::SeqCst) >= 3
            }
        }
        let armed = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let control = ImportOperationControl::default().with_cancellation(ArmedCancellation {
            armed: Arc::clone(&armed),
            polls: Arc::clone(&polls),
        });
        with_store_body(ResourceLimits::default(), control, |store| {
            testing::with_context(&mut store.owner, |context| {
                let old = "\u{e9}".repeat(4097);
                let mut text = context
                    .with_execution(|exec| -> ParseResult<_> {
                        let mut text = exec.services()?.metered_string(
                            ImportStorageClass::FormatNormalization,
                            "exclusive-old-copy",
                        );
                        text.try_push_str(&mut exec.services()?, &old)?;
                        Ok(text)
                    })
                    .unwrap();
                assert_eq!(text.len(), 8194);
                let old_pointer = text.as_str().as_ptr();
                let old_capacity = text.capacity();
                let old_token_bytes = text.capacity.as_ref().unwrap().bytes();
                let terminal = context.terminal;
                let result = context.with_parser_execution(|root| -> ParseResult<()> {
                    armed.store(true, Ordering::SeqCst);
                    root.storage()?.reserve_string(&mut text, old_capacity + 1)
                });
                assert!(result.is_err());
                // Initial reserve poll, backing admission, first old-copy chunk, denied next chunk.
                assert_eq!(polls.load(Ordering::SeqCst), 4);
                assert_eq!(text.as_str(), old);
                assert_eq!(text.as_str().as_ptr(), old_pointer);
                assert_eq!(text.capacity(), old_capacity);
                assert_eq!(text.capacity.as_ref().unwrap().bytes(), old_token_bytes);
                assert!(matches!(
                    terminal.borrow().as_ref(),
                    Some(ScopeAbortCause::Control(
                        baozi_operation::ControlCause::Cancelled {
                            origin: baozi_operation::CancellationOrigin::Signal,
                        }
                    ))
                ));
                let next = Cell::new(false);
                assert!(
                    context
                        .with_execution(|_| {
                            next.set(true);
                            Ok::<_, ParseFailure>(())
                        })
                        .is_err()
                );
                assert!(!next.get());
                assert_eq!(polls.load(Ordering::SeqCst), 4);
            });
        });
    }
}
